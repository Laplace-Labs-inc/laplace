// SPDX-License-Identifier: Apache-2.0
use rustls::ClientConfig;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use std::sync::Arc;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Build a verified rustls client config from a CA certificate PEM.
///
/// Loads the CA into a `RootCertStore` and returns a `ClientConfig` that
/// validates the server certificate against it.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "40_Probe_Transport",
        link = "LEP-0015-laplace-probe-matrix_di_and_chaos"
    )
)]
pub fn verified_client_config(
    ca_cert_pem: &[u8],
) -> Result<ClientConfig, Box<dyn std::error::Error>> {
    // rustls v0.23: install ring crypto provider before building configs
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut root_store = rustls::RootCertStore::empty();
    let certs: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(ca_cert_pem).collect::<Result<Vec<_>, _>>()?;
    for cert in certs {
        root_store.add(cert)?;
    }

    let config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    Ok(config)
}

/// Build a `quinn::ServerConfig` from PEM-encoded cert and key byte slices.
///
/// The caller is responsible for reading the files and passing the raw bytes.
/// No file I/O is performed inside this function.
pub fn load_server_tls(
    cert_pem: &[u8],
    key_pem: &[u8],
) -> Result<quinn::ServerConfig, Box<dyn std::error::Error>> {
    let cert_chain: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(cert_pem).collect::<Result<Vec<_>, _>>()?;

    if cert_chain.is_empty() {
        return Err("No certificate found in PEM".into());
    }

    let key = PrivateKeyDer::from_pem_slice(key_pem).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("No private key found in PEM file: {e}"),
        )
    })?;

    let server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key)?;

    let quic_server_config = quinn::crypto::rustls::QuicServerConfig::try_from(server_config)?;

    Ok(quinn::ServerConfig::with_crypto(Arc::new(
        quic_server_config,
    )))
}
