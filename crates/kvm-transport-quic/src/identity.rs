//! Persistent node identity (self-signed cert reused across restarts so TOFU pins stay
//! valid), TLS-bound peer fingerprint extraction, and sealed (MAC + anti-replay) HID
//! datagram helpers.

use crate::{GeneratedCert, QuicError};
use bytes::Bytes;
use kvm_omni_core::datagram_auth::{DatagramOpener, DatagramSealer};
use kvm_omni_core::protocol::OmniPacket;
use quinn::{Connection, Endpoint, ServerConfig};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::net::SocketAddr;
use std::path::Path;

const CERT_FILE: &str = "node_cert.der";
const KEY_FILE: &str = "node_key.der";

fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Loads `node_cert.der` / `node_key.der` from `dir`, generating and persisting them
/// on first run. Returns the identity and whether it was freshly created.
pub fn load_or_generate_identity(dir: &Path) -> Result<(GeneratedCert, bool), QuicError> {
    let cert_p = dir.join(CERT_FILE);
    let key_p = dir.join(KEY_FILE);
    if let (Ok(cert), Ok(key)) = (std::fs::read(&cert_p), std::fs::read(&key_p)) {
        let fingerprint = *blake3::hash(&cert).as_bytes();
        return Ok((
            GeneratedCert {
                cert_der: CertificateDer::from(cert),
                key_der: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key)),
                fingerprint,
            },
            false,
        ));
    }
    let gen = crate::generate_self_signed_cert(vec!["localhost".into(), "omnikvm.local".into()])?;
    std::fs::create_dir_all(dir).map_err(|e| QuicError::Cert(e.to_string()))?;
    std::fs::write(&cert_p, gen.cert_der.as_ref()).map_err(|e| QuicError::Cert(e.to_string()))?;
    write_private(&key_p, gen.key_der.secret_der()).map_err(|e| QuicError::Cert(e.to_string()))?;
    Ok((gen, true))
}

/// Builds a QUIC server endpoint from an existing identity.
pub fn make_server_endpoint_with_cert(
    bind_addr: SocketAddr,
    cert: GeneratedCert,
) -> Result<(Endpoint, [u8; 32]), QuicError> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let fp = cert.fingerprint;
    let server_config = ServerConfig::with_single_cert(vec![cert.cert_der], cert.key_der)
        .map_err(|e| QuicError::Endpoint(e.to_string()))?;
    let endpoint =
        Endpoint::server(server_config, bind_addr).map_err(|e| QuicError::Endpoint(e.to_string()))?;
    Ok((endpoint, fp))
}

/// Blake3 fingerprint of the certificate the remote peer actually presented in TLS.
pub fn peer_cert_fingerprint(conn: &Connection) -> Option<[u8; 32]> {
    let any = conn.peer_identity()?;
    let certs = any.downcast::<Vec<CertificateDer<'static>>>().ok()?;
    certs.first().map(|c| *blake3::hash(c.as_ref()).as_bytes())
}

/// Sends an OmniKVM frame sealed with the session MAC + sequence number.
pub fn send_sealed_datagram(
    conn: &Connection,
    sealer: &mut DatagramSealer,
    pkt: &OmniPacket,
) -> Result<(), QuicError> {
    let sealed = sealer.seal(&pkt.serialize());
    conn.send_datagram(Bytes::from(sealed))
        .map_err(|e| QuicError::Connection(e.to_string()))
}

/// Receives the next sealed datagram. Forged / replayed datagrams yield `Ok(None)`
/// (silently dropped, counted by the caller) instead of tearing down the session.
pub async fn recv_sealed_datagram(
    conn: &Connection,
    opener: &mut DatagramOpener,
) -> Result<Option<OmniPacket>, QuicError> {
    let data = conn
        .read_datagram()
        .await
        .map_err(|e| QuicError::Connection(e.to_string()))?;
    match opener.open(&data) {
        Ok(frame) => Ok(Some(OmniPacket::deserialize(frame)?)),
        Err(_) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_persists_across_loads() {
        let dir = std::env::temp_dir().join(format!("omnikvm_id_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (a, fresh_a) = load_or_generate_identity(&dir).unwrap();
        let (b, fresh_b) = load_or_generate_identity(&dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(fresh_a);
        assert!(!fresh_b);
        assert_eq!(a.fingerprint, b.fingerprint);
    }
}
