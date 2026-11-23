use bytes::Bytes;
use kvm_omni_core::protocol::OmniPacket;
use quinn::{ClientConfig, Connection, Endpoint, ServerConfig};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use std::net::SocketAddr;
use std::sync::Arc;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum QuicError {
    #[error("certificate generation failed: {0}")]
    Cert(String),
    #[error("endpoint error: {0}")]
    Endpoint(String),
    #[error("connection error: {0}")]
    Connection(String),
    #[error("protocol error: {0}")]
    Protocol(#[from] kvm_omni_core::protocol::ProtocolError),
}

pub struct GeneratedCert {
    pub cert_der: CertificateDer<'static>,
    pub key_der: PrivateKeyDer<'static>,
    pub fingerprint: [u8; 32],
}

pub fn generate_self_signed_cert(sans: Vec<String>) -> Result<GeneratedCert, QuicError> {
    let certified = rcgen::generate_simple_self_signed(sans)
        .map_err(|e| QuicError::Cert(e.to_string()))?;
    let cert_der = certified.cert.der().clone();
    let key_bytes = certified.signing_key.serialize_der();
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_bytes));
    let fingerprint = *blake3::hash(cert_der.as_ref()).as_bytes();
    Ok(GeneratedCert {
        cert_der,
        key_der,
        fingerprint,
    })
}

#[derive(Debug)]
struct TofuSkipServerVerification;

impl ServerCertVerifier for TofuSkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ED25519,
            SignatureScheme::RSA_PSS_SHA256,
        ]
    }
}

pub fn make_server_endpoint(bind_addr: SocketAddr) -> Result<(Endpoint, [u8; 32]), QuicError> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let cert = generate_self_signed_cert(vec!["localhost".into(), "omnikvm.local".into()])?;
    let fp = cert.fingerprint;
    let server_config = ServerConfig::with_single_cert(vec![cert.cert_der], cert.key_der)
        .map_err(|e| QuicError::Endpoint(e.to_string()))?;
    let endpoint =
        Endpoint::server(server_config, bind_addr).map_err(|e| QuicError::Endpoint(e.to_string()))?;
    Ok((endpoint, fp))
}

pub fn make_client_endpoint(bind_addr: SocketAddr) -> Result<Endpoint, QuicError> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(TofuSkipServerVerification))
        .with_no_client_auth();
    let client_cfg = ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(crypto)
            .map_err(|e| QuicError::Endpoint(e.to_string()))?,
    ));
    let mut endpoint =
        Endpoint::client(bind_addr).map_err(|e| QuicError::Endpoint(e.to_string()))?;
    endpoint.set_default_client_config(client_cfg);
    Ok(endpoint)
}

pub fn send_hid_datagram(conn: &Connection, pkt: &OmniPacket) -> Result<(), QuicError> {
    let data = Bytes::from(pkt.serialize());
    conn.send_datagram(data)
        .map_err(|e| QuicError::Connection(e.to_string()))
}

pub async fn recv_hid_datagram(conn: &Connection) -> Result<OmniPacket, QuicError> {
    let data = conn
        .read_datagram()
        .await
        .map_err(|e| QuicError::Connection(e.to_string()))?;
    Ok(OmniPacket::deserialize(&data)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kvm_omni_core::protocol::{flags, MouseMoveDeltaPayload, PayloadType};
    use std::time::Instant;

    #[tokio::test]
    async fn quic_loopback_10k_datagrams() {
        let (server, _fp) = make_server_endpoint("127.0.0.1:0".parse().unwrap()).unwrap();
        let server_addr = server.local_addr().unwrap();
        let client = make_client_endpoint("127.0.0.1:0".parse().unwrap()).unwrap();

        let server_task = tokio::spawn(async move {
            let incoming = server.accept().await.unwrap();
            let conn = incoming.await.unwrap();
            let mut count = 0u32;
            loop {
                tokio::select! {
                    res = recv_hid_datagram(&conn) => {
                        if res.is_ok() {
                            count += 1;
                        } else {
                            break;
                        }
                    }
                    uni = conn.accept_uni() => {
                        if uni.is_ok() {
                            break;
                        }
                    }
                }
            }
            count
        });

        let conn = client
            .connect(server_addr, "localhost")
            .unwrap()
            .await
            .unwrap();

        let start = Instant::now();
        for seq in 0..10_000u32 {
            let pkt = OmniPacket::from_typed(
                flags::URGENT_HID,
                1,
                seq,
                seq as u64 * 100,
                PayloadType::MouseMoveDelta,
                &MouseMoveDeltaPayload { dx: 1, dy: -1 },
            )
            .unwrap();
            send_hid_datagram(&conn, &pkt).unwrap();
            if seq % 250 == 0 {
                tokio::task::yield_now().await;
            }
        }
        // Send a reliable stream sentinel so the server knows the burst is complete
        let mut finish_stream = conn.open_uni().await.unwrap();
        finish_stream.write_all(b"END").await.unwrap();
        finish_stream.finish().unwrap();

        let elapsed = start.elapsed();
        let received = tokio::time::timeout(std::time::Duration::from_secs(5), server_task)
            .await
            .unwrap()
            .unwrap();
        conn.close(0u32.into(), b"done");
        assert!(received > 5_000, "received only {received}/10000");
        assert!(elapsed.as_secs_f64() < 5.0, "took {:?}", elapsed);
    }
}
