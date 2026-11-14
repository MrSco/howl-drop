//! Who a sender is: Contacts Only, and "Everyone, ask me" telling a known
//! contact from anyone else. A port of omdrop-plugin's contacts.py (and of
//! the add-on's senders.py, which turned the `senders` option into its
//! list), with the same three checks:
//!
//! 1. the sender's Apple ID validation record (`SenderRecordData`, a CMS
//!    blob) verifies to Apple's root, and hasn't expired;
//! 2. the TLS client certificate it presented on `/Ask` verifies to Apple's
//!    root too, and its CN, `com.apple.idms.appleid.prd.<altDsID>`, names
//!    the record's account: a record harvested off the air can't be
//!    replayed by someone without that account's key;
//! 3. one of the record's hashed phone numbers and emails is a contact's.
//!
//! The name a sender gives itself is never trusted. Signatures are checked
//! as contacts.py had the `openssl` command check them ([`crate::pki`]).
//! Nothing here logs an identifier or a hash: the hashes give a phone number
//! back by brute force.

use std::collections::HashSet;
use std::io;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use der::{Decode, DecodePem};
use sha2::{Digest, Sha256};
use x509_cert::Certificate;

use crate::pki;

/// Apple's public root and the issuing CAs above the certificates senders
/// present (a sender presents only its own).
const APPLE_ROOT: &str = include_str!("../apple/apple_root_ca.pem");
const APPLE_ISSUERS: [&str; 2] = [
    include_str!("../apple/apple-aai-ca.pem"),
    include_str!("../apple/apple-aai2-ca.pem"),
];
const CN_PREFIX: &str = "com.apple.idms.appleid.prd.";
/// Seconds from 1970 to 2001-01-01, where plist dates count from.
const APPLE_EPOCH: f64 = 978_307_200.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Accept,
    NoRecord,
    BadRecord,
    NoCertificate,
    Unbound,
    Unknown,
}

impl Outcome {
    pub fn why(self) -> &'static str {
        match self {
            Outcome::Accept => "sender is a known contact",
            Outcome::NoRecord => "sender sent no Apple ID validation record",
            Outcome::BadRecord => "the validation record is not signed by Apple, or has expired",
            Outcome::NoCertificate => "sender presented no client certificate",
            Outcome::Unbound => {
                "the validation record does not belong to the presented certificate"
            }
            Outcome::Unknown => "sender is not a contact",
        }
    }
}

/// The contacts, hashed, and the CAs to check senders against.
pub struct Contacts {
    hashes: HashSet<String>,
    root: Certificate,
    issuers: Vec<Certificate>,
}

/// What a verified record says.
#[derive(Debug, PartialEq)]
struct Record {
    account: String,
    hashes: HashSet<String>,
    expired: bool,
    valid_from: Option<SystemTime>,
    valid_until: Option<SystemTime>,
}

pub struct SenderProfile {
    pub prefixes: Vec<[u8; 2]>,
    pub expires: SystemTime,
}

impl Contacts {
    /// From the `senders` entries, against Apple's CAs. Returns the
    /// contacts and what to tell the user about entries that won't match.
    pub fn new(entries: &[String]) -> io::Result<(Contacts, Vec<String>)> {
        Self::with_cas(entries, APPLE_ROOT, &APPLE_ISSUERS)
    }

    fn with_cas(
        entries: &[String],
        root: &str,
        issuers: &[&str],
    ) -> io::Result<(Self, Vec<String>)> {
        let pem = |p: &str| {
            Certificate::from_pem(p.trim().as_bytes())
                .map_err(|e| io::Error::other(format!("a CA certificate: {e}")))
        };
        let root = pem(root)?;
        let issuers = issuers.iter().map(|p| pem(p)).collect::<io::Result<_>>()?;
        let mut hashes = HashSet::new();
        let mut notes = Vec::new();
        for entry in entries {
            match normalize(entry) {
                Ok((value, warning)) => {
                    hashes.insert(hash(&value));
                    notes.extend(warning);
                }
                Err(why) => notes.push(why),
            }
        }
        Ok((
            Contacts {
                hashes,
                root,
                issuers,
            },
            notes,
        ))
    }

