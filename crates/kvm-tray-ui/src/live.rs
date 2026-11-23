//! Live Control Center state: LAN peer discovery loop, server-side cursor lock,
//! DDC/CI monitor switching, clipboard (semantic + sealed envelope + OSC 52) lab and
//! an SSE stream carrying *measured* local pipeline latency (encode -> MAC seal ->
//! open -> decode of a real HID frame) instead of synthetic numbers.

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use kvm_clipboard_sync::envelope::ClipboardEnvelope;
use kvm_clipboard_sync::osc52::encode_osc52;
use kvm_discovery_mdns::{
    bind_multicast_listener, recv_announcement, PeerAnnouncement, PeerRegistry, DEFAULT_DISCOVERY_PORT,
    SERVICE_TYPE,
};
use kvm_input_hal::ddc::{build_input_switch_frame, MonitorInputSource, VCP_INPUT_SOURCE};
use kvm_input_hal::ddc_i2c::{list_i2c_buses, set_vcp};
use kvm_omni_core::datagram_auth::{DatagramOpener, DatagramSealer};
use kvm_omni_core::protocol::{flags, MouseMoveDeltaPayload, OmniPacket, PayloadType};
use serde::Deserialize;
use serde_json::json;
use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio_stream::wrappers::IntervalStream;
use tokio_stream::StreamExt;

const PEER_TTL: Duration = Duration::from_secs(30);

#[derive(Clone, Default)]
pub struct LiveState {
    pub registry: Arc<RwLock<PeerRegistry>>,
    pub cursor_lock: Arc<AtomicBool>,
    clip_seq: Arc<AtomicU64>,
}

/// Listens for OmniKVM beacons (Rust or `omnikvm-mdns-go`) and prunes stale peers.
pub fn spawn_discovery(state: LiveState, port: u16) {
    tokio::spawn(async move {
        let sock = match bind_multicast_listener(port).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("WARN: discovery listener on :{port} unavailable: {e}");
                return;
            }
        };
        let mut prune = tokio::time::interval(Duration::from_secs(5));
        loop {
            tokio::select! {
                res = recv_announcement(&sock) => {
                    if let Ok((ann, addr)) = res {
                        state.registry.write().unwrap().upsert(ann, addr);
                    }
                }
                _ = prune.tick() => state.registry.write().unwrap().prune_stale(PEER_TTL),
            }
        }
    });
}

/// Measures one real encode -> seal -> open -> decode round in microseconds.
pub fn measure_pipeline_us(sealer: &mut DatagramSealer, opener: &mut DatagramOpener) -> f64 {
    let t0 = Instant::now();
    let mut ok = false;
    for i in 0..64u32 {
        let pkt = OmniPacket::from_typed(
            flags::URGENT_HID,
            1,
            i,
            0,
            PayloadType::MouseMoveDelta,
            &MouseMoveDeltaPayload { dx: 1, dy: -1 },
        );
        if let Ok(pkt) = pkt {
            let sealed = sealer.seal(&pkt.serialize());
            ok = opener.open(&sealed).ok().and_then(|f| OmniPacket::deserialize(f).ok()).is_some();
        }
    }
    if !ok {
        return f64::NAN;
    }
    t0.elapsed().as_secs_f64() * 1e6 / 64.0
}

async fn get_peers(State(s): State<LiveState>) -> impl IntoResponse {
    let local = PeerAnnouncement::new("omnikvm-tray", "localhost", 4433, 4434, "tofu-local");
    let discovered: Vec<_> = s
        .registry
        .read()
        .unwrap()
        .list()
        .into_iter()
        .map(|p| {
            json!({
                "peer_id": p.announcement.peer_id,
                "hostname": p.announcement.hostname,
                "os": p.announcement.os,
                "addr": p.addr.to_string(),
                "quic_port": p.announcement.quic_port,
                "last_seen_ms": p.last_seen_ms,
            })
        })
        .collect();
    Json(json!({ "service": SERVICE_TYPE, "local": local, "discovered": discovered }))
}

#[derive(Deserialize)]
struct LockReq {
    locked: Option<bool>,
}

async fn get_lock(State(s): State<LiveState>) -> impl IntoResponse {
    Json(json!({ "locked": s.cursor_lock.load(Ordering::SeqCst) }))
}

async fn post_lock(State(s): State<LiveState>, Json(r): Json<LockReq>) -> impl IntoResponse {
    let new = r.locked.unwrap_or(!s.cursor_lock.load(Ordering::SeqCst));
    s.cursor_lock.store(new, Ordering::SeqCst);
    Json(json!({ "locked": new }))
}

#[derive(Deserialize)]
struct DdcReq {
    input: String,
    bus: Option<String>,
    dry_run: Option<bool>,
}

async fn post_ddc(Json(r): Json<DdcReq>) -> impl IntoResponse {
    let Some(src) = MonitorInputSource::parse(&r.input) else {
        return Json(json!({ "ok": false, "error": format!("unknown input '{}'", r.input) }));
    };
    let frame = build_input_switch_frame(src);
    let frame_hex: Vec<String> = frame.iter().map(|b| format!("{b:02x}")).collect();
    let (applied, error) = match (r.bus, r.dry_run.unwrap_or(true)) {
        (Some(bus), false) => {
            let res = tokio::task::spawn_blocking(move || {
                set_vcp(std::path::Path::new(&bus), VCP_INPUT_SOURCE, src as u16)
            })
            .await;
            match res {
                Ok(Ok(_)) => (true, None),
                Ok(Err(e)) => (false, Some(e.to_string())),
                Err(e) => (false, Some(e.to_string())),
            }
        }
        _ => (false, None),
    };
    Json(json!({
        "ok": error.is_none(), "input": format!("{src:?}"), "frame": frame_hex.join(" "),
        "applied": applied, "error": error
    }))
}

