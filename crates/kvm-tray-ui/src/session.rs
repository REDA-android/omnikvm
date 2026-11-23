//! Real live KVM session manager: authenticated QUIC Host & Client, seamless
//! Windows screen-edge switching (`Win32Hook`), modifier key release safety,
//! and bidirectional encrypted clipboard synchronization (`clip_sync`).

use crate::clip_sync::spawn_bidirectional_clipboard;
use crate::codec::{event_to_packet, make_injector, packet_to_event};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use kvm_discovery_mdns::detect_local_ipv4;
use kvm_input_hal::edge_kvm::ScreenEdge;
use kvm_input_hal::windows::Win32HookControl;
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
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;

#[derive(Clone, Debug, Serialize)]
pub struct SessionSnapshot {
    pub mode: String,
    pub bind_addr: Option<String>,
    pub lan_addr: Option<String>,
    pub peer_addr: Option<String>,
    pub pin: Option<String>,
    pub edge: String,
    pub remote_active: bool,
    pub events_sent: u64,
    pub events_received: u64,
    pub clips_synced: u64,
    pub last_error: Option<String>,
}

pub(crate) struct ActiveClient {
    pub(crate) conn: Connection,
    pub(crate) session_key: [u8; 32],
    pub(crate) sealer: Mutex<DatagramSealer>,
    pub(crate) seq: AtomicU64,
}

pub(crate) struct Inner {
    pub(crate) mode: String,
    pub(crate) bind_addr: Option<String>,
    pub(crate) lan_addr: Option<String>,
    pub(crate) peer_addr: Option<String>,
    pub(crate) pin: Option<String>,
    pub(crate) edge: String,
    pub(crate) last_error: Option<String>,
    pub(crate) task: Option<JoinHandle<()>>,
    pub(crate) client: Option<Arc<ActiveClient>>,
    pub(crate) peer_conn: Option<(Connection, [u8; 32])>,
    pub(crate) hook_ctrl: Win32HookControl,
    #[cfg(windows)]
    pub(crate) win_hook: Option<kvm_input_hal::windows::Win32Hook>,
}

#[derive(Clone)]
pub struct SessionManager {
    pub(crate) config_path: PathBuf,
    pub(crate) local_ip: String,
    pub(crate) cursor_lock: Arc<AtomicBool>,
    pub(crate) quic_port: Arc<AtomicU16>,
    pub(crate) inner: Arc<Mutex<Inner>>,
    pub(crate) sent: Arc<AtomicU64>,
    pub(crate) received: Arc<AtomicU64>,
    pub(crate) clips: Arc<AtomicU64>,
    pub(crate) last_clip: Arc<Mutex<Option<String>>>,
}

impl SessionManager {
    pub fn new(
        config_path: Option<PathBuf>,
        cursor_lock: Arc<AtomicBool>,
        quic_port: Arc<AtomicU16>,
    ) -> Self {
        Self {
            config_path: config_path.unwrap_or_else(|| PathBuf::from("omnikvm.json")),
            local_ip: detect_local_ipv4(),
            cursor_lock,
            quic_port,
            inner: Arc::new(Mutex::new(Inner {
                mode: "idle".into(),
                bind_addr: None,
                lan_addr: None,
                peer_addr: None,
                pin: None,
                edge: "right".into(),
                last_error: None,
                task: None,
                client: None,
                peer_conn: None,
                hook_ctrl: Win32HookControl::default(),
                #[cfg(windows)]
                win_hook: None,
            })),
            sent: Arc::new(AtomicU64::new(0)),
            received: Arc::new(AtomicU64::new(0)),
            clips: Arc::new(AtomicU64::new(0)),
            last_clip: Arc::new(Mutex::new(None)),
        }
    }