    pub fn is_empty(&self) -> bool {
        self.hashes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.hashes.len()
    }

    /// Validate a local sending identity before it is stored or presented.
    /// The record and certificate must be current, Apple-issued and bound
    /// to the same account. Only the short BLE prefixes leave this module.
    pub fn sender_profile(&self, record: &[u8], certificate: &[u8]) -> io::Result<SenderProfile> {
        let now = SystemTime::now();
        let info = self.verify_record(record).map_err(io::Error::other)?;
        let expires = info
            .valid_until
            .ok_or_else(|| io::Error::other("validation record has no valid expiry"))?;
        if expires <= now || info.valid_from.is_none_or(|start| start > now) {
            return Err(io::Error::other(
                "validation record is expired or not yet valid",
            ));
        }
        let account = self
            .certificate_account(certificate)
            .map_err(io::Error::other)?;
        if info.account.is_empty() || account != info.account {
            return Err(io::Error::other(
                "validation record and certificate belong to different accounts",
            ));
        }
        let leaf = Certificate::from_der(certificate).map_err(io::Error::other)?;
        if leaf.tbs_certificate.validity.not_before.to_system_time() > now {
            return Err(io::Error::other("the certificate is not yet valid"));
        }
        let mut hashes: Vec<_> = info.hashes.into_iter().collect();
        hashes.sort();
        let mut prefixes = Vec::new();
        for hash in hashes {
            if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(io::Error::other(
                    "validation record contains an invalid contact hash",
                ));
            }
            let a = u8::from_str_radix(&hash[..2], 16).map_err(io::Error::other)?;
            let b = u8::from_str_radix(&hash[2..4], 16).map_err(io::Error::other)?;
            if !prefixes.contains(&[a, b]) {
                prefixes.push([a, b]);
            }
        }
        if prefixes.is_empty() || prefixes.len() > 128 {
            return Err(io::Error::other(
                "validation record has no usable contact hashes",
            ));
        }
        Ok(SenderProfile {
            prefixes,
            expires: expires.min(leaf.tbs_certificate.validity.not_after.to_system_time()),
        })
    }

    /// Whether to take a transfer from this sender. Blocking.
    #[cfg(test)]
    pub fn decide(&self, record: Option<&[u8]>, certificate: Option<&[u8]>) -> Outcome {
        self.explain(record, certificate).0
    }

    /// [`decide`](Self::decide), and what failed, for the log: the reason,
    /// never an identifier. Blocking.
    pub fn explain(&self, record: Option<&[u8]>, certificate: Option<&[u8]>) -> (Outcome, String) {
        let Some(record) = record.filter(|r| !r.is_empty()) else {
            return (Outcome::NoRecord, String::new());
        };
        let info = match self.verify_record(record) {
            Ok(info) if !info.expired => info,
            Ok(_) => return (Outcome::BadRecord, "the record has expired".into()),
            Err(why) => return (Outcome::BadRecord, format!("record: {why}")),
        };
        let Some(certificate) = certificate.filter(|c| !c.is_empty()) else {
            return (Outcome::NoCertificate, String::new());
        };
        match self.certificate_account(certificate) {
            Err(why) => return (Outcome::Unbound, format!("certificate: {why}")),
            Ok(account) if account != info.account => {
                return (
                    Outcome::Unbound,
                    "the certificate is for another Apple ID".into(),
                );
            }
            Ok(_) => {}
        }
        if info.hashes.is_disjoint(&self.hashes) {
            return (
                Outcome::Unknown,
                format!(
                    "none of its {} verified addresses is in senders",
                    info.hashes.len()
                ),
            );
        }
        (Outcome::Accept, String::new())
    }

    /// Whether this sender may see the receiver at all. Weaker than
    /// [`decide`](Self::decide), and only for discovery: `/Discover` comes
    /// without a certificate, so a replayed record passes. It fails open:
    /// only a sender positively identified as a stranger is hidden from,
    /// since a phone that sends no record would otherwise never see us.
    /// `/Ask` still decides who may send. Blocking.
    pub fn may_discover(&self, record: Option<&[u8]>) -> bool {
        let Some(record) = record.filter(|r| !r.is_empty()) else {
            return true;
        };
        match self.verify_record(record) {
            Ok(info) if !info.expired => !info.hashes.is_disjoint(&self.hashes),
            _ => true,
        }
    }

    /// Apple's signature over the record, and what it says; why not when
    /// it doesn't verify. The signer is a record-signing certificate, of no
    /// particular purpose. No dates: the record's own ValidAsOf and
    /// SuggestValidDuration say whether it's current (parse_record), so an
    /// Apple CA past its date doesn't refuse it, as one did the
    /// certificates.
    fn verify_record(&self, record: &[u8]) -> Result<Record, String> {
        let payload = pki::verify_signed_data(record, &self.root, &self.issuers)?;
        parse_record(&payload, SystemTime::now()).ok_or_else(|| "not a record plist".into())
    }

    /// The Apple ID a client certificate belongs to; why not when it isn't
    /// an Apple-issued one: it must verify to Apple's root. Everyone-mode
    /// senders present a self-signed certificate (CN=airdrop).
    fn certificate_account(&self, der: &[u8]) -> Result<String, String> {
        let leaf = Certificate::from_der(der).map_err(|_| "not a certificate".to_string())?;
        // The chain to Apple's root, its CAs' dates aside: Apple
        // Application Integration CA expired on 21 Oct 2026 with iPhones
        // still presenting current certificates it issued. The sender's own
        // certificate must be current.
        let verified = pki::verify_chain(&leaf, &self.issuers, &self.root).and_then(|()| {
            let expires = leaf.tbs_certificate.validity.not_after.to_system_time();
            if expires < SystemTime::now() {
                Err("the certificate has expired".to_string())
            } else {
                Ok(())
            }
        });
        if let Err(why) = verified {
            // The issuer is one of Apple's CAs, or the sender itself.
            let issuer = pki::common_name(&leaf.tbs_certificate.issuer);
            let issuer = match issuer.as_deref() {
                Some(cn) if cn.starts_with(CN_PREFIX) || cn == "airdrop" => "itself",
                Some(cn) => cn,
                None => "?",
            };
            return Err(format!("{why} (issued by {issuer})"));
        }
        pki::common_name(&leaf.tbs_certificate.subject)
            .and_then(|cn| cn.strip_prefix(CN_PREFIX).map(str::to_string))
            .ok_or_else(|| "not an Apple ID certificate".to_string())
    }
}

