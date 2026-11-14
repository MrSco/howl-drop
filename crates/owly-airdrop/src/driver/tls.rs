//! TLS for AirDrop: encryption only, no peer authentication (§6.1).
//!
//! Both sides offer TLS 1.2 and 1.3 and nothing older (TLS-1). The receiver
//! presents a self-signed certificate made once per install, and accepts
//! clients with or without a certificate (TLS-3). The sender checks no
//! hostname or chain; an anonymous sender presents no certificate, while
//! an identified sender presents its imported chain (TLS-2, TLS-3). Handshake
//! signatures are still verified, so a peer has to hold the key for the
//! certificate it shows.
//!
//! The crypto is rustls's process-wide default provider, which the
//! application installs, for example with
//! `rustls::crypto::ring::default_provider().install_default()`. That keeps
//! this crate free of C code, so it builds for any target.

use std::io;
use std::path::Path;
use std::sync::Arc;

use p256::ecdsa::signature::Signer;
use p256::pkcs8::EncodePrivateKey;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{
    CryptoProvider, WebPkiSupportedAlgorithms, verify_tls12_signature, verify_tls13_signature,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{
    ClientConfig, DigitallySignedStruct, DistinguishedName, ServerConfig, SignatureScheme,
};

pub use rustls;

/// A certificate and its key.
pub struct Identity {
    pub cert: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
    chain: Vec<CertificateDer<'static>>,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("certificates", &self.chain.len())
            .finish_non_exhaustive()
    }
}

impl Clone for Identity {
    fn clone(&self) -> Self {
        Identity {
            cert: self.cert.clone(),
            key: self.key.clone_key(),
            chain: self.chain.clone(),
        }
    }
}

const CERT_FILE: &str = "tls-cert.der";
const KEY_FILE: &str = "tls-key.der";

impl Identity {
    /// A fresh self-signed certificate on a P-256 key (TLS-2).
    pub fn generate() -> io::Result<Self> {
        let key = loop {
            let mut bytes = [0u8; 32];
            getrandom::fill(&mut bytes).map_err(|e| io::Error::other(e.to_string()))?;
            // Fails only for the rare value outside the curve's order.
            if let Ok(key) = p256::ecdsa::SigningKey::from_bytes(&bytes.into()) {
                break EcKey::new(key);
            }
        };
        let mut params = rcgen::CertificateParams::new(Vec::new()).map_err(io::Error::other)?;
        let mut serial = [0u8; 16];
        getrandom::fill(&mut serial).map_err(|e| io::Error::other(e.to_string()))?;
        // Positive, as DER integers for serial numbers must be.
        serial[0] &= 0x7f;
        params.serial_number = Some(rcgen::SerialNumber::from_slice(&serial));
        params.distinguished_name = rcgen::DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Owly");
        let cert = params.self_signed(&key).map_err(io::Error::other)?;
        let pkcs8 = key.key.to_pkcs8_der().map_err(io::Error::other)?;
        Ok(Identity {
            cert: cert.der().clone(),
            key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(pkcs8.as_bytes().to_vec())),
            chain: vec![cert.der().clone()],
        })
    }

    /// The identity stored in `dir`, made on first use. The key file is
    /// readable by its owner only.
    pub fn load_or_generate(dir: &Path) -> io::Result<Self> {
        let (cert_path, key_path) = (dir.join(CERT_FILE), dir.join(KEY_FILE));
        match (std::fs::read(&cert_path), std::fs::read(&key_path)) {
            (Ok(cert), Ok(key)) => Self::from_der_chain(
                vec![CertificateDer::from(cert)],
                PrivateKeyDer::try_from(key).map_err(io::Error::other)?,
            ),
            _ => {
                let identity = Self::generate()?;
                std::fs::create_dir_all(dir)?;
                write_private(&key_path, identity.key.secret_der())?;
                std::fs::write(&cert_path, identity.cert.as_ref())?;
                Ok(identity)
            }
        }
    }

    fn key(&self) -> PrivateKeyDer<'static> {
        self.key.clone_key()
    }

    /// Import a PEM chain and unencrypted PEM private key, checking that the
    /// leaf certificate belongs to the key. No Apple trust decision is made here.
    pub fn from_pem(certificates: &[u8], key: &[u8]) -> io::Result<Self> {
        use rustls::pki_types::pem::PemObject;
        let chain = CertificateDer::pem_slice_iter(certificates)
            .collect::<Result<Vec<_>, _>>()
            .map_err(io::Error::other)?;
        let key = PrivateKeyDer::from_pem_slice(key)
            .map_err(|_| io::Error::other("key.pem must contain an unencrypted PEM private key"))?;
        Self::from_der_chain(chain, key)
    }

    fn from_der_chain(
        chain: Vec<CertificateDer<'static>>,
        key: PrivateKeyDer<'static>,
    ) -> io::Result<Self> {
        let cert = chain
            .first()
            .ok_or_else(|| io::Error::other("certificate chain is empty"))?
            .clone();
        let certified = rustls::sign::CertifiedKey::from_der(
            chain.clone(),
            key.clone_key(),
            provider()?.as_ref(),
        )
        .map_err(io::Error::other)?;
        certified.keys_match().map_err(io::Error::other)?;
        Ok(Self { cert, key, chain })
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    use base64::Engine;

    fn pem(label: &str, der: &[u8]) -> Vec<u8> {
        let data = base64::engine::general_purpose::STANDARD.encode(der);
        format!("-----BEGIN {label}-----\n{data}\n-----END {label}-----\n").into_bytes()
    }

    #[test]
    fn imported_keys_must_match_and_debug_never_contains_them() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let a = Identity::generate().unwrap();
        let b = Identity::generate().unwrap();
        assert!(Identity::from_der_chain(vec![a.cert.clone()], a.key()).is_ok());
        assert!(Identity::from_der_chain(vec![a.cert.clone()], b.key()).is_err());
        assert!(Identity::from_der_chain(Vec::new(), a.key()).is_err());
        let certificate = pem("CERTIFICATE", a.cert.as_ref());
        let key = pem("PRIVATE KEY", a.key.secret_der());
        let mut chain = certificate.clone();
        chain.extend(pem("CERTIFICATE", b.cert.as_ref()));
        let imported = Identity::from_pem(&chain, &key).unwrap();
        assert_eq!(imported.cert, a.cert);
        assert_eq!(imported.chain, vec![a.cert.clone(), b.cert.clone()]);
        assert!(Identity::from_pem(&certificate, &pem("PRIVATE KEY", b.key.secret_der())).is_err());
        assert!(Identity::from_pem(&[], &key).is_err());
        assert!(Identity::from_pem(&certificate, b"encrypted or malformed key").is_err());
        assert_eq!(format!("{a:?}"), "Identity { certificates: 1, .. }");
    }
}

