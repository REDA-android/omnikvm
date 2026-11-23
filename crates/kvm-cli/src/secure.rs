//! Authenticated live daemon: persistent TLS identity + SPAKE2+/TOFU handshake +
//! session-keyed sealed HID datagrams (MAC + anti-replay). Trusted peers are persisted
//! into `OmniConfig.trusted_peers` so subsequent sessions are pinned.

use crate::daemon::{event_to_packet, packet_to_event};
use anyhow::{Context, Result};
use kvm_input_hal::{InputEvent, SystemInputInjector};
use kvm_omni_core::config::OmniConfig;
use kvm_omni_core::crypto::TofuKeystore;
use kvm_omni_core::datagram_auth::{DatagramOpener, DatagramSealer};
use kvm_transport_quic::handshake::{client_handshake, server_handshake};
use kvm_transport_quic::identity::{
    load_or_generate_identity, make_server_endpoint_with_cert, recv_sealed_datagram,
    send_sealed_datagram,
};
use kvm_transport_quic::{make_client_endpoint, Connection, Endpoint};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct SecureOpts {
    pub pin: String,
    pub config_path: PathBuf,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SecureStats {
    pub injected: usize,
    pub dropped_forged: usize,
    pub rejected_sessions: usize,
}

fn identity_dir(config_path: &Path) -> PathBuf {
    match config_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.join("identity"),
        _ => PathBuf::from("identity"),
    }
}

fn persist_trust(cfg: &mut OmniConfig, ks: &TofuKeystore, path: &Path) -> Result<()> {
    let map = ks.to_hex_map();
    if map != cfg.trusted_peers {
        cfg.trusted_peers = map;
        cfg.save(path).context("saving trusted peers")?;
    }
    Ok(())
}

/// Runs the authenticated receiver. Sessions are served sequentially; when
/// `max_events` is set the task returns once that many events were injected.
pub async fn run_secure_server(
    bind_addr: SocketAddr,
    injector: Arc<dyn SystemInputInjector>,
    opts: SecureOpts,
    max_events: Option<usize>,
) -> Result<(SocketAddr, tokio::task::JoinHandle<Result<SecureStats>>)> {
    let mut cfg = OmniConfig::load_or_default(&opts.config_path)?;
    let mut ks = TofuKeystore::from_hex_map(&cfg.trusted_peers);
    let (ident, fresh) = load_or_generate_identity(&identity_dir(&opts.config_path))?;
    let (endpoint, fp) = make_server_endpoint_with_cert(bind_addr, ident)?;
    let local_addr = endpoint.local_addr()?;
    println!(
        "OmniKVM secure server '{}' on {local_addr} (identity {}, fp {:02x?}, {} trusted peer(s))",
        cfg.peer_id,
        if fresh { "generated" } else { "loaded" },
        &fp[..4],
        ks.len()
    );

    let handle = tokio::spawn(async move {
        let mut stats = SecureStats::default();
        while let Some(incoming) = endpoint.accept().await {
            let Ok(conn) = incoming.await else { continue };
            let (peer, sk) = match server_handshake(&conn, &cfg.peer_id, fp, &opts.pin, &mut ks).await {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("WARN: rejected session from {}: {e}", conn.remote_address());
                    stats.rejected_sessions += 1;
                    conn.close(1u32.into(), b"auth failed");
                    continue;
                }
            };
            persist_trust(&mut cfg, &ks, &opts.config_path)?;
            println!("Authenticated peer '{peer}' from {}", conn.remote_address());
            let mut opener = DatagramOpener::new(&sk);
            while let Ok(next) = recv_sealed_datagram(&conn, &mut opener).await {
                let Some(pkt) = next else {
                    stats.dropped_forged += 1;
                    continue;
                };
                if let Some(ev) = packet_to_event(&pkt)? {
                    injector.inject(ev)?;
                    stats.injected += 1;
                    if max_events.is_some_and(|m| stats.injected >= m) {
                        return Ok(stats);
                    }
                }
            }
        }
        Ok(stats)
    });
    Ok((local_addr, handle))
}

