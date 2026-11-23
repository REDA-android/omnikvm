use crate::QuicError;
use kvm_omni_core::crypto::{
    compute_pairing_proof, derive_session_key, verify_pairing_proof, TofuKeystore,
};
use quinn::Connection;
use rand::RngCore;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClientHello {
    peer_id: String,
    client_nonce: [u8; 32],
    cert_fingerprint: [u8; 32],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ServerChallenge {
    peer_id: String,
    server_nonce: [u8; 32],
    cert_fingerprint: [u8; 32],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClientProof {
    mac: [u8; 32],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ServerAuthResult {
    accepted: bool,
    server_mac: [u8; 32],
}

async fn write_frame<T: Serialize>(send: &mut quinn::SendStream, msg: &T) -> Result<(), QuicError> {
    let data = bincode::serialize(msg).map_err(|e| QuicError::Connection(e.to_string()))?;
    send.write_all(&(data.len() as u32).to_be_bytes())
        .await
        .map_err(|e| QuicError::Connection(e.to_string()))?;
    send.write_all(&data)
        .await
        .map_err(|e| QuicError::Connection(e.to_string()))?;
    Ok(())
}

async fn read_frame<T: for<'de> Deserialize<'de>>(
    recv: &mut quinn::RecvStream,
) -> Result<T, QuicError> {
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf)
        .await
        .map_err(|e| QuicError::Connection(e.to_string()))?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 64 * 1024 {
        return Err(QuicError::Connection("handshake frame too large".into()));
    }
    let mut buf = vec![0u8; len];
    recv.read_exact(&mut buf)
        .await
        .map_err(|e| QuicError::Connection(e.to_string()))?;
    bincode::deserialize(&buf).map_err(|e| QuicError::Connection(e.to_string()))
}

/// Performs the client side of the SPAKE2+-style authenticated QUIC handshake.
/// Returns the derived 32-byte session key on mutual verification.
pub async fn client_handshake(
    conn: &Connection,
    local_peer_id: &str,
    local_fp: [u8; 32],
    pin: &str,
    keystore: &mut TofuKeystore,
) -> Result<[u8; 32], QuicError> {
    let (mut send, mut recv) = conn
        .open_bi()
        .await
        .map_err(|e| QuicError::Connection(e.to_string()))?;

    let mut client_nonce = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut client_nonce);

    write_frame(
        &mut send,
        &ClientHello {
            peer_id: local_peer_id.to_string(),
            client_nonce,
            cert_fingerprint: local_fp,
        },
    )
    .await?;

    let chal: ServerChallenge = read_frame(&mut recv).await?;
    keystore
        .verify_or_trust(&chal.peer_id, chal.cert_fingerprint)
        .map_err(|e| QuicError::Connection(e.to_string()))?;

    let session_key = derive_session_key(pin, &client_nonce, &chal.server_nonce);
    let mac = compute_pairing_proof(&session_key, local_peer_id, &chal.server_nonce);
    write_frame(&mut send, &ClientProof { mac }).await?;

    let result: ServerAuthResult = read_frame(&mut recv).await?;
    if !result.accepted {
        return Err(QuicError::Connection("server rejected pairing PIN".into()));
    }
    verify_pairing_proof(&session_key, &chal.peer_id, &client_nonce, &result.server_mac)
        .map_err(|e| QuicError::Connection(e.to_string()))?;

    let _ = send.finish();
    Ok(session_key)
}

/// Performs the server side of the SPAKE2+-style authenticated QUIC handshake.
pub async fn server_handshake(
    conn: &Connection,
    local_peer_id: &str,
    local_fp: [u8; 32],
    expected_pin: &str,
    keystore: &mut TofuKeystore,
) -> Result<(String, [u8; 32]), QuicError> {
    let (mut send, mut recv) = conn
        .accept_bi()
        .await
        .map_err(|e| QuicError::Connection(e.to_string()))?;

    let hello: ClientHello = read_frame(&mut recv).await?;
    keystore
        .verify_or_trust(&hello.peer_id, hello.cert_fingerprint)
        .map_err(|e| QuicError::Connection(e.to_string()))?;

    let mut server_nonce = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut server_nonce);

    write_frame(
        &mut send,
        &ServerChallenge {
            peer_id: local_peer_id.to_string(),
            server_nonce,
            cert_fingerprint: local_fp,
        },
    )
    .await?;

    let proof: ClientProof = read_frame(&mut recv).await?;
    let session_key = derive_session_key(expected_pin, &hello.client_nonce, &server_nonce);
    let ok = verify_pairing_proof(&session_key, &hello.peer_id, &server_nonce, &proof.mac).is_ok();
    let server_mac = compute_pairing_proof(&session_key, local_peer_id, &hello.client_nonce);

    write_frame(
        &mut send,
        &ServerAuthResult {
            accepted: ok,
            server_mac,
        },
    )
    .await?;
    let _ = send.finish();

    if !ok {
        return Err(QuicError::Connection("client PIN verification failed".into()));
    }
    Ok((hello.peer_id, session_key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{make_client_endpoint, make_server_endpoint};

    #[tokio::test]
    async fn authenticated_quic_handshake_succeeds_and_rejects_wrong_pin() {
        let (server, server_fp) = make_server_endpoint("127.0.0.1:0".parse().unwrap()).unwrap();
        let addr = server.local_addr().unwrap();
        let client = make_client_endpoint("127.0.0.1:0".parse().unwrap()).unwrap();

        let srv = tokio::spawn(async move {
            let mut ks = TofuKeystore::new();
            // 1. Valid PIN connection
            let c1 = server.accept().await.unwrap().await.unwrap();
            let (peer, sk) = server_handshake(&c1, "host-a", server_fp, "482910", &mut ks)
                .await
                .unwrap();
            assert_eq!(peer, "client-b");
            // 2. Wrong PIN connection
            let c2 = server.accept().await.unwrap().await.unwrap();
            let res2 = server_handshake(&c2, "host-a", server_fp, "482910", &mut ks).await;
            assert!(res2.is_err());
            sk
        });

        let mut c_ks = TofuKeystore::new();
        let conn1 = client.connect(addr, "localhost").unwrap().await.unwrap();
        let ck = client_handshake(&conn1, "client-b", [0xAA; 32], "482910", &mut c_ks)
            .await
            .unwrap();
        conn1.close(0u32.into(), b"ok");

        let conn2 = client.connect(addr, "localhost").unwrap().await.unwrap();
        let bad = client_handshake(&conn2, "client-b", [0xAA; 32], "000000", &mut c_ks).await;
        assert!(bad.is_err());
        conn2.close(0u32.into(), b"bad");

        let sk = srv.await.unwrap();
        assert_eq!(ck, sk);
    }
}
