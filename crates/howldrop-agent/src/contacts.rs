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
//! with the `openssl` command, as contacts.py did. Nothing here logs an
//! identifier or a hash: the hashes give a phone number back by brute force.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

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

/// The contacts, hashed, and the CAs to check senders against, in files
/// for openssl.
pub struct Contacts {
    hashes: HashSet<String>,
    certs: tempfile::TempDir,
}

/// What a verified record says.
#[derive(Debug, PartialEq)]
struct Record {
    account: String,
    hashes: HashSet<String>,
    expired: bool,
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
        let certs = tempfile::tempdir()?;
        std::fs::write(certs.path().join("root.pem"), root)?;
        std::fs::write(certs.path().join("issuers.pem"), issuers.concat())?;
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
        Ok((Contacts { hashes, certs }, notes))
    }

    pub fn is_empty(&self) -> bool {
        self.hashes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.hashes.len()
    }

    fn root(&self) -> PathBuf {
        self.certs.path().join("root.pem")
    }

    /// Whether to take a transfer from this sender. Blocking.
    #[cfg(test)]
    pub fn decide(&self, record: Option<&[u8]>, certificate: Option<&[u8]>) -> Outcome {
        self.explain(record, certificate).0
    }

    /// [`decide`](Self::decide), and what failed, for the log: openssl's
    /// reason, never an identifier. Blocking.
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
    /// it doesn't verify. `-purpose any`: the signer is a record-signing
    /// certificate, which openssl's S/MIME purpose check refuses.
    fn verify_record(&self, record: &[u8]) -> Result<Record, String> {
        let out = self.certs.path().join(format!("record-{}.bin", unique()));
        let verified = openssl(
            &[
                "cms",
                "-verify",
                "-inform",
                "DER",
                "-CAfile",
                path(&self.root())?,
                "-purpose",
                "any",
                "-out",
                path(&out)?,
            ],
            record,
        );
        let payload = std::fs::read(&out);
        let _ = std::fs::remove_file(&out);
        verified?;
        let payload = payload.map_err(|e| e.to_string())?;
        parse_record(&payload, SystemTime::now()).ok_or_else(|| "not a record plist".into())
    }

    /// The Apple ID a client certificate belongs to; why not when it isn't
    /// an Apple-issued one: it must verify to Apple's root. Everyone-mode
    /// senders present a self-signed certificate (CN=airdrop).
    fn certificate_account(&self, der: &[u8]) -> Result<String, String> {
        let leaf = self.certs.path().join(format!("leaf-{}.pem", unique()));
        let account = (|| {
            let leaf_path = path(&leaf)?;
            openssl(&["x509", "-inform", "DER", "-out", leaf_path], der)?;
            let issuers = self.certs.path().join("issuers.pem");
            // One attribute per line: OpenSSL 3.0 prints "CN = value" in
            // the default form, older ones "CN=value".
            let name = |which: &str| {
                openssl(
                    &[
                        "x509",
                        "-in",
                        leaf_path,
                        "-noout",
                        which,
                        "-nameopt",
                        "multiline",
                    ],
                    b"",
                )
            };
            if let Err(why) = openssl(
                &[
                    "verify",
                    "-CAfile",
                    path(&self.root())?,
                    "-untrusted",
                    path(&issuers)?,
                    "-purpose",
                    "any",
                    leaf_path,
                ],
                b"",
            ) {
                // The issuer is one of Apple's CAs, or the sender itself.
                let issuer = name("-issuer").ok();
                let issuer = issuer.as_deref().and_then(common_name).unwrap_or("?");
                let issuer = if issuer.starts_with(CN_PREFIX) || issuer == "airdrop" {
                    "itself"
                } else {
                    issuer
                };
                return Err(format!("{why} (issued by {issuer})"));
            }
            let subject = name("-subject")?;
            common_name(&subject)
                .and_then(|cn| cn.strip_prefix(CN_PREFIX))
                .map(str::to_string)
                .ok_or_else(|| "not an Apple ID certificate".to_string())
        })();
        let _ = std::fs::remove_file(&leaf);
        account
    }
}

fn path(p: &Path) -> Result<&str, String> {
    p.to_str()
        .ok_or_else(|| "a temporary path isn't UTF-8".into())
}

fn unique() -> String {
    let mut bytes = [0u8; 6];
    let _ = getrandom::fill(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// openssl with `args`, `input` on stdin; its stdout when it succeeds,
/// else its reason.
fn openssl(args: &[&str], input: &[u8]) -> Result<String, String> {
    use std::io::Write;
    let mut child = Command::new("openssl")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("openssl: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("openssl: no stdin")?;
    let input = input.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let out = child
        .wait_with_output()
        .map_err(|e| format!("openssl: {e}"))?;
    let _ = writer.join();
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    Err(openssl_reason(&String::from_utf8_lossy(&out.stderr)))
}

/// What openssl's errors say, without the names it prints with them (a
/// certificate's subject is an Apple ID).
fn openssl_reason(stderr: &str) -> String {
    // verify: "error 10 at 1 depth lookup: certificate has expired"
    if let Some(line) = stderr.lines().find(|l| l.contains(" depth lookup: ")) {
        let (head, why) = line.split_once(" lookup: ").unwrap_or((line, ""));
        let depth = head.split(" at ").nth(1).unwrap_or(head);
        return format!("{why} (at {depth})");
    }
    // cms: "...:Verify error: certificate has expired", or the last
    // routine's message.
    if let Some((_, why)) = stderr.rsplit_once("Verify error:") {
        return why.lines().next().unwrap_or("").trim().to_string();
    }
    // "pid:error:code:library routines:function:reason:file:line:"
    let line = stderr
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("failed");
    match line.split(':').collect::<Vec<_>>().as_slice() {
        [_, "error", _, _, _, reason, ..] => reason,
        _ => line,
    }
    .trim()
    .to_string()
}

/// `commonName = value` from `-nameopt multiline`.
fn common_name(subject: &str) -> Option<&str> {
    subject.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "commonName").then(|| value.trim())
    })
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
    Some(Record {
        account,
        hashes,
        expired,
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
    /// for `account`, made with openssl, as Apple's are shaped.
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
            let d = self.dir.path();
            self.leaf("signer", "Test Record Signer", Some("issuer"));
            std::fs::write(d.join("payload.plist"), payload).unwrap();
            run(
                d,
                &[
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
                ],
            );
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
    fn openssls_reasons_come_without_names() {
        let verify = "CN = com.apple.idms.appleid.prd.001234\nerror 10 at 1 depth lookup: certificate has expired\nerror leaf.pem: verification failed\n";
        assert_eq!(
            openssl_reason(verify),
            "certificate has expired (at 1 depth)"
        );
        let cms = "40A7:error:10800065:CMS routines:CMS_signerinfo_verify_cert:certificate verify error:../crypto/cms/cms_smime.c:286:Verify error:unable to get local issuer certificate\n";
        assert_eq!(
            openssl_reason(cms),
            "unable to get local issuer certificate"
        );
        assert_eq!(
            openssl_reason(
                "40A7:error:068000A8:asn1 encoding routines:asn1_check_tlen:wrong tag:../crypto/asn1/tasn_dec.c:1188:\n"
            ),
            "wrong tag"
        );
    }

    #[test]
    fn apples_cas_load() {
        let (contacts, notes) = Contacts::new(&["+15551234567".into()]).unwrap();
        assert_eq!(contacts.len(), 1);
        assert!(notes.is_empty());
        assert!(contacts.certs.path().join("issuers.pem").exists());
    }
}
