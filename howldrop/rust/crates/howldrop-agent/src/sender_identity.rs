//! Explicit import of an Apple-issued sending identity. No credentials are
//! extracted from a Mac here. Windows stores the bundle with user-scoped DPAPI.
use crate::contacts::{Contacts, SenderProfile};
use owly_airdrop::driver::tls::Identity;
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, Read, Write},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

const LIMIT: usize = 64 << 10;
const BUNDLE_LIMIT: usize = 2 << 20;
const HEADER: &[u8] = b"HOWLDROP-SENDER-IDENTITY-1\n";

pub struct SenderIdentity {
    pub tls: Identity,
    pub record: Vec<u8>,
    pub profile: SenderProfile,
}

impl SenderIdentity {
    fn validate(certificate: &[u8], key: &[u8], record: Vec<u8>) -> io::Result<Self> {
        let tls = Identity::from_pem(certificate, key)?;
        let (verifier, _) = Contacts::new(&[])?;
        let profile = verifier.sender_profile(&record, tls.cert.as_ref())?;
        Ok(Self {
            tls,
            record,
            profile,
        })
    }

    pub fn current(&self) -> io::Result<()> {
        if SystemTime::now() >= self.profile.expires {
            Err(io::Error::other(
                "trusted-contact identity expired; import a fresh identity",
            ))
        } else {
            Ok(())
        }
    }

    pub fn expiry(&self) -> u64 {
        self.profile
            .expires
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        let bytes = bounded(path, BUNDLE_LIMIT)?;
        let bytes = bytes
            .strip_prefix(HEADER)
            .ok_or_else(|| io::Error::other("not a HowlDrop sender identity"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if fs::metadata(path)?.permissions().mode() & 0o077 != 0 {
                return Err(io::Error::other(
                    "sender identity must be readable only by its owner (chmod 600)",
                ));
            }
        }
        let plaintext = unprotect(bytes)?;
        let (certificate, key, record) = decode(&plaintext)?;
        Self::validate(&certificate, &key, record)
    }

    pub fn check_export(dir: &Path) -> io::Result<Self> {
        let (certificate, key, record) = export_files(dir)?;
        Self::validate(&certificate, &key, record)
    }

    pub fn import(dir: &Path, destination: &Path) -> io::Result<Self> {
        let (certificate, key, record) = export_files(dir)?;
        let identity = Self::validate(&certificate, &key, record.clone())?;
        let payload = serde_json::to_vec(
            &json!({"version":1,"certificate":certificate,"key":key,"record":record}),
        )?;
        let mut stored = HEADER.to_vec();
        stored.extend(protect(&payload)?);
        atomic_private(destination, &stored)?;
        Ok(identity)
    }
}

fn bounded(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    let mut data = Vec::new();
    fs::File::open(path)?
        .take((limit + 1) as u64)
        .read_to_end(&mut data)?;
    if data.len() > limit {
        return Err(io::Error::other("identity file is too large"));
    }
    Ok(data)
}

type Export = (Vec<u8>, Vec<u8>, Vec<u8>);
fn export_files(dir: &Path) -> io::Result<Export> {
    Ok((
        bounded(&dir.join("certificate.pem"), LIMIT)?,
        bounded(&dir.join("key.pem"), LIMIT)?,
        bounded(&dir.join("validation_record.cms"), LIMIT)?,
    ))
}

fn decode(bytes: &[u8]) -> io::Result<Export> {
    let value: Value = serde_json::from_slice(bytes)?;
    if value.get("version").and_then(Value::as_u64) != Some(1)
        || value.as_object().is_none_or(|v| v.len() != 4)
    {
        return Err(io::Error::other("unsupported identity bundle"));
    }
    let field = |name: &str| -> io::Result<Vec<u8>> {
        let array = value
            .get(name)
            .and_then(Value::as_array)
            .ok_or_else(|| io::Error::other("identity bundle is incomplete"))?;
        if array.is_empty() || array.len() > LIMIT {
            return Err(io::Error::other("invalid identity component size"));
        }
        array
            .iter()
            .map(|n| {
                n.as_u64()
                    .and_then(|n| u8::try_from(n).ok())
                    .ok_or_else(|| io::Error::other("invalid identity component"))
            })
            .collect()
    };
    Ok((field("certificate")?, field("key")?, field("record")?))
}

fn atomic_private(path: &Path, data: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(parent)?;
    let mut nonce = [0; 8];
    getrandom::fill(&mut nonce).map_err(|e| io::Error::other(e.to_string()))?;
    let suffix: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
    let temporary = parent.join(format!(".sender-identity-{suffix}.partial"));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut out = options.open(&temporary)?;
        out.write_all(data)?;
        out.sync_all()?;
        drop(out);
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn crypt(data: &[u8], encrypt: bool) -> io::Result<Vec<u8>> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        },
    };
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(data.len()).map_err(io::Error::other)?,
        pbData: data.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: input borrows the complete slice; DPAPI allocates output, whose
    // documented length is copied before that allocation is freed with LocalFree.
    unsafe {
        let ok = if encrypt {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        if output.cbData == 0 || output.pbData.is_null() {
            LocalFree(output.pbData.cast());
            return Err(io::Error::other("DPAPI returned an empty identity"));
        }
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData.cast());
        Ok(bytes)
    }
}
#[cfg(windows)]
fn protect(data: &[u8]) -> io::Result<Vec<u8>> {
    crypt(data, true)
}
#[cfg(windows)]
fn unprotect(data: &[u8]) -> io::Result<Vec<u8>> {
    crypt(data, false)
}
#[cfg(not(windows))]
fn protect(data: &[u8]) -> io::Result<Vec<u8>> {
    Ok(data.to_vec())
}
#[cfg(not(windows))]
fn unprotect(data: &[u8]) -> io::Result<Vec<u8>> {
    Ok(data.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protected_storage_and_atomic_replacement_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity");
        let payload = b"test identity bundle";
        let protected = protect(payload).unwrap();
        #[cfg(windows)]
        {
            assert_ne!(protected, payload);
            let mut corrupt = protected.clone();
            corrupt[0] ^= 1;
            assert!(unprotect(&corrupt).is_err());
        }
        assert_eq!(unprotect(&protected).unwrap(), payload);
        atomic_private(&path, b"old").unwrap();
        atomic_private(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
    #[test]
    fn malformed_import_never_replaces_an_existing_identity() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("installed");
        fs::write(&destination, b"keep this").unwrap();
        fs::write(dir.path().join("certificate.pem"), b"invalid").unwrap();
        fs::write(dir.path().join("key.pem"), b"invalid").unwrap();
        fs::write(dir.path().join("validation_record.cms"), b"invalid").unwrap();
        assert!(SenderIdentity::import(dir.path(), &destination).is_err());
        assert_eq!(fs::read(destination).unwrap(), b"keep this");
        assert!(decode(br#"{"version":1,"certificate":[],"key":[256],"record":[1]}"#).is_err());
        assert!(decode(br#"{"version":2}"#).is_err());
        fs::write(dir.path().join("key.pem"), vec![0; LIMIT + 1]).unwrap();
        assert!(export_files(dir.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn readable_private_bundles_are_refused_before_decoding() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity");
        fs::write(&path, HEADER).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let error = SenderIdentity::load(&path).err().unwrap();
        assert!(error.to_string().contains("chmod 600"));
    }
}
