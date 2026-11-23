//! Real live KVM session manager for the OmniKVM desktop/web Control Center.
//! Starts/stops genuine authenticated QUIC Host (receiver) and Client (controller)
//! sessions using native OS input HAL (`Win32Injector`/`Win32Hook` on Windows,
//! `UinputDevice` on Linux).

use crate::codec::{event_to_packet, make_injector, packet_to_event};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use kvm_input_hal::InputEvent;
use kvm_omni_core::config::OmniConfig;
use kvm_omni_core::crypto::{generate_pairing_pin, TofuKeystore};
use kvm_omni_core::datagram_auth::{DatagramOpener, DatagramSealer};
use kvm_transport_quic::handshake::{client_handshake, server_handshake};
use kvm_transport_quic::identity::{
    load_or_generate_identity, make_server_endpoint_with_cert, recv_sealed_datagram,
    send_sealed_datagram,
};
use kvm_transport_quic::{make_client_endpoint, Connection};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;

#[derive(Clone, Debug, Serialize)]
pub struct SessionSnapshot {
    pub mode: String,
    pub bind_addr: Option<String>,
    pub peer_addr: Option<String>,
    pub pin: Option<String>,
    pub events_sent: u64,
    pub events_received: u64,
    pub last_error: Option<String>,
}

struct ActiveClient {
    conn: Connection,
    sealer: Mutex<DatagramSealer>,
    seq: AtomicU64,
}

struct Inner {
    mode: String,
    bind_addr: Option<String>,
    peer_addr: Option<String>,
    pin: Option<String>,
    last_error: Option<String>,
    task: Option<JoinHandle<()>>,
    client: Option<Arc<ActiveClient>>,
    #[cfg(windows)]
    win_hook: Option<kvm_input_hal::windows::Win32Hook>,
}

#[derive(Clone)]
pub struct SessionManager {
    config_path: PathBuf,
    inner: Arc<Mutex<Inner>>,
    sent: Arc<AtomicU64>,
    received: Arc<AtomicU64>,
}

