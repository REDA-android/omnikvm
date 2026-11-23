//! Real bidirectional encrypted clipboard synchronization over QUIC unidirectional
//! streams using [`ClipboardEnvelope`] (Blake3-XOF + Blake3 integrity check) and
//! native Windows UTF-16 clipboard access (`win_clipboard`).

use kvm_clipboard_sync::envelope::ClipboardEnvelope;
use kvm_input_hal::win_clipboard::{get_clipboard_text, set_clipboard_text};
use kvm_transport_quic::Connection;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MAX_CLIP_BYTES: usize = 1_048_576; // 1 MiB text limit for instant sync

pub async fn push_clipboard_over_quic(
    conn: &Connection,
    session_key: &[u8; 32],
    transfer_id: u64,
    text: &str,
) -> Result<(), String> {
    let env = ClipboardEnvelope::seal(transfer_id, text.as_bytes(), session_key);
    let wire = bincode::serialize(&env).map_err(|e| e.to_string())?;
    let mut send = conn.open_uni().await.map_err(|e| e.to_string())?;
    send.write_all(&wire).await.map_err(|e| e.to_string())?;
    send.finish().map_err(|e| e.to_string())?;
    Ok(())
}

/// Spawns bidirectional clipboard sync tasks (OS watcher -> QUIC & QUIC -> OS clipboard)
/// bound to the lifetime of `conn`.
pub fn spawn_bidirectional_clipboard(
    conn: Connection,
    session_key: [u8; 32],
    clips_synced: Arc<AtomicU64>,
    last_received: Arc<Mutex<Option<String>>>,
) {
    let seen_hash: Arc<Mutex<[u8; 32]>> = Arc::new(Mutex::new(
        get_clipboard_text()
            .map(|t| *blake3::hash(t.as_bytes()).as_bytes())
            .unwrap_or([0u8; 32]),
    ));

    // 1. Inbound: receive encrypted ClipboardEnvelope streams from peer.
    let conn_rx = conn.clone();
    let seen_rx = seen_hash.clone();
    let synced_rx = clips_synced.clone();
    tokio::spawn(async move {
        while let Ok(mut recv) = conn_rx.accept_uni().await {
            let Ok(bytes) = recv.read_to_end(MAX_CLIP_BYTES).await else { continue };
            let Ok(env) = bincode::deserialize::<ClipboardEnvelope>(&bytes) else { continue };
            let Ok(plain) = env.open(&session_key) else { continue };
            let Ok(text) = String::from_utf8(plain) else { continue };
            *seen_rx.lock().unwrap() = env.blake3_digest;
            let _ = set_clipboard_text(&text);
            *last_received.lock().unwrap() = Some(text);
            synced_rx.fetch_add(1, Ordering::Relaxed);
        }
    });

    // 2. Outbound: watch local OS clipboard for changes and push sealed envelope.
    tokio::spawn(async move {
        let mut seq = 1u64;
        let mut tick = tokio::time::interval(Duration::from_millis(650));
        loop {
            tick.tick().await;
            if conn.close_reason().is_some() {
                break;
            }
            let Some(text) = get_clipboard_text() else { continue };
            if text.len() > MAX_CLIP_BYTES {
                continue;
            }
            let digest = *blake3::hash(text.as_bytes()).as_bytes();
            let changed = {
                let mut g = seen_hash.lock().unwrap();
                if *g == digest {
                    false
                } else {
                    *g = digest;
                    true
                }
            };
            if changed && push_clipboard_over_quic(&conn, &session_key, seq, &text).await.is_ok() {
                seq = seq.wrapping_add(1);
                clips_synced.fetch_add(1, Ordering::Relaxed);
            }
        }
    });
}
