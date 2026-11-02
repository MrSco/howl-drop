//! What [`crate::contacts`] had the `openssl` command do: a certificate's
//! chain to a trusted root, and a CMS SignedData's signature (RFC 5652)
//! with its signer's chain, as `openssl verify -purpose any` and `openssl
//! cms -verify -purpose any` check them. Read with the RustCrypto types,
//! verified with ring. No dates here: the caller decides which matter.
//! Errors read as openssl's did, and name no certificate.

use cms::cert::CertificateChoices;
use cms::content_info::ContentInfo;
use cms::signed_data::{SignedData, SignerIdentifier, SignerInfo};
use der::asn1::ObjectIdentifier;
use der::{Decode, Encode};
use ring::signature::{self, UnparsedPublicKey, VerificationAlgorithm};
use x509_cert::Certificate;
use x509_cert::ext::pkix::{BasicConstraints, SubjectKeyIdentifier};
use x509_cert::name::Name;
use x509_cert::spki::SubjectPublicKeyInfoOwned;

const fn oid(s: &str) -> ObjectIdentifier {
    ObjectIdentifier::new_unwrap(s)
}

const SIGNED_DATA: ObjectIdentifier = oid("1.2.840.113549.1.7.2");
const MESSAGE_DIGEST: ObjectIdentifier = oid("1.2.840.113549.1.9.4");
const COMMON_NAME: ObjectIdentifier = oid("2.5.4.3");
const BASIC_CONSTRAINTS: ObjectIdentifier = oid("2.5.29.19");
const SUBJECT_KEY_ID: ObjectIdentifier = oid("2.5.29.14");

const SHA1: ObjectIdentifier = oid("1.3.14.3.2.26");
const SHA256: ObjectIdentifier = oid("2.16.840.1.101.3.4.2.1");
const SHA384: ObjectIdentifier = oid("2.16.840.1.101.3.4.2.2");
const SHA512: ObjectIdentifier = oid("2.16.840.1.101.3.4.2.3");

const RSA: ObjectIdentifier = oid("1.2.840.113549.1.1.1");
const SHA1_RSA: ObjectIdentifier = oid("1.2.840.113549.1.1.5");
const SHA256_RSA: ObjectIdentifier = oid("1.2.840.113549.1.1.11");
const SHA384_RSA: ObjectIdentifier = oid("1.2.840.113549.1.1.12");
const SHA512_RSA: ObjectIdentifier = oid("1.2.840.113549.1.1.13");
const EC_KEY: ObjectIdentifier = oid("1.2.840.10045.2.1");
const SHA256_ECDSA: ObjectIdentifier = oid("1.2.840.10045.4.3.2");
const SHA384_ECDSA: ObjectIdentifier = oid("1.2.840.10045.4.3.3");
const P256: ObjectIdentifier = oid("1.2.840.10045.3.1.7");
const P384: ObjectIdentifier = oid("1.3.132.0.34");

/// Issuers between a certificate and the root, at most.
const MAX_DEPTH: usize = 6;

#[derive(Clone, Copy)]
enum Hash {
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}

impl Hash {
    fn of_digest(oid: ObjectIdentifier) -> Option<Hash> {
        match oid {
            SHA1 => Some(Hash::Sha1),
            SHA256 => Some(Hash::Sha256),
            SHA384 => Some(Hash::Sha384),
            SHA512 => Some(Hash::Sha512),
            _ => None,
        }
    }

    /// The hash a signature algorithm names; `digest`'s for plain
    /// rsaEncryption, as CMS allows.
    fn of_signature(oid: ObjectIdentifier, digest: Option<ObjectIdentifier>) -> Option<Hash> {
        match oid {
            SHA1_RSA => Some(Hash::Sha1),
            SHA256_RSA | SHA256_ECDSA => Some(Hash::Sha256),
            SHA384_RSA | SHA384_ECDSA => Some(Hash::Sha384),
            SHA512_RSA => Some(Hash::Sha512),
            RSA => digest.and_then(Hash::of_digest),
            _ => None,
        }
    }