/// Connects to `remote_addr`, authenticates with `opts.pin`, pins the server identity
/// (TOFU, persisted) and returns a live connection plus its datagram sealer.
pub async fn connect_secure(
    remote_addr: SocketAddr,
    opts: &SecureOpts,
) -> Result<(Endpoint, Connection, DatagramSealer)> {
    let mut cfg = OmniConfig::load_or_default(&opts.config_path)?;
    let mut ks = TofuKeystore::from_hex_map(&cfg.trusted_peers);
    let (ident, _) = load_or_generate_identity(&identity_dir(&opts.config_path))?;
    let endpoint = make_client_endpoint("0.0.0.0:0".parse()?)?;
    let conn = endpoint.connect(remote_addr, "localhost")?.await?;
    let sk = client_handshake(&conn, &cfg.peer_id, ident.fingerprint, &opts.pin, &mut ks)
        .await
        .context("authenticated handshake failed")?;
    persist_trust(&mut cfg, &ks, &opts.config_path)?;
    Ok((endpoint, conn, DatagramSealer::new(&sk)))
}

/// Streams sealed HID datagrams from `rx` until the channel closes.
pub async fn forward_stream(
    conn: &Connection,
    sealer: &mut DatagramSealer,
    mut rx: tokio::sync::mpsc::Receiver<InputEvent>,
) -> Result<usize> {
    let mut seq = 0u32;
    while let Some(ev) = rx.recv().await {
        send_sealed_datagram(conn, sealer, &event_to_packet(seq, ev)?)?;
        seq = seq.wrapping_add(1);
    }
    Ok(seq as usize)
}

/// One-shot helper: authenticates and sends `events` as sealed datagrams.
pub async fn send_events_secure(
    remote_addr: SocketAddr,
    events: &[InputEvent],
    opts: &SecureOpts,
) -> Result<usize> {
    let (endpoint, conn, mut sealer) = connect_secure(remote_addr, opts).await?;
    let (tx, rx) = tokio::sync::mpsc::channel(events.len().max(1));
    for &ev in events {
        tx.send(ev).await?;
    }
    drop(tx);
    let sent = forward_stream(&conn, &mut sealer, rx).await?;
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    conn.close(0u32.into(), b"done");
    endpoint.wait_idle().await;
    Ok(sent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kvm_input_hal::mock::MockHal;

    fn opts(dir: &Path, name: &str, peer_id: &str, pin: &str) -> SecureOpts {
        let path = dir.join(name).join("config.json");
        let mut cfg = OmniConfig::load_or_default(&path).unwrap();
        cfg.peer_id = peer_id.into();
        cfg.save(&path).unwrap();
        SecureOpts {
            pin: pin.into(),
            config_path: path,
        }
    }

    #[tokio::test]
    async fn e2e_secure_pin_tofu_and_sealed_datagrams() {
        let root = std::env::temp_dir().join(format!("omnikvm_secure_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let srv_opts = opts(&root, "srv", "desk-host", "135790");
        let cli_opts = opts(&root, "cli", "laptop", "135790");
        let bad_opts = opts(&root, "bad", "intruder", "000000");

        let hal = Arc::new(MockHal::new());
        let events = vec![
            InputEvent::MouseMove { dx: 4, dy: 2 },
            InputEvent::Key { keycode: 30, pressed: true },
            InputEvent::Key { keycode: 30, pressed: false },
        ];
        let (addr, handle) =
            run_secure_server("127.0.0.1:0".parse().unwrap(), hal.clone(), srv_opts.clone(), Some(events.len()))
                .await
                .unwrap();

        assert!(send_events_secure(addr, &events, &bad_opts).await.is_err());
        assert_eq!(send_events_secure(addr, &events, &cli_opts).await.unwrap(), 3);
        let stats = tokio::time::timeout(std::time::Duration::from_secs(5), handle)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(stats.injected, 3);
        assert_eq!(stats.rejected_sessions, 1);
        assert_eq!(hal.take_events(), events);

        // Both sides persisted the counterpart's pinned fingerprint.
        let srv_cfg = OmniConfig::load_or_default(&srv_opts.config_path).unwrap();
        assert!(srv_cfg.trusted_peers.contains_key("laptop"));
        let cli_cfg = OmniConfig::load_or_default(&cli_opts.config_path).unwrap();
        assert!(cli_cfg.trusted_peers.contains_key("desk-host"));

        // An impostor reusing the 'desk-host' name with a different identity is refused.
        let imp_opts = opts(&root, "impostor", "desk-host", "135790");
        let (imp_addr, _h) =
            run_secure_server("127.0.0.1:0".parse().unwrap(), Arc::new(MockHal::new()), imp_opts, Some(1))
                .await
                .unwrap();
        let err = send_events_secure(imp_addr, &events, &cli_opts).await;
        assert!(err.is_err(), "TOFU must reject changed server identity");
        let _ = std::fs::remove_dir_all(&root);
    }
}