/// The record's payload: a plist with `altDsID`, `ValidatedEmailHashes`,
/// `ValidatedPhoneHashes`, `ValidAsOf` and `SuggestValidDuration`.
fn parse_record(payload: &[u8], now: SystemTime) -> Option<Record> {
    let value = plist::Value::from_reader(io::Cursor::new(payload)).ok()?;
    let d = value.as_dictionary()?;
    let account = d
        .get("altDsID")
        .and_then(|v| v.as_string())
        .unwrap_or_default()
        .to_string();
    let mut hashes = HashSet::new();
    for key in ["ValidatedEmailHashes", "ValidatedPhoneHashes"] {
        for h in d.get(key).and_then(|v| v.as_array()).into_iter().flatten() {
            if let Some(s) = h.as_string() {
                hashes.insert(s.to_lowercase());
            }
        }
    }
    let valid_as = d.get("ValidAsOf").and_then(|v| v.as_date());
    let duration = d.get("SuggestValidDuration").and_then(|v| {
        v.as_signed_integer()
            .map(|n| n as f64)
            .or_else(|| v.as_real())
    });
    let expired = match (valid_as, duration) {
        (Some(at), Some(secs)) => {
            let at = SystemTime::from(at)
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(APPLE_EPOCH);
            let now = now
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_secs_f64();
            at + secs < now
        }
        _ => false,
    };
    let valid_from = valid_as.map(SystemTime::from);
    let valid_until = valid_from.and_then(|start| {
        let seconds = duration?;
        if !seconds.is_finite() || seconds <= 0.0 {
            return None;
        }
        start.checked_add(Duration::try_from_secs_f64(seconds).ok()?)
    });
    Some(Record {
        account,
        hashes,
        expired,
        valid_from,
        valid_until,
    })
}

