//! 개발용 자체서명 TLS 헬퍼.
//!
//! [GHOST CONSTRAINT]: SkipVerify/자체서명은 개발 전용.
//! Phase 7에서 CA 인증서 핀닝으로 교체 예정.
//! 프로덕션 배포 전 반드시 certbot 인증서로 전환할 것.

use std::sync::Arc;

/// Create a self-signed TLS server config for development.
///
/// # Errors
///
/// Returns [`crate::RpcError::Tls`] on cert generation or TLS setup failure.
pub fn make_server_config(hostnames: &[&str]) -> Result<quinn::ServerConfig, crate::RpcError> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let cert_key = rcgen::generate_simple_self_signed(
        hostnames
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>(),
    )
    .map_err(|e| crate::RpcError::Tls(e.to_string()))?;

    let cert_der = cert_key.cert.der().clone();
    let key_der =
        rustls::pki_types::PrivateKeyDer::try_from(cert_key.signing_key.serialize_der().clone())
            .map_err(|e| crate::RpcError::Tls(e.to_string()))?;

    let server_tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .map_err(|e| crate::RpcError::Tls(e.to_string()))?;

    let quic_server_config = quinn::crypto::rustls::QuicServerConfig::try_from(server_tls)
        .map_err(|e| crate::RpcError::Tls(e.to_string()))?;

    Ok(quinn::ServerConfig::with_crypto(Arc::new(
        quic_server_config,
    )))
}

/// Create a QUIC client config that skips TLS verification (dev only).
///
/// # Errors
///
/// Returns [`crate::RpcError::Tls`] on TLS setup failure.
pub fn make_client_config_skip_verify() -> Result<quinn::ClientConfig, crate::RpcError> {
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::{DigitallySignedStruct, SignatureScheme};

    #[derive(Debug)]
    struct SkipVerify;

    impl rustls::client::danger::ServerCertVerifier for SkipVerify {
        fn verify_server_cert(
            &self,
            _: &CertificateDer,
            _: &[CertificateDer],
            _: &ServerName,
            _: &[u8],
            _: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            Ok(ServerCertVerified::assertion())
        }
        fn verify_tls12_signature(
            &self,
            _: &[u8],
            _: &CertificateDer,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }
        fn verify_tls13_signature(
            &self,
            _: &[u8],
            _: &CertificateDer,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }
        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            vec![
                SignatureScheme::RSA_PKCS1_SHA256,
                SignatureScheme::RSA_PKCS1_SHA384,
                SignatureScheme::ECDSA_NISTP256_SHA256,
                SignatureScheme::ED25519,
            ]
        }
    }

    let _ = rustls::crypto::ring::default_provider().install_default();
    let client_tls = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SkipVerify))
        .with_no_client_auth();

    let quinn_client_config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(client_tls)
            .map_err(|e| crate::RpcError::Tls(e.to_string()))?,
    ));

    Ok(quinn_client_config)
}