    fn digest(self, data: &[u8]) -> Vec<u8> {
        let alg = match self {
            Hash::Sha1 => &ring::digest::SHA1_FOR_LEGACY_USE_ONLY,
            Hash::Sha256 => &ring::digest::SHA256,
            Hash::Sha384 => &ring::digest::SHA384,
            Hash::Sha512 => &ring::digest::SHA512,
        };
        ring::digest::digest(alg, data).as_ref().to_vec()
    }
}

/// ring's algorithm for `hash` with this kind of key.
fn algorithm(
    hash: Hash,
    key: &SubjectPublicKeyInfoOwned,
) -> Option<&'static dyn VerificationAlgorithm> {
    if key.algorithm.oid == RSA {
        return Some(match hash {
            Hash::Sha1 => &signature::RSA_PKCS1_2048_8192_SHA1_FOR_LEGACY_USE_ONLY,
            Hash::Sha256 => &signature::RSA_PKCS1_2048_8192_SHA256,
            Hash::Sha384 => &signature::RSA_PKCS1_2048_8192_SHA384,
            Hash::Sha512 => &signature::RSA_PKCS1_2048_8192_SHA512,
        });
    }
    if key.algorithm.oid != EC_KEY {
        return None;
    }
    let curve = key
        .algorithm
        .parameters
        .as_ref()?
        .decode_as::<ObjectIdentifier>()
        .ok()?;
    match (curve, hash) {
        (P256, Hash::Sha256) => Some(&signature::ECDSA_P256_SHA256_ASN1),
        (P256, Hash::Sha384) => Some(&signature::ECDSA_P256_SHA384_ASN1),
        (P384, Hash::Sha256) => Some(&signature::ECDSA_P384_SHA256_ASN1),
        (P384, Hash::Sha384) => Some(&signature::ECDSA_P384_SHA384_ASN1),
        _ => None,
    }
}

/// `signature` over `message` by `key`, with `hash`.
fn check(
    hash: Hash,
    key: &SubjectPublicKeyInfoOwned,
    message: &[u8],
    signature: &[u8],
) -> Result<(), String> {
    let alg = algorithm(hash, key).ok_or("unsupported signature algorithm")?;
    UnparsedPublicKey::new(alg, key.subject_public_key.raw_bytes())
        .verify(message, signature)
        .map_err(|_| "signature failure".to_string())
}

/// Whether `issuer`'s key signed `cert`.
fn signed_by(cert: &Certificate, issuer: &Certificate) -> bool {
    let Some(hash) = Hash::of_signature(cert.signature_algorithm.oid, None) else {
        return false;
    };
    let Ok(tbs) = cert.tbs_certificate.to_der() else {
        return false;
    };
    check(
        hash,
        &issuer.tbs_certificate.subject_public_key_info,
        &tbs,
        cert.signature.raw_bytes(),
    )
    .is_ok()
}

fn extension(cert: &Certificate, id: ObjectIdentifier) -> Option<&[u8]> {
    cert.tbs_certificate
        .extensions
        .as_ref()?
        .iter()
        .find(|e| e.extn_id == id)
        .map(|e| e.extn_value.as_bytes())
}

fn is_ca(cert: &Certificate) -> bool {
    extension(cert, BASIC_CONSTRAINTS)
        .and_then(|b| BasicConstraints::from_der(b).ok())
        .is_some_and(|b| b.ca)
}

/// That `cert` chains to `root`, through CAs from `pool`.
pub fn verify_chain(
    cert: &Certificate,
    pool: &[Certificate],
    root: &Certificate,
) -> Result<(), String> {
    let root_name = &root.tbs_certificate.subject;
    let mut cert = cert;
    for _ in 0..MAX_DEPTH {
        let issuer_name = &cert.tbs_certificate.issuer;
        if issuer_name == root_name {
            return if signed_by(cert, root) {
                Ok(())
            } else {
                Err("certificate signature failure".into())
            };
        }
        cert = pool
            .iter()
            .find(|c| &c.tbs_certificate.subject == issuer_name && is_ca(c) && signed_by(cert, c))
            .ok_or_else(|| {
                if issuer_name == &cert.tbs_certificate.subject {
                    "self-signed certificate".to_string()
                } else {
                    "unable to get local issuer certificate".to_string()
                }
            })?;
    }
    Err("certificate chain too long".into())
}