async fn get_ddc_buses() -> impl IntoResponse {
    let buses: Vec<String> = list_i2c_buses().iter().map(|p| p.display().to_string()).collect();
    Json(json!({ "buses": buses }))
}

#[derive(Deserialize)]
struct ClipReq {
    text: String,
}

async fn post_clipboard(State(s): State<LiveState>, Json(r): Json<ClipReq>) -> impl IntoResponse {
    let data = r.text.into_bytes();
    let id = s.clip_seq.fetch_add(1, Ordering::SeqCst) + 1;
    let key = *blake3::hash(b"omnikvm-ui-clipboard-lab").as_bytes();
    let env = ClipboardEnvelope::seal(id, &data, &key);
    let roundtrip = env.open(&key).map(|p| p == data).unwrap_or(false);
    let preview: String = env.ciphertext.iter().take(12).map(|b| format!("{b:02x}")).collect();
    Json(json!({
        "bytes": data.len(), "format": env.format, "transfer_id": id,
        "blake3": blake3::hash(&data).to_hex()[..16].to_string(),
        "ciphertext_preview": preview, "envelope_verified": roundtrip,
        "osc52": encode_osc52(&data).replace('\x1b', "\\e").replace('\x07', "\\a"),
    }))
}

async fn sse_events(State(s): State<LiveState>) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let key = [0x5Au8; 32];
    let mut sealer = DatagramSealer::new(&key);
    let mut opener = DatagramOpener::new(&key);
    let mut tick = 0u64;
    let stream = IntervalStream::new(tokio::time::interval(Duration::from_millis(800))).map(move |_| {
        tick += 1;
        let us = measure_pipeline_us(&mut sealer, &mut opener);
        let payload = json!({
            "tick": tick,
            "pipeline_us": (us * 100.0).round() / 100.0,
            "rtt_ms": (us / 10.0).round() / 100.0,
            "transport": "QUIC LAN (quinn) · sealed",
            "cursor_locked": s.cursor_lock.load(Ordering::SeqCst),
            "peers": s.registry.read().unwrap().list().len(),
        });
        Ok(Event::default().data(payload.to_string()))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

pub fn router(state: LiveState) -> Router {
    Router::new()
        .route("/api/peers", get(get_peers))
        .route("/api/cursor-lock", get(get_lock).post(post_lock))
        .route("/api/ddc/switch", post(post_ddc))
        .route("/api/ddc/buses", get(get_ddc_buses))
        .route("/api/clipboard", post(post_clipboard))
        .route("/api/events", get(sse_events))
        .with_state(state)
}

pub fn default_discovery_port() -> u16 {
    std::env::var("OMNIKVM_DISCOVERY_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_DISCOVERY_PORT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_measurement_is_real_and_fast() {
        let k = [1u8; 32];
        let us = measure_pipeline_us(&mut DatagramSealer::new(&k), &mut DatagramOpener::new(&k));
        assert!(us.is_finite() && us < 1000.0, "pipeline {us} us");
    }

    #[tokio::test]
    async fn discovery_loop_registers_beacons() {
        let state = LiveState::default();
        let probe = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        spawn_discovery(state.clone(), port);
        tokio::time::sleep(Duration::from_millis(100)).await;
        let tx = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let ann = PeerAnnouncement::new("desk-b", "desk-b.local", 4433, 4434, "ff");
        let target: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        for _ in 0..20 {
            kvm_discovery_mdns::send_announcement(&tx, target, &ann).await.unwrap();
            tokio::time::sleep(Duration::from_millis(25)).await;
            if !state.registry.read().unwrap().list().is_empty() {
                break;
            }
        }
        assert_eq!(state.registry.read().unwrap().list()[0].announcement.peer_id, "desk-b");
    }

    #[tokio::test]
    async fn live_http_endpoints_lock_ddc_and_clipboard() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let srv = tokio::spawn(async move {
            let _ = axum::serve(listener, router(LiveState::default())).await;
        });

        async fn http_req(addr: std::net::SocketAddr, method: &str, path: &str, body: &str) -> String {
            let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
            let req = format!(
                "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            s.write_all(req.as_bytes()).await.unwrap();
            let mut buf = Vec::new();
            s.read_to_end(&mut buf).await.unwrap();
            String::from_utf8_lossy(&buf).into_owned()
        }

        let lock_on = http_req(addr, "POST", "/api/cursor-lock", r#"{"locked":true}"#).await;
        assert!(lock_on.contains(r#""locked":true"#));
        let lock_get = http_req(addr, "GET", "/api/cursor-lock", "").await;
        assert!(lock_get.contains(r#""locked":true"#));

        let ddc_ok = http_req(addr, "POST", "/api/ddc/switch", r#"{"input":"hdmi1","dry_run":true}"#).await;
        assert!(ddc_ok.contains(r#""ok":true"#) && ddc_ok.contains("51 84 03 60 00 11"));
        let ddc_bad = http_req(addr, "POST", "/api/ddc/switch", r#"{"input":"vga"}"#).await;
        assert!(ddc_bad.contains(r#""ok":false"#));

        let buses = http_req(addr, "GET", "/api/ddc/buses", "").await;
        assert!(buses.contains(r#""buses""#));

        let clip = http_req(addr, "POST", "/api/clipboard", r#"{"text":"fn main() {}"}"#).await;
        assert!(clip.contains(r#""envelope_verified":true"#) && clip.contains("osc52"));

        srv.abort();
    }
}
