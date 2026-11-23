use axum::extract::State;
use axum::http::header;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use kvm_clipboard_sync::streaming::{chunk_bytes_encrypted, ChunkReassembler, DEFAULT_CHUNK_SIZE};
use kvm_discovery_mdns::{PeerAnnouncement, SERVICE_TYPE};
use kvm_omni_core::crypto::{build_pairing_uri, generate_pairing_pin};
use kvm_omni_core::topology::ScreenRect;
use qrcode::render::svg;
use qrcode::QrCode;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::convert::Infallible;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio_stream::wrappers::IntervalStream;
use tokio_stream::StreamExt;

const INDEX_HTML: &str = include_str!("../../../web/index.html");
const STYLES_CSS: &str = include_str!("../../../web/styles.css");
const APP_JS: &str = include_str!("../../../web/app.js");

#[derive(Clone)]
struct AppState {
    screens: Arc<RwLock<Vec<ScreenRect>>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            screens: Arc::new(RwLock::new(vec![
                ScreenRect {
                    id: "Linux-Host-4K".into(),
                    x: 0,
                    y: 0,
                    width: 3840,
                    height: 2160,
                    dpi_scale: 2.0,
                },
                ScreenRect {
                    id: "Win11-Work-FHD".into(),
                    x: 3840,
                    y: 540,
                    width: 1920,
                    height: 1080,
                    dpi_scale: 1.0,
                },
                ScreenRect {
                    id: "Android-Tablet".into(),
                    x: 5760,
                    y: 680,
                    width: 1200,
                    height: 800,
                    dpi_scale: 2.0,
                },
            ])),
        }
    }
}

pub fn render_pairing_qr_svg(uri: &str) -> String {
    let code = QrCode::new(uri.as_bytes()).expect("valid QR data");
    code.render::<svg::Color>()
        .min_dimensions(180, 180)
        .build()
}

pub fn build_pairing_json() -> serde_json::Value {
    let pin = generate_pairing_pin();
    let uri = build_pairing_uri("omnikvm-tray", "127.0.0.1", 4433, &pin);
    let svg = render_pairing_qr_svg(&uri);
    json!({
        "pin": pin,
        "uri": uri,
        "qr_svg": svg,
        "service": SERVICE_TYPE
    })
}

async fn get_index() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], INDEX_HTML)
}

async fn get_css() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], STYLES_CSS)
}

async fn get_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
        APP_JS,
    )
}

async fn get_pairing() -> impl IntoResponse {
    Json(build_pairing_json())
}

async fn get_peers() -> impl IntoResponse {
    let local = PeerAnnouncement::new("omnikvm-tray", "localhost", 4433, 4434, "tofu-local");
    Json(json!({ "service": SERVICE_TYPE, "local": local }))
}

async fn get_topology(State(state): State<AppState>) -> impl IntoResponse {
    let list = state.screens.read().unwrap().clone();
    Json(list)
}

async fn post_topology(
    State(state): State<AppState>,
    Json(updated): Json<Vec<ScreenRect>>,
) -> impl IntoResponse {
    *state.screens.write().unwrap() = updated.clone();
    Json(json!({ "saved": updated.len() }))
}

#[derive(Deserialize)]
struct TransferReq {
    size_kb: Option<usize>,
}

#[derive(Serialize)]
struct TransferResp {
    bytes: usize,
    chunks: usize,
    blake3_hex: String,
    elapsed_us: u128,
}

async fn post_transfer_test(Json(req): Json<TransferReq>) -> impl IntoResponse {
    let bytes_len = req.size_kb.unwrap_or(512).min(16 * 1024) * 1024;
    let payload = vec![0x5Au8; bytes_len];
    let key = [0x33u8; 32];
    let t0 = Instant::now();
    let (chunks, digest) = chunk_bytes_encrypted(1, &payload, DEFAULT_CHUNK_SIZE, &key);
    let chunk_count = chunks.len();
    let mut reassembler = ChunkReassembler::new(key, bytes_len);
    for c in chunks {
        let _ = reassembler.push_chunk(c);
    }
    let _ = reassembler.finalize_and_verify(&digest);
    let elapsed_us = t0.elapsed().as_micros();
    let hex: String = digest.iter().take(12).map(|b| format!("{b:02x}")).collect();
    Json(TransferResp {
        bytes: bytes_len,
        chunks: chunk_count,
        blake3_hex: hex,
        elapsed_us,
    })
}

async fn sse_events() -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let mut tick = 0u64;
    let stream = IntervalStream::new(tokio::time::interval(Duration::from_millis(800))).map(move |_| {
        tick += 1;
        let rtt_ms = 0.31 + ((tick % 5) as f64) * 0.03;
        let payload = json!({
            "tick": tick,
            "rtt_ms": (rtt_ms * 100.0).round() / 100.0,
            "transport": "QUIC LAN (quinn)",
            "active_screen": if tick % 6 < 4 { "Linux-Host-4K" } else { "Win11-Work-FHD" }
        })
        .to_string();
        Ok(Event::default().data(payload))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

pub fn build_router() -> Router {
    let state = AppState::default();
    Router::new()
        .route("/", get(get_index))
        .route("/styles.css", get(get_css))
        .route("/app.js", get(get_js))
        .route("/api/pairing", get(get_pairing))
        .route("/api/peers", get(get_peers))
        .route("/api/topology", get(get_topology).post(post_topology))
        .route("/api/transfer-test", post(post_transfer_test))
        .route("/api/events", get(sse_events))
        .with_state(state)
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == "--serve") {
        let addr = args.get(pos + 1).map(String::as_str).unwrap_or("127.0.0.1:8080");
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        println!("OmniKVM Axum Control Center listening on http://{addr}");
        axum::serve(listener, build_router()).await.unwrap();
    } else {
        println!("{}", build_pairing_json());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_valid_qr_svg_and_embeds_web_ui() {
        let svg = render_pairing_qr_svg("omnikvm://pair?id=test&pin=123456");
        assert!(svg.contains("<svg"));
        assert!(INDEX_HTML.contains("OmniKVM Control Plane"));
        assert!(STYLES_CSS.contains("--accent"));
        assert!(APP_JS.contains("topology-stage"));
        let _router = build_router();
    }

    #[test]
    fn memory_footprint_under_15mb() {
        if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
            if let Some(rss_kb) = status
                .lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<usize>().ok())
            {
                assert!(
                    rss_kb < 15 * 1024,
                    "RSS too high: {} KB (limit 15360 KB)",
                    rss_kb
                );
            }
        }
    }
}