/// The content of a CMS SignedData whose signer chains to `root`, through
/// CAs it carries or from `pool`.
pub fn verify_signed_data(
    der: &[u8],
    root: &Certificate,
    pool: &[Certificate],
) -> Result<Vec<u8>, String> {
    let der = ber_to_der(der).ok_or("not a CMS message (bad encoding)")?;
    let info =
        ContentInfo::from_der(&der).map_err(|e| format!("not a CMS message ({})", e.kind()))?;
    if info.content_type != SIGNED_DATA {
        return Err("not signed data".into());
    }
    let signed: SignedData = info
        .content
        .decode_as()
        .map_err(|e| format!("not a CMS message ({})", e.kind()))?;
    let content = signed
        .encap_content_info
        .econtent
        .as_ref()
        .ok_or("no content")?
        .value()
        .to_vec();
    let mut certs: Vec<Certificate> = signed
        .certificates
        .iter()
        .flat_map(|set| set.0.iter())
        .filter_map(|c| match c {
            CertificateChoices::Certificate(c) => Some(c.clone()),
            CertificateChoices::Other(_) => None,
        })
        .collect();
    let signer_info = signed.signer_infos.0.iter().next().ok_or("no signers")?;
    let signer = certs
        .iter()
        .find(|c| is_signer(c, signer_info))
        .ok_or("signer certificate not found")?
        .clone();
    check_signer_info(signer_info, &signer, &content)?;
    certs.extend(pool.iter().cloned());
    verify_chain(&signer, &certs, root)?;
    Ok(content)
}

fn is_signer(cert: &Certificate, info: &SignerInfo) -> bool {
    match &info.sid {
        SignerIdentifier::IssuerAndSerialNumber(id) => {
            cert.tbs_certificate.issuer == id.issuer
                && cert.tbs_certificate.serial_number == id.serial_number
        }
        SignerIdentifier::SubjectKeyIdentifier(id) => extension(cert, SUBJECT_KEY_ID)
            .and_then(|b| SubjectKeyIdentifier::from_der(b).ok())
            .is_some_and(|ski| ski == *id),
    }
}

/// The signer's signature: over its signed attributes, whose message
/// digest must be the content's, or over the content when it has none.
fn check_signer_info(
    info: &SignerInfo,
    signer: &Certificate,
    content: &[u8],
) -> Result<(), String> {
    let digest = Hash::of_digest(info.digest_alg.oid).ok_or("unsupported digest algorithm")?;
    let hash = Hash::of_signature(info.signature_algorithm.oid, Some(info.digest_alg.oid))
        .ok_or("unsupported signature algorithm")?;
    let message = match &info.signed_attrs {
        Some(attrs) => {
            let stated = attrs
                .iter()
                .find(|a| a.oid == MESSAGE_DIGEST)
                .and_then(|a| a.values.iter().next())
                .map(|v| v.value().to_vec())
                .ok_or("no message digest")?;
            if stated != digest.digest(content) {
                return Err("content verify error".into());
            }
            attrs.to_der().map_err(|e| e.to_string())?
        }
        None => content.to_vec(),
    };
    check(
        hash,
        &signer.tbs_certificate.subject_public_key_info,
        &message,
        info.signature.as_bytes(),
    )
}

/// BER as DER, as far as CMS messages need it: indefinite lengths made
/// definite, long lengths shortest, and constructed OCTET STRINGs (streamed
/// content) joined into one. DER comes back as it was, byte for byte, so
/// the certificates inside keep their signatures. None for what isn't one
/// whole element.
pub fn ber_to_der(input: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len());
    let used = element(input, &mut out, 0)?;
    (used == input.len()).then_some(out)
}

