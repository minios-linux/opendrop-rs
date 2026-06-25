//! TLS self-signed certificate generation and `rustls::ServerConfig` creation
//! for the AirDrop HTTPS server (port 8771). Apple accepts self-signed certs
//! for AirDrop; we generate an ephemeral one at startup, like opendrop.
//!
//! Reference: `opendrop-patch/server.py.patched` (`get_ssl_context`).

use anyhow::Result;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::ServerConfig;

/// A self-signed certificate + private key for the AirDrop HTTPS server.
pub struct AirDropTlsCert {
    pub cert_der: CertificateDer<'static>,
    pub key_der: PrivateKeyDer<'static>,
}

/// Generate an ephemeral self-signed certificate suitable for AirDrop.
pub fn generate_self_signed_cert() -> Result<AirDropTlsCert> {
    anyhow::bail!("unimplemented")
}

/// Build a `rustls::ServerConfig` from a self-signed cert. No client-cert
/// verification (Apple doesn't send one for AirDrop receive).
pub fn build_server_config(_cert: &AirDropTlsCert) -> Result<ServerConfig> {
    anyhow::bail!("unimplemented")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_cert_produces_non_empty_der() {
        let cert = generate_self_signed_cert().unwrap();
        assert!(!cert.cert_der.as_ref().is_empty());
        // The key's usability is validated by build_server_config below.
    }

    #[test]
    fn build_server_config_from_generated_cert_succeeds() {
        let cert = generate_self_signed_cert().unwrap();
        // Just verify it builds without panicking.
        let _config = build_server_config(&cert).unwrap();
    }
}