/// What AirDrop hashes: an email lowercased, a phone number as its digits.
/// Ok((value, warning)), or Err(why the entry can't match anyone).
pub fn normalize(entry: &str) -> Result<(String, Option<String>), String> {
    let entry = entry.trim();
    if let Some((local, domain)) = entry.split_once('@') {
        if local.is_empty() || !domain.contains('.') {
            return Err(format!("{entry:?} is not an email address"));
        }
        return Ok((entry.to_lowercase(), None));
    }
    let digits: String = entry.chars().filter(char::is_ascii_digit).collect();
    if digits.len() < 7 {
        return Err(format!(
            "{entry:?} is neither an email address nor a phone number"
        ));
    }
    // Apple hashes the number with its country code: "555 123 4567" never
    // matches a phone that is +1 555 123 4567.
    let warning = (!entry.starts_with('+')).then(|| {
        format!(
            "a phone number without \"+\" ({} digits) is matched exactly as written: include the country code, e.g. +1 555 123 4567",
            digits.len()
        )
    });
    Ok((digits, warning))
}

fn hash(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The `senders` option: entries separated by commas, semicolons or new
/// lines.
pub fn entries(option: &str) -> Vec<String> {
    option
        .split([',', ';', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    use super::*;

    #[test]
    fn entries_normalize_as_airdrop_hashes_them() {
        assert_eq!(
            entries(" +1 (555) 123-4567, Me@Example.com;\nx "),
            ["+1 (555) 123-4567", "Me@Example.com", "x"]
        );
        assert_eq!(
            normalize("+1 (555) 123-4567").unwrap(),
            ("15551234567".into(), None)
        );
        assert_eq!(
            normalize("Me@Example.com").unwrap(),
            ("me@example.com".into(), None)
        );
        assert!(normalize("555 123 4567").unwrap().1.is_some());
        assert!(normalize("x").is_err());
        assert!(normalize("me@localhost").is_err());
        assert_eq!(
            hash("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    /// A CA, an issuer under it, a record signer and a sender certificate
    /// for `account`, made with the openssl command, as Apple's are shaped.
    struct Pki {
        dir: tempfile::TempDir,
    }

    fn run(dir: &Path, args: &[&str]) {
        let ok = Command::new("openssl")
            .args(args)
            .current_dir(dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success();
        assert!(ok, "openssl {args:?}");
    }

    impl Pki {
        fn new() -> Pki {
            let dir = tempfile::tempdir().unwrap();
            let d = dir.path();
            std::fs::write(
                d.join("ca.ext"),
                "basicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\n",
            )
            .unwrap();
            run(
                d,
                &[
                    "req",
                    "-x509",
                    "-newkey",
                    "rsa:2048",
                    "-nodes",
                    "-keyout",
                    "root.key",
                    "-out",
                    "root.pem",
                    "-days",
                    "2",
                    "-subj",
                    "/CN=Test Root CA",
                ],
            );
            run(
                d,
                &[
                    "req",
                    "-newkey",
                    "rsa:2048",
                    "-nodes",
                    "-keyout",
                    "issuer.key",
                    "-out",
                    "issuer.csr",
                    "-subj",
                    "/CN=Test Issuer",
                ],
            );
            run(
                d,
                &[
                    "x509",
                    "-req",
                    "-in",
                    "issuer.csr",
                    "-CA",
                    "root.pem",
                    "-CAkey",
                    "root.key",
                    "-CAcreateserial",
                    "-out",
                    "issuer.pem",
                    "-days",
                    "2",
                    "-extfile",
                    "ca.ext",
                ],
            );
            Pki { dir }
        }

        fn path(&self, name: &str) -> PathBuf {
            self.dir.path().join(name)
        }

        fn read(&self, name: &str) -> String {
            std::fs::read_to_string(self.path(name)).unwrap()
        }

        /// A certificate with this CN, issued by `issuer` (or self-signed).
        fn leaf(&self, name: &str, cn: &str, issuer: Option<&str>) -> Vec<u8> {
            let d = self.dir.path();
            let key = format!("{name}.key");
            let pem = format!("{name}.pem");
            let subj = format!("/CN={cn}");
            match issuer {
                Some(issuer) => {
                    let csr = format!("{name}.csr");
                    run(
                        d,
                        &[
                            "req", "-newkey", "rsa:2048", "-nodes", "-keyout", &key, "-out", &csr,
                            "-subj", &subj,
                        ],
                    );
                    run(
                        d,
                        &[
                            "x509",
                            "-req",
                            "-in",
                            &csr,
                            "-CA",
                            &format!("{issuer}.pem"),
                            "-CAkey",
                            &format!("{issuer}.key"),
                            "-CAcreateserial",
                            "-out",
                            &pem,
                            "-days",
                            "2",
                        ],
                    );
                }
                None => run(
                    d,
                    &[
                        "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", &key, "-out",
                        &pem, "-days", "2", "-subj", &subj,
                    ],
                ),
            }
            let der = format!("{name}.der");
            run(d, &["x509", "-in", &pem, "-outform", "DER", "-out", &der]);
            std::fs::read(self.path(&der)).unwrap()
        }

        /// A validation record: `payload` signed as CMS by a signer under
        /// the issuer, carrying the signer and the issuer.
        fn record(&self, payload: &[u8]) -> Vec<u8> {
            self.record_with(payload, &[])
        }

        /// As [`record`](Self::record), with more of openssl's options.
        fn record_with(&self, payload: &[u8], more: &[&str]) -> Vec<u8> {
            let d = self.dir.path();
            self.leaf("signer", "Test Record Signer", Some("issuer"));
            std::fs::write(d.join("payload.plist"), payload).unwrap();
            let mut args = vec![
                "cms",
                "-sign",
                "-binary",
                "-nodetach",
                "-in",
                "payload.plist",
                "-signer",
                "signer.pem",
                "-inkey",
                "signer.key",
                "-certfile",
                "issuer.pem",
                "-outform",
                "DER",
                "-out",
                "record.der",
            ];
            args.extend_from_slice(more);
            run(d, &args);
            std::fs::read(self.path("record.der")).unwrap()
        }

        fn contacts(&self, entries: &[&str]) -> Contacts {
            let entries: Vec<String> = entries.iter().map(|s| s.to_string()).collect();
            Contacts::with_cas(
                &entries,
                &self.read("root.pem"),
                &[&self.read("issuer.pem")],
            )
            .unwrap()
            .0
        }
    }

    fn payload(account: &str, phones: &[&str], valid_as: SystemTime, secs: i64) -> Vec<u8> {
        let mut d = plist::Dictionary::new();
        d.insert("altDsID".into(), account.into());
        d.insert(
            "ValidatedPhoneHashes".into(),
            plist::Value::Array(
                phones
                    .iter()
                    .map(|p| hash(p).to_uppercase().into())
                    .collect(),
            ),
        );
        d.insert("ValidatedEmailHashes".into(), plist::Value::Array(vec![]));
        d.insert("ValidAsOf".into(), plist::Value::Date(valid_as.into()));
        d.insert("SuggestValidDuration".into(), secs.into());
        let mut out = Vec::new();
        plist::Value::Dictionary(d).to_writer_xml(&mut out).unwrap();
        out
    }

    // spec: ID-1, ID-5, BLE-2
    #[test]
    fn imported_sender_profiles_are_bound_current_and_have_valid_hashes() {
        let pki = Pki::new();
        let verifier = pki.contacts(&[]);
        let cert = pki.leaf(
            "profile",
            &format!("{CN_PREFIX}001234.abcd"),
            Some("issuer"),
        );
        let now = SystemTime::now() - Duration::from_secs(1);
        let record = pki.record(&payload("001234.abcd", &["15551234567"], now, 86400));
        let profile = verifier.sender_profile(&record, &cert).unwrap();
        assert!(
            Contacts::new(&[])
                .unwrap()
                .0
                .sender_profile(&record, &cert)
                .is_err()
        );
        let expected = hash("15551234567");
        assert_eq!(
            profile.prefixes,
            vec![[
                u8::from_str_radix(&expected[..2], 16).unwrap(),
                u8::from_str_radix(&expected[2..4], 16).unwrap()
            ]]
        );
        assert!(profile.expires > SystemTime::now());
        let other = pki.leaf(
            "different",
            &format!("{CN_PREFIX}009999.ffff"),
            Some("issuer"),
        );
        assert!(verifier.sender_profile(&record, &other).is_err());
        let expired = pki.record(&payload(
            "001234.abcd",
            &["15551234567"],
            now - Duration::from_secs(86400),
            60,
        ));
        assert!(verifier.sender_profile(&expired, &cert).is_err());
        let future = pki.record(&payload(
            "001234.abcd",
            &["15551234567"],
            now + Duration::from_secs(3600),
            86400,
        ));
        assert!(verifier.sender_profile(&future, &cert).is_err());
        let mut undated = plist::Value::from_reader(io::Cursor::new(payload(
            "001234.abcd",
            &["15551234567"],
            now,
            86400,
        )))
        .unwrap();
        undated
            .as_dictionary_mut()
            .unwrap()
            .remove("SuggestValidDuration");
        let mut xml = Vec::new();
        undated.to_writer_xml(&mut xml).unwrap();
        assert!(verifier.sender_profile(&pki.record(&xml), &cert).is_err());
        let empty = pki.record(&payload("001234.abcd", &[], now, 86400));
        assert!(verifier.sender_profile(&empty, &cert).is_err());
        let mut unsigned = plist::Value::from_reader(io::Cursor::new(payload(
            "001234.abcd",
            &["15551234567"],
            now,
            86400,
        )))
        .unwrap();
        unsigned.as_dictionary_mut().unwrap().insert(
            "ValidatedPhoneHashes".into(),
            plist::Value::Array(vec!["not-a-hash".into()]),
        );
        let mut xml = Vec::new();
        unsigned.to_writer_xml(&mut xml).unwrap();
        assert!(verifier.sender_profile(&pki.record(&xml), &cert).is_err());
        let mut corrupt = record;
        corrupt[20] ^= 1;
        assert!(verifier.sender_profile(&corrupt, &cert).is_err());
    }

    #[test]
    fn a_contact_with_its_own_certificate_is_accepted() {
        let pki = Pki::new();
        let contacts = pki.contacts(&["+1 555 123 4567"]);
        let record = pki.record(&payload(
            "001234.abcd",
            &["15551234567"],
            SystemTime::now(),
            86400,
        ));
        let cert = pki.leaf("sender", &format!("{CN_PREFIX}001234.abcd"), Some("issuer"));
        assert_eq!(contacts.decide(Some(&record), Some(&cert)), Outcome::Accept);
        assert!(contacts.may_discover(Some(&record)));

        // Someone else's certificate, a self-signed one, or none.
        let other = pki.leaf("other", &format!("{CN_PREFIX}009999.ffff"), Some("issuer"));
        assert_eq!(
            contacts.decide(Some(&record), Some(&other)),
            Outcome::Unbound
        );
        let forged = pki.leaf("forged", &format!("{CN_PREFIX}001234.abcd"), None);
        assert_eq!(
            contacts.decide(Some(&record), Some(&forged)),
            Outcome::Unbound
        );
        assert_eq!(contacts.decide(Some(&record), None), Outcome::NoCertificate);
        assert_eq!(contacts.decide(None, Some(&cert)), Outcome::NoRecord);

        // Not on the list.
        let stranger = pki.contacts(&["+44 20 7946 0000"]);
        assert_eq!(
            stranger.decide(Some(&record), Some(&cert)),
            Outcome::Unknown
        );
        assert!(!stranger.may_discover(Some(&record)));
    }

    #[test]
    fn a_record_must_be_signed_and_current() {
        let pki = Pki::new();
        let contacts = pki.contacts(&["+1 555 123 4567"]);
        let cert = pki.leaf("sender", &format!("{CN_PREFIX}001234.abcd"), Some("issuer"));
        let old = SystemTime::now() - Duration::from_secs(7 * 86400);
        let expired = pki.record(&payload("001234.abcd", &["15551234567"], old, 86400));
        assert_eq!(
            contacts.decide(Some(&expired), Some(&cert)),
            Outcome::BadRecord
        );
        // Not CMS at all; discovery fails open.
        assert_eq!(
            contacts.decide(Some(b"junk"), Some(&cert)),
            Outcome::BadRecord
        );
        assert!(contacts.may_discover(Some(b"junk")));
        assert!(contacts.may_discover(None));
        // Signed by a CA that isn't the trusted root.
        let elsewhere = Pki::new();
        let record = elsewhere.record(&payload(
            "001234.abcd",
            &["15551234567"],
            SystemTime::now(),
            86400,
        ));
        assert_eq!(
            contacts.decide(Some(&record), Some(&cert)),
            Outcome::BadRecord
        );
    }

    #[test]
    fn a_record_in_ber_reads_as_in_der() {
        let pki = Pki::new();
        let contacts = pki.contacts(&["me@example.com"]);
        let mut p = plist::Dictionary::new();
        p.insert("altDsID".into(), "001234.abcd".into());
        p.insert(
            "ValidatedEmailHashes".into(),
            plist::Value::Array(vec![hash("me@example.com").into()]),
        );
        let mut payload = Vec::new();
        plist::Value::Dictionary(p)
            .to_writer_xml(&mut payload)
            .unwrap();
        // Streamed: indefinite lengths, the content in pieces.
        let record = pki.record_with(&payload, &["-stream"]);
        assert_eq!(record[..2], [0x30, 0x80]);
        let cert = pki.leaf("sender", &format!("{CN_PREFIX}001234.abcd"), Some("issuer"));
        assert_eq!(contacts.decide(Some(&record), Some(&cert)), Outcome::Accept);
        // DER stays as it was.
        let der = pki.record(&payload);
        assert_eq!(pki::ber_to_der(&der).unwrap(), der);
        assert_eq!(pki::ber_to_der(&record[..record.len() - 1]), None);
    }

    #[test]
    fn reasons_name_no_one() {
        let pki = Pki::new();
        let contacts = pki.contacts(&["+1 555 123 4567"]);
        let record = pki.record(&payload(
            "001234.abcd",
            &["15551234567"],
            SystemTime::now(),
            86400,
        ));
        let forged = pki.leaf("forged", &format!("{CN_PREFIX}001234.abcd"), None);
        let (outcome, why) = contacts.explain(Some(&record), Some(&forged));
        assert_eq!(outcome, Outcome::Unbound);
        assert_eq!(
            why,
            "certificate: self-signed certificate (issued by itself)"
        );
        // Signed under a root of the same name, but not the trusted one.
        let elsewhere = Pki::new().record(&payload("001234.abcd", &[], SystemTime::now(), 86400));
        let (_, why) = contacts.explain(Some(&elsewhere), Some(&forged));
        assert_eq!(why, "record: certificate signature failure");
        let (_, why) = contacts.explain(Some(b"junk"), Some(&forged));
        assert_eq!(why, "record: not a CMS message (bad encoding)");
    }

    #[test]
    fn apples_cas_load() {
        let (contacts, notes) = Contacts::new(&["+15551234567".into()]).unwrap();
        assert_eq!(contacts.len(), 1);
        assert!(notes.is_empty());
        assert_eq!(contacts.issuers.len(), 2);
        for issuer in &contacts.issuers {
            assert_eq!(
                issuer.tbs_certificate.issuer,
                contacts.root.tbs_certificate.subject
            );
        }
    }
}