fn write_private(path: &Path, data: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)?.write_all(data)
}

/// A P-256 key that signs for rcgen without rcgen's own crypto backend.
struct EcKey {
    key: p256::ecdsa::SigningKey,
    public: Vec<u8>,
}

impl EcKey {
    fn new(key: p256::ecdsa::SigningKey) -> Self {
        let public = key
            .verifying_key()
            .to_encoded_point(false)
            .as_bytes()
            .to_vec();
        EcKey { key, public }
    }
}

impl rcgen::PublicKeyData for EcKey {
    /// The uncompressed point, as SubjectPublicKeyInfo carries it.
    fn der_bytes(&self) -> &[u8] {
        &self.public
    }

    fn algorithm(&self) -> &'static rcgen::SignatureAlgorithm {
        &rcgen::PKCS_ECDSA_P256_SHA256
    }
}

impl rcgen::SigningKey for EcKey {
    /// ECDSA over SHA-256, DER-encoded.
    fn sign(&self, msg: &[u8]) -> Result<Vec<u8>, rcgen::Error> {
        let signature: p256::ecdsa::Signature = self.key.sign(msg);
        Ok(signature.to_der().as_bytes().to_vec())
    }
}

fn provider() -> io::Result<Arc<CryptoProvider>> {
    CryptoProvider::get_default().cloned().ok_or_else(|| {
        io::Error::other("no rustls crypto provider installed; the application has to install one")
    })
}

/// TLS-1: 1.2 and 1.3 only. rustls has nothing older.
pub const VERSIONS: &[&rustls::SupportedProtocolVersion] =
    &[&rustls::version::TLS13, &rustls::version::TLS12];

/// The receiver's configuration.
pub fn server_config(identity: &Identity) -> io::Result<Arc<ServerConfig>> {
    let provider = provider()?;
    let verifier = Arc::new(AnyClient(provider.signature_verification_algorithms));
    let config = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(VERSIONS)
        .map_err(io::Error::other)?
        .with_client_cert_verifier(verifier)
        .with_single_cert(identity.chain.clone(), identity.key())
        .map_err(io::Error::other)?;
    Ok(Arc::new(config))
}

/// The sender's configuration. `identity` is a client certificate to
/// present; senders don't need one (TLS-3), it's there for tests.
pub fn client_config(identity: Option<&Identity>) -> io::Result<Arc<ClientConfig>> {
    client_config_with_versions(identity, VERSIONS)
}

/// A sender configuration offering only some of [`VERSIONS`], for tests.
#[doc(hidden)]
pub fn client_config_with_versions(
    identity: Option<&Identity>,
    versions: &[&'static rustls::SupportedProtocolVersion],
) -> io::Result<Arc<ClientConfig>> {
    let provider = provider()?;
    let verifier = Arc::new(AnyServer(provider.signature_verification_algorithms));
    let builder = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(versions)
        .map_err(io::Error::other)?
        .dangerous()
        .with_custom_certificate_verifier(verifier);
    let config = match identity {
        Some(identity) => builder
            .with_client_auth_cert(identity.chain.clone(), identity.key())
            .map_err(io::Error::other)?,
        None => builder.with_no_client_auth(),
    };
    Ok(Arc::new(config))
}

/// The server name a sender sends. AirDrop peers have none, and receivers
/// don't check it.
pub fn server_name() -> ServerName<'static> {
    ServerName::try_from("airdrop.local").unwrap()
}

/// Accepts any server certificate, self-signed included (TLS-2).
#[derive(Debug)]
struct AnyServer(WebPkiSupportedAlgorithms);

impl ServerCertVerifier for AnyServer {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.0)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.0)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_schemes()
    }
}

/// Offers client authentication without requiring it, and accepts any
/// certificate a client does present (TLS-3).
#[derive(Debug)]
struct AnyClient(WebPkiSupportedAlgorithms);

impl ClientCertVerifier for AnyClient {
    fn offer_client_auth(&self) -> bool {
        true
    }

    fn client_auth_mandatory(&self) -> bool {
        false
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.0)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.0)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_schemes()
    }
}
