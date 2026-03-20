use std::sync::Arc;

use ed25519_dalek::{SigningKey, pkcs8::EncodePrivateKey};
use rand_core::OsRng;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

fn ensure_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn verify_schemes() -> rustls::crypto::WebPkiSupportedAlgorithms {
    rustls::crypto::ring::default_provider().signature_verification_algorithms
}

pub struct HoshiIdentity {
    signing_key: SigningKey,
}

impl HoshiIdentity {
    pub fn generate() -> Self {
        Self {
            signing_key: SigningKey::generate(&mut OsRng),
        }
    }

    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self {
            signing_key: SigningKey::from_bytes(&seed),
        }
    }

    pub fn seed(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    pub fn public_key_hex(&self) -> String {
        self.signing_key
            .verifying_key()
            .to_bytes()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect()
    }

    pub fn generate_self_signed_cert(&self) -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
        let pkcs8_der = self
            .signing_key
            .to_pkcs8_der()
            .expect("failed to encode Ed25519 key as PKCS#8");

        let key_pair =
            rcgen::KeyPair::try_from(pkcs8_der.as_bytes()).expect("rcgen failed to parse keypair");

        let mut params = rcgen::CertificateParams::new(vec![]).expect("cert params");
        params.distinguished_name = rcgen::DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, self.public_key_hex());

        let cert = params.self_signed(&key_pair).expect("self-sign");
        let cert_der = CertificateDer::from(cert.der().to_vec());
        let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(pkcs8_der.as_bytes().to_vec()));

        (cert_der, key_der)
    }

    pub fn make_client_tls_config(&self) -> rustls::ClientConfig {
        ensure_crypto_provider();
        let (cert_der, key_der) = self.generate_self_signed_cert();

        rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAllServerCerts))
            .with_client_auth_cert(vec![cert_der], key_der)
            .expect("client TLS config")
    }

    pub fn make_server_tls_config(&self) -> rustls::ServerConfig {
        ensure_crypto_provider();
        let (cert_der, key_der) = self.generate_self_signed_cert();

        rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
            .with_client_cert_verifier(Arc::new(RequireClientCert))
            .with_single_cert(vec![cert_der], key_der)
            .expect("server TLS config")
    }
}

/// Extract the Ed25519 public key hex from a DER-encoded X.509 certificate.
pub fn extract_ed25519_public_key_hex(cert_der: &[u8]) -> Option<String> {
    use ed25519_dalek::pkcs8::DecodePublicKey;
    use x509_cert::der::{Decode, Encode};

    let cert = x509_cert::Certificate::from_der(cert_der).ok()?;
    let spki_der = cert.tbs_certificate.subject_public_key_info.to_der().ok()?;
    let vk = ed25519_dalek::VerifyingKey::from_public_key_der(&spki_der).ok()?;
    Some(vk.to_bytes().iter().map(|b| format!("{:02x}", b)).collect())
}

// --- TLS verifiers ---
//
// We accept any certificate (no chain verification) but we DO verify TLS
// handshake signatures. This proves the peer actually owns the private key
// for whatever cert they present. Chain verification will be added when
// the control plane CA is implemented.

#[derive(Debug)]
struct AcceptAllServerCerts;

impl rustls::client::danger::ServerCertVerifier for AcceptAllServerCerts {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        unreachable!("TLS 1.2 is disabled")
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &verify_schemes())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        verify_schemes().supported_schemes()
    }
}

#[derive(Debug)]
struct RequireClientCert;

impl rustls::server::danger::ClientCertVerifier for RequireClientCert {
    fn root_hint_subjects(&self) -> &[rustls::DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::server::danger::ClientCertVerified, rustls::Error> {
        Ok(rustls::server::danger::ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        unreachable!("TLS 1.2 is disabled")
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &verify_schemes())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        verify_schemes().supported_schemes()
    }

    fn client_auth_mandatory(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_roundtrip() {
        let identity = HoshiIdentity::generate();
        let seed = identity.seed();
        let restored = HoshiIdentity::from_seed(seed);
        assert_eq!(identity.public_key_hex(), restored.public_key_hex());
    }

    #[test]
    fn public_key_is_64_hex_chars() {
        let identity = HoshiIdentity::generate();
        let hex = identity.public_key_hex();
        assert_eq!(hex.len(), 64);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn cert_contains_correct_public_key() {
        let identity = HoshiIdentity::generate();
        let (cert_der, _) = identity.generate_self_signed_cert();
        let extracted = extract_ed25519_public_key_hex(cert_der.as_ref()).unwrap();
        assert_eq!(extracted, identity.public_key_hex());
    }
}