/// Nesting deeper than this isn't a CMS message.
const MAX_NESTING: usize = 64;
const OCTET_STRING: u8 = 0x04;
const CONSTRUCTED: u8 = 0x20;

/// The element at the start of `input`, written to `out` as DER; the
/// bytes of `input` it took.
fn element(input: &[u8], out: &mut Vec<u8>, depth: usize) -> Option<usize> {
    if depth > MAX_NESTING {
        return None;
    }
    let first = *input.first()?;
    let mut i = 1;
    if first & 0x1f == 0x1f {
        // A high tag number, base 128.
        while input.get(i)? & 0x80 != 0 {
            i += 1;
        }
        i += 1;
    }
    let identifier = &input[..i];
    let constructed = first & CONSTRUCTED != 0;
    let length = *input.get(i)?;
    i += 1;
    let definite = match length {
        0x80 => None,
        l if l & 0x80 == 0 => Some(usize::from(l)),
        l => {
            let n = usize::from(l & 0x7f);
            if n > 4 {
                return None;
            }
            let mut len = 0usize;
            for _ in 0..n {
                len = len << 8 | usize::from(*input.get(i)?);
                i += 1;
            }
            Some(len)
        }
    };
    let mut content = Vec::new();
    match (constructed, definite) {
        (false, None) => return None,
        (false, Some(len)) => {
            let end = i.checked_add(len)?;
            content.extend_from_slice(input.get(i..end)?);
            i = end;
        }
        (true, Some(len)) => {
            let end = i.checked_add(len)?;
            let body = input.get(i..end)?;
            let mut j = 0;
            while j < body.len() {
                j += element(&body[j..], &mut content, depth + 1)?;
            }
            i = end;
        }
        (true, None) => loop {
            if input.get(i..i + 2)? == [0, 0] {
                i += 2;
                break;
            }
            i += element(&input[i..], &mut content, depth + 1)?;
        },
    }
    if first == OCTET_STRING | CONSTRUCTED {
        // Its pieces, themselves OCTET STRINGs (DER by now), as one.
        let mut joined = Vec::new();
        let mut j = 0;
        while j < content.len() {
            if content[j] != OCTET_STRING {
                return None;
            }
            let (len, header) = der_length(&content[j + 1..])?;
            let start = j + 1 + header;
            joined.extend_from_slice(content.get(start..start + len)?);
            j = start + len;
        }
        write_element(out, &[OCTET_STRING], &joined);
    } else {
        write_element(out, identifier, &content);
    }
    Some(i)
}

/// A DER length at the start of `input`: the length, and its own size.
fn der_length(input: &[u8]) -> Option<(usize, usize)> {
    let first = *input.first()?;
    if first & 0x80 == 0 {
        return Some((usize::from(first), 1));
    }
    let n = usize::from(first & 0x7f);
    let mut len = 0usize;
    for k in 0..n {
        len = len << 8 | usize::from(*input.get(1 + k)?);
    }
    Some((len, 1 + n))
}

fn write_element(out: &mut Vec<u8>, identifier: &[u8], content: &[u8]) {
    out.extend_from_slice(identifier);
    let len = content.len();
    if len < 0x80 {
        out.push(len as u8);
    } else {
        let bytes = len.to_be_bytes();
        let skip = bytes.iter().take_while(|b| **b == 0).count();
        out.push(0x80 | (bytes.len() - skip) as u8);
        out.extend_from_slice(&bytes[skip..]);
    }
    out.extend_from_slice(content);
}

/// The name's common name, the first if several.
pub fn common_name(name: &Name) -> Option<String> {
    name.0
        .iter()
        .flat_map(|rdn| rdn.0.iter())
        .find(|atv| atv.oid == COMMON_NAME)
        .and_then(|atv| std::str::from_utf8(atv.value.value()).ok())
        .map(str::to_string)
}