    pub fn snapshot(&self) -> SessionSnapshot {
        let g = self.inner.lock().unwrap();
        g.hook_ctrl.set_cursor_locked(self.cursor_lock.load(Ordering::Relaxed));
        SessionSnapshot {
            mode: g.mode.clone(),
            bind_addr: g.bind_addr.clone(),
            lan_addr: g.lan_addr.clone(),
            peer_addr: g.peer_addr.clone(),
            pin: g.pin.clone(),
            edge: g.edge.clone(),
            remote_active: g.hook_ctrl.is_remote_active(),
            events_sent: self.sent.load(Ordering::Relaxed),
            events_received: self.received.load(Ordering::Relaxed),
            clips_synced: self.clips.load(Ordering::Relaxed),
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
        if let Some((conn, _)) = g.peer_conn.take() {
            conn.close(0u32.into(), b"stopped");
        }
        #[cfg(windows)]
        if let Some(mut h) = g.win_hook.take() {
            use kvm_input_hal::SystemInputHook;
            h.stop_capture();
        }
        g.hook_ctrl.set_remote_active(false);
        g.mode = "idle".into();
        g.bind_addr = None;
        g.lan_addr = None;
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

pub(crate) fn release_modifiers(c: &ActiveClient) {
    if let Ok(mut s) = c.sealer.lock() {
        for keycode in [29u32, 97, 56, 100, 42, 54] {
            let seq = c.seq.fetch_add(1, Ordering::Relaxed) as u32;
            if let Some(pkt) = event_to_packet(seq, InputEvent::Key { keycode, pressed: false }) {
                let _ = send_sealed_datagram(&c.conn, &mut s, &pkt);
            }
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct HostReq {
    pub(crate) port: Option<u16>,
    pub(crate) pin: Option<String>,
    pub(crate) mock: Option<bool>,
}

pub(crate) async fn post_start_host(State(mgr): State<SessionManager>, Json(req): Json<HostReq>) -> impl IntoResponse {
    mgr.stop();
    let pin = req.pin.filter(|p| !p.trim().is_empty()).unwrap_or_else(generate_pairing_pin);
    let bind: SocketAddr = ([0, 0, 0, 0], req.port.unwrap_or(4433)).into();
    let mut cfg = OmniConfig::load_or_default(&mgr.config_path).unwrap_or_default();
    let mut ks = TofuKeystore::from_hex_map(&cfg.trusted_peers);
    let Ok((ident, _)) = load_or_generate_identity(&mgr.identity_dir()) else {
        return Json(json!({ "ok": false, "error": "Échec génération identité TLS" }));
    };
    let (endpoint, fp) = match make_server_endpoint_with_cert(bind, ident) {
        Ok(v) => v,
        Err(e) => return Json(json!({ "ok": false, "error": e.to_string() })),
    };
    let actual = endpoint.local_addr().unwrap_or(bind);
    mgr.quic_port.store(actual.port(), Ordering::Relaxed);
    let lan_addr = format!("{}:{}", mgr.local_ip, actual.port());
    let injector = make_injector(req.mock.unwrap_or(false));
    let rx_count = mgr.received.clone();
    let clips = mgr.clips.clone();
    let last_clip = mgr.last_clip.clone();
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
                    inner_ref.lock().unwrap().peer_conn = Some((conn.clone(), sk));
                    spawn_bidirectional_clipboard(conn.clone(), sk, clips.clone(), last_clip.clone());
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
        g.lan_addr = Some(lan_addr);
        g.pin = Some(pin);
        g.last_error = None;
        g.task = Some(task);
    }
    Json(json!({ "ok": true, "session": mgr.snapshot() }))
}

#[derive(Deserialize)]
pub(crate) struct ClientReq {
    pub(crate) peer: String,
    pub(crate) pin: String,
    pub(crate) edge: Option<String>,
    pub(crate) capture: Option<bool>,
}

pub(crate) async fn post_start_client(State(mgr): State<SessionManager>, Json(req): Json<ClientReq>) -> impl IntoResponse {
    mgr.stop();
    let addr: SocketAddr = match req.peer.trim().parse() {
        Ok(a) => a,
        Err(_) => return Json(json!({ "ok": false, "error": "Adresse invalide (ex: 192.168.1.20:4433)" })),
    };
    let edge_str = req.edge.unwrap_or_else(|| "right".into());
    let edge = ScreenEdge::parse(&edge_str);
    let mut cfg = OmniConfig::load_or_default(&mgr.config_path).unwrap_or_default();
    let mut ks = TofuKeystore::from_hex_map(&cfg.trusted_peers);
    let Ok((ident, _)) = load_or_generate_identity(&mgr.identity_dir()) else {
        return Json(json!({ "ok": false, "error": "Échec identité TLS" }));
    };
    let Ok(endpoint) = make_client_endpoint("0.0.0.0:0".parse().unwrap()) else {
        return Json(json!({ "ok": false, "error": "Échec endpoint QUIC" }));
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
            return Json(json!({ "ok": false, "error": format!("Code PIN refusé: {e}") }));
        }
    };
    cfg.trusted_peers = ks.to_hex_map();
    let _ = cfg.save(&mgr.config_path);
    spawn_bidirectional_clipboard(conn.clone(), sk, mgr.clips.clone(), mgr.last_clip.clone());

    let active = Arc::new(ActiveClient {
        conn: conn.clone(),
        session_key: sk,
        sealer: Mutex::new(DatagramSealer::new(&sk)),
        seq: AtomicU64::new(0),
    });

    #[cfg(windows)]
    let (win_hook, hook_ctrl) = if req.capture.unwrap_or(true) {
        use kvm_input_hal::SystemInputHook;
        let mut h = kvm_input_hal::windows::Win32Hook::with_edge(edge);
        let ctrl = h.control();
        let ctrl_cb = ctrl.clone();
        let lock_cb = mgr.cursor_lock.clone();
        let active_cb = active.clone();
        let sent_cb = mgr.sent.clone();
        let mgr_cb = mgr.clone();
        let hk = Mutex::new(kvm_omni_core::hotkeys::HotkeyManager::with_defaults());
        let _ = h.start_capture(move |ev| {
            ctrl_cb.set_cursor_locked(lock_cb.load(Ordering::Relaxed));
            if let InputEvent::Key { keycode, pressed } = ev {
                if let Some(act) = hk.lock().unwrap().on_key(keycode, pressed) {
                    match act {
                        kvm_omni_core::hotkeys::HotkeyAction::ReleaseCapture => {
                            release_modifiers(&active_cb);
                            if edge == ScreenEdge::Immediate {
                                let m = mgr_cb.clone();
                                std::thread::spawn(move || m.stop());
                            } else {
                                ctrl_cb.set_remote_active(false);
                            }
                            return;
                        }
                        kvm_omni_core::hotkeys::HotkeyAction::ToggleCursorLock => {
                            let new = !lock_cb.load(Ordering::Relaxed);
                            lock_cb.store(new, Ordering::SeqCst);
                            ctrl_cb.set_cursor_locked(new);
                            return;
                        }
                        _ => return,
                    }
                }
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
        (Some(h), ctrl)
    } else {
        (None, Win32HookControl::default())
    };
    #[cfg(not(windows))]
    let hook_ctrl = {
        let _ = (req.capture, edge);
        let c = Win32HookControl::default();
        c.set_remote_active(edge == ScreenEdge::Immediate);
        c
    };

    {
        let mut g = mgr.inner.lock().unwrap();
        g.mode = "client".into();
        g.peer_addr = Some(addr.to_string());
        g.pin = Some(req.pin);
        g.edge = edge_str;
        g.last_error = None;
        g.client = Some(active);
        g.peer_conn = Some((conn, sk));
        g.hook_ctrl = hook_ctrl;
        #[cfg(windows)]
        {
            g.win_hook = win_hook;
        }
    }
    Json(json!({ "ok": true, "session": mgr.snapshot() }))
}