impl SessionManager {
    pub fn new(config_path: Option<PathBuf>) -> Self {
        Self {
            config_path: config_path.unwrap_or_else(|| PathBuf::from("omnikvm.json")),
            inner: Arc::new(Mutex::new(Inner {
                mode: "idle".into(),
                bind_addr: None,
                peer_addr: None,
                pin: None,
                last_error: None,
                task: None,
                client: None,
                #[cfg(windows)]
                win_hook: None,
            })),
            sent: Arc::new(AtomicU64::new(0)),
            received: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn snapshot(&self) -> SessionSnapshot {
        let g = self.inner.lock().unwrap();
        SessionSnapshot {
            mode: g.mode.clone(),
            bind_addr: g.bind_addr.clone(),
            peer_addr: g.peer_addr.clone(),
            pin: g.pin.clone(),
            events_sent: self.sent.load(Ordering::Relaxed),
            events_received: self.received.load(Ordering::Relaxed),
            last_error: g.last_error.clone(),
        }
    }

    pub fn stop(&self) {
        let mut g = self.inner.lock().unwrap();
        if let Some(t) = g.task.take() {
            t.abort();
        }
        if let Some(c) = g.client.take() {
            c.conn.close(0u32.into(), b"stopped");
        }
        #[cfg(windows)]
        if let Some(mut h) = g.win_hook.take() {
            use kvm_input_hal::SystemInputHook;
            h.stop_capture();
        }
        g.mode = "idle".into();
        g.bind_addr = None;
        g.peer_addr = None;
        g.last_error = None;
    }

    fn identity_dir(&self) -> PathBuf {
        match self.config_path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.join("identity"),
            _ => PathBuf::from("identity"),
        }
    }
}

#[derive(Deserialize)]
struct HostReq {
    port: Option<u16>,
    pin: Option<String>,
    mock: Option<bool>,
}

async fn post_start_host(State(mgr): State<SessionManager>, Json(req): Json<HostReq>) -> impl IntoResponse {
    mgr.stop();
    let pin = req.pin.filter(|p| !p.trim().is_empty()).unwrap_or_else(generate_pairing_pin);
    let bind: SocketAddr = ([0, 0, 0, 0], req.port.unwrap_or(4433)).into();
    let mut cfg = OmniConfig::load_or_default(&mgr.config_path).unwrap_or_default();
    let mut ks = TofuKeystore::from_hex_map(&cfg.trusted_peers);
    let (ident, _) = match load_or_generate_identity(&mgr.identity_dir()) {
        Ok(v) => v,
        Err(e) => return Json(json!({ "ok": false, "error": e.to_string() })),
    };
    let (endpoint, fp) = match make_server_endpoint_with_cert(bind, ident) {
        Ok(v) => v,
        Err(e) => return Json(json!({ "ok": false, "error": e.to_string() })),
    };
    let actual = endpoint.local_addr().unwrap_or(bind);
    let injector = make_injector(req.mock.unwrap_or(false));
    let rx_count = mgr.received.clone();
    let cfg_path = mgr.config_path.clone();
    let pin_clone = pin.clone();
    let inner_ref = mgr.inner.clone();

    let task = tokio::spawn(async move {
        while let Some(incoming) = endpoint.accept().await {
            let Ok(conn) = incoming.await else { continue };
            match server_handshake(&conn, &cfg.peer_id, fp, &pin_clone, &mut ks).await {
                Ok((_peer, sk)) => {
                    cfg.trusted_peers = ks.to_hex_map();
                    let _ = cfg.save(&cfg_path);
                    let mut opener = DatagramOpener::new(&sk);
                    while let Ok(next) = recv_sealed_datagram(&conn, &mut opener).await {
                        if let Some(ev) = next.and_then(|p| packet_to_event(&p)) {
                            if injector.inject(ev).is_ok() {
                                rx_count.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                }
                Err(e) => {
                    inner_ref.lock().unwrap().last_error = Some(e.to_string());
                    conn.close(1u32.into(), b"auth failed");
                }
            }
        }
    });

    {
        let mut g = mgr.inner.lock().unwrap();
        g.mode = "host".into();
        g.bind_addr = Some(actual.to_string());
        g.pin = Some(pin);
        g.last_error = None;
        g.task = Some(task);
    }
    Json(json!({ "ok": true, "session": mgr.snapshot() }))
}

#[derive(Deserialize)]
struct ClientReq {
    peer: String,
    pin: String,
    capture: Option<bool>,
}

async fn post_start_client(State(mgr): State<SessionManager>, Json(req): Json<ClientReq>) -> impl IntoResponse {
    mgr.stop();
    let addr: SocketAddr = match req.peer.trim().parse() {
        Ok(a) => a,
        Err(_) => return Json(json!({ "ok": false, "error": "Adresse invalide (ex: 192.168.1.20:4433)" })),
    };
    let mut cfg = OmniConfig::load_or_default(&mgr.config_path).unwrap_or_default();
    let mut ks = TofuKeystore::from_hex_map(&cfg.trusted_peers);
    let (ident, _) = match load_or_generate_identity(&mgr.identity_dir()) {
        Ok(v) => v,
        Err(e) => return Json(json!({ "ok": false, "error": e.to_string() })),
    };
    let endpoint = match make_client_endpoint("0.0.0.0:0".parse().unwrap()) {
        Ok(e) => e,
        Err(e) => return Json(json!({ "ok": false, "error": e.to_string() })),
    };
    let conn = match endpoint.connect(addr, "localhost") {
        Ok(c) => match tokio::time::timeout(std::time::Duration::from_secs(5), c).await {
            Ok(Ok(conn)) => conn,
            Ok(Err(e)) => return Json(json!({ "ok": false, "error": format!("Connexion QUIC refusée: {e}") })),
            Err(_) => return Json(json!({ "ok": false, "error": "Délai dépassé (vérifiez IP et pare-feu UDP 4433)" })),
        },
        Err(e) => return Json(json!({ "ok": false, "error": e.to_string() })),
    };
    let sk = match client_handshake(&conn, &cfg.peer_id, ident.fingerprint, req.pin.trim(), &mut ks).await {
        Ok(k) => k,
        Err(e) => {
            conn.close(1u32.into(), b"auth failed");
            return Json(json!({ "ok": false, "error": format!("Authentification PIN/TOFU échouée: {e}") }));
        }
    };
    cfg.trusted_peers = ks.to_hex_map();
    let _ = cfg.save(&mgr.config_path);

    let active = Arc::new(ActiveClient {
        conn,
        sealer: Mutex::new(DatagramSealer::new(&sk)),
        seq: AtomicU64::new(0),
    });

    #[cfg(windows)]
    let win_hook = if req.capture.unwrap_or(true) {
        use kvm_input_hal::SystemInputHook;
        let mut h = kvm_input_hal::windows::Win32Hook::default();
        let active_cb = active.clone();
        let sent_cb = mgr.sent.clone();
        let mgr_cb = mgr.clone();
        let hk = Mutex::new((kvm_omni_core::hotkeys::HotkeyManager::with_defaults(), false));
        let _ = h.start_capture(move |ev| {
            let mut g = hk.lock().unwrap();
            let (hkm, locked) = &mut *g;
            if let InputEvent::Key { keycode, pressed } = ev {
                match hkm.on_key(keycode, pressed) {
                    Some(kvm_omni_core::hotkeys::HotkeyAction::ReleaseCapture) => {
                        let m = mgr_cb.clone();
                        std::thread::spawn(move || m.stop());
                        return;
                    }
                    Some(kvm_omni_core::hotkeys::HotkeyAction::ToggleCursorLock) => {
                        *locked = !*locked;
                        return;
                    }
                    Some(_) => return,
                    None => {}
                }
            }
            if *locked {
                return;
            }
            let seq = active_cb.seq.fetch_add(1, Ordering::Relaxed) as u32;
            if let Some(pkt) = event_to_packet(seq, ev) {
                if let Ok(mut s) = active_cb.sealer.lock() {
                    if send_sealed_datagram(&active_cb.conn, &mut s, &pkt).is_ok() {
                        sent_cb.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        });
        Some(h)
    } else {
        None
    };
    #[cfg(not(windows))]
    let _ = req.capture;

    {
        let mut g = mgr.inner.lock().unwrap();
        g.mode = "client".into();
        g.peer_addr = Some(addr.to_string());
        g.pin = Some(req.pin);
        g.last_error = None;
        g.client = Some(active);
        #[cfg(windows)]
        {
            g.win_hook = win_hook;
        }
    }
    Json(json!({ "ok": true, "session": mgr.snapshot() }))
}

#[derive(Deserialize)]
struct MoveReq {
    dx: Option<i32>,
    dy: Option<i32>,
}

async fn post_test_move(State(mgr): State<SessionManager>, Json(req): Json<MoveReq>) -> impl IntoResponse {
    let client = mgr.inner.lock().unwrap().client.clone();
    let Some(c) = client else {
        return Json(json!({ "ok": false, "error": "Aucune session client active" }));
    };
    let seq = c.seq.fetch_add(1, Ordering::Relaxed) as u32;
    let ev = InputEvent::MouseMove {
        dx: req.dx.unwrap_or(40),
        dy: req.dy.unwrap_or(0),
    };
    let Some(pkt) = event_to_packet(seq, ev) else {
        return Json(json!({ "ok": false, "error": "packet encode error" }));
    };
    let res = {
        let mut s = c.sealer.lock().unwrap();
        send_sealed_datagram(&c.conn, &mut s, &pkt)
    };
    match res {
        Ok(()) => {
            mgr.sent.fetch_add(1, Ordering::Relaxed);
            Json(json!({ "ok": true, "session": mgr.snapshot() }))
        }
        Err(e) => Json(json!({ "ok": false, "error": e.to_string() })),
    }
}

async fn get_session(State(mgr): State<SessionManager>) -> impl IntoResponse {
    Json(mgr.snapshot())
}

async fn post_stop(State(mgr): State<SessionManager>) -> impl IntoResponse {
    mgr.stop();
    Json(json!({ "ok": true, "session": mgr.snapshot() }))
}

pub fn router(mgr: SessionManager) -> Router {
    Router::new()
        .route("/api/session", get(get_session))
        .route("/api/session/start-host", post(post_start_host))
        .route("/api/session/start-client", post(post_start_client))
        .route("/api/session/test-move", post(post_test_move))
        .route("/api/session/stop", post(post_stop))
        .with_state(mgr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn live_ui_session_host_and_client_e2e() {
        let dir = std::env::temp_dir().join(format!("omnikvm_ui_sess_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let host_mgr = SessionManager::new(Some(dir.join("host.json")));
        let cli_mgr = SessionManager::new(Some(dir.join("cli.json")));

        let _ = post_start_host(
            State(host_mgr.clone()),
            Json(HostReq { port: Some(0), pin: Some("654321".into()), mock: Some(true) }),
        ).await;
        let h_snap = host_mgr.snapshot();
        assert_eq!(h_snap.mode, "host");
        let port = h_snap.bind_addr.unwrap().split(':').next_back().unwrap().to_string();

        let _ = post_start_client(
            State(cli_mgr.clone()),
            Json(ClientReq { peer: format!("127.0.0.1:{port}"), pin: "654321".into(), capture: Some(false) }),
        ).await;
        assert_eq!(cli_mgr.snapshot().mode, "client");

        let _ = post_test_move(State(cli_mgr.clone()), Json(MoveReq { dx: Some(25), dy: Some(-5) })).await;
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        assert_eq!(cli_mgr.snapshot().events_sent, 1);
        assert_eq!(host_mgr.snapshot().events_received, 1);

        cli_mgr.stop();
        host_mgr.stop();
        assert_eq!(host_mgr.snapshot().mode, "idle");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
