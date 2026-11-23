use anyhow::Result;
use kvm_clipboard_sync::streaming::{
    chunk_bytes_encrypted, ChunkReassembler, FileChunk, DEFAULT_CHUNK_SIZE,
};
use kvm_transport_quic::{make_client_endpoint, make_server_endpoint};
use std::net::SocketAddr;
use std::path::Path;

/// Streams a file over a reliable QUIC unidirectional stream in 64 KB Blake3-XOF encrypted chunks.
pub async fn send_file_quic(
    remote_addr: SocketAddr,
    file_path: &Path,
    session_key: [u8; 32],
) -> Result<[u8; 32]> {
    let data = std::fs::read(file_path)?;
    let (chunks, digest) = chunk_bytes_encrypted(1, &data, DEFAULT_CHUNK_SIZE, &session_key);

    let endpoint = make_client_endpoint("0.0.0.0:0".parse()?)?;
    let conn = endpoint.connect(remote_addr, "localhost")?.await?;
    let mut send = conn.open_uni().await?;

    // Header: expected Blake3 digest (32 bytes) + chunk count (u64 BE)
    send.write_all(&digest).await?;
    send.write_all(&(chunks.len() as u64).to_be_bytes()).await?;

    for chunk in chunks {
        let encoded = bincode::serialize(&chunk)?;
        send.write_all(&(encoded.len() as u32).to_be_bytes()).await?;
        send.write_all(&encoded).await?;
    }
    send.finish()?;
    // Allow peer to finish reading stream before closing connection
    let _ = send.stopped().await;
    conn.close(0u32.into(), b"file-sent");
    Ok(digest)
}

/// Receives a Blake3-XOF encrypted file stream over QUIC, verifies its digest, and writes to disk.
pub async fn recv_file_quic(
    bind_addr: SocketAddr,
    out_path: &Path,
    session_key: [u8; 32],
) -> Result<(SocketAddr, tokio::task::JoinHandle<Result<[u8; 32]>>)> {
    let (endpoint, _fp) = make_server_endpoint(bind_addr)?;
    let local_addr = endpoint.local_addr()?;
    let out_buf = out_path.to_path_buf();

    let handle = tokio::spawn(async move {
        let incoming = endpoint
            .accept()
            .await
            .ok_or_else(|| anyhow::anyhow!("endpoint closed"))?;
        let conn = incoming.await?;
        let mut recv = conn.accept_uni().await?;

        let mut expected_digest = [0u8; 32];
        recv.read_exact(&mut expected_digest).await?;

        let mut count_bytes = [0u8; 8];
        recv.read_exact(&mut count_bytes).await?;
        let total_chunks = u64::from_be_bytes(count_bytes);

        let mut reassembler = ChunkReassembler::new(session_key, (total_chunks as usize) * DEFAULT_CHUNK_SIZE);
        for _ in 0..total_chunks {
            let mut len_bytes = [0u8; 4];
            recv.read_exact(&mut len_bytes).await?;
            let frame_len = u32::from_be_bytes(len_bytes) as usize;
            let mut frame = vec![0u8; frame_len];
            recv.read_exact(&mut frame).await?;
            let chunk: FileChunk = bincode::deserialize(&frame)?;
            reassembler.push_chunk(chunk)?;
        }

        let restored = reassembler.finalize_and_verify(&expected_digest)?;
        std::fs::write(&out_buf, &restored)?;
        Ok(expected_digest)
    });

    Ok((local_addr, handle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn e2e_quic_encrypted_file_transfer() {
        let src = std::env::temp_dir().join("omnikvm_xfer_src.bin");
        let dst = std::env::temp_dir().join("omnikvm_xfer_dst.bin");
        let payload: Vec<u8> = (0..(256 * 1024)).map(|i| (i % 251) as u8).collect();
        std::fs::write(&src, &payload).unwrap();

        let key = [0x77u8; 32];
        let (addr, server_handle) = recv_file_quic("127.0.0.1:0".parse().unwrap(), &dst, key)
            .await
            .unwrap();

        let sent_digest = send_file_quic(addr, &src, key).await.unwrap();
        let recv_digest = tokio::time::timeout(std::time::Duration::from_secs(5), server_handle)
            .await
            .unwrap()
            .unwrap()
            .unwrap();

        assert_eq!(sent_digest, recv_digest);
        let restored = std::fs::read(&dst).unwrap();
        assert_eq!(restored, payload);

        let _ = std::fs::remove_file(&src);
        let _ = std::fs::remove_file(&dst);
    }
}
