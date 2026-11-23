mod live;

use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use kvm_clipboard_sync::streaming::{chunk_bytes_encrypted, ChunkReassembler, DEFAULT_CHUNK_SIZE};
use kvm_discovery_mdns::SERVICE_TYPE;
use kvm_omni_core::config::OmniConfig;
use kvm_omni_core::crypto::{build_pairing_uri, generate_pairing_pin};
use kvm_omni_core::topology::ScreenRect;
use qrcode::render::svg;
use qrcode::QrCode;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Instant;

const INDEX_HTML: &str = include_str!("../../../web/index.html");
const STYLES_CSS: &str = include_str!("../../../web/styles.css");
const APP_JS: &str = include_str!("../../../web/app.js");

#[derive(Clone)]
struct AppState {
    config: Arc<RwLock<OmniConfig>>,
    config_path: Option<PathBuf>,
}

impl AppState {
    fn from_path(path: Option<PathBuf>) -> Self {
        let cfg = path
            .as_deref()
            .and_then(|p| OmniConfig::load_or_default(p).ok())
            .unwrap_or_default();
        Self {
            config: Arc::new(RwLock::new(cfg)),
            config_path: path,
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

fn current_rss_kb() -> Option<usize> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find(|l| l.starts_with("VmRSS:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse::<usize>().ok())
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

async fn get_health() -> impl IntoResponse {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "rss_kb": current_rss_kb(),
    }))
}

async fn get_pairing() -> impl IntoResponse {
    Json(build_pairing_json())
}

async fn get_topology(State(state): State<AppState>) -> impl IntoResponse {
    let list = state.config.read().unwrap().screens.clone();
    Json(list)
}

async fn post_topology(
    State(state): State<AppState>,
    Json(updated): Json<Vec<ScreenRect>>,
) -> impl IntoResponse {
    let persisted = {
        let mut guard = state.config.write().unwrap();
        guard.screens = updated.clone();
        state
            .config_path
            .as_deref()
            .is_some_and(|p| guard.save(p).is_ok())
    };
    Json(json!({ "saved": updated.len(), "persisted": persisted }))
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

pub fn build_router(config_path: Option<PathBuf>) -> Router {
    build_router_with_live(config_path, live::LiveState::default())
}

pub fn build_router_with_live(config_path: Option<PathBuf>, live_state: live::LiveState) -> Router {
    let state = AppState::from_path(config_path);
    Router::new()
        .route("/", get(get_index))
        .route("/styles.css", get(get_css))
        .route("/app.js", get(get_js))
        .route("/api/health", get(get_health))
        .route("/api/pairing", get(get_pairing))
        .route("/api/topology", get(get_topology).post(post_topology))
        .route("/api/transfer-test", post(post_transfer_test))
        .with_state(state)
        .merge(live::router(live_state))
}

fn open_in_browser(url: &str) {
    #[cfg(windows)]
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--print-pairing") {
        println!("{}", build_pairing_json());
        return;
    }
    let config_path = args
        .iter()
        .position(|a| a == "--config")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .or_else(|| Some(PathBuf::from("omnikvm.json")));
    let addr = args
        .iter()
        .position(|a| a == "--serve")
        .and_then(|pos| args.get(pos + 1))
        .filter(|v| !v.starts_with("--"))
        .map(String::as_str)
        .unwrap_or("127.0.0.1:18080");
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("WARN: cannot bind {addr} ({e}); falling back to an available local port...");
            tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("failed to bind fallback localhost port")
        }
    };
    let actual = listener.local_addr().unwrap();
    let url = format!("http://{actual}");
    println!("OmniKVM Control Center listening on {url}");
    if !args.iter().any(|a| a == "--no-open") {
        open_in_browser(&url);
    }
    let live_state = live::LiveState::default();
    live::spawn_discovery(live_state.clone(), live::default_discovery_port());
    axum::serve(listener, build_router_with_live(config_path, live_state))
        .await
        .unwrap();
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
        let _router = build_router(None);
    }

    #[test]
    fn memory_footprint_under_15mb() {
        if let Some(rss_kb) = current_rss_kb() {
            assert!(
                rss_kb < 15 * 1024,
                "RSS too high: {} KB (limit 15360 KB)",
                rss_kb
            );
        }
    }

    #[tokio::test]
    async fn axum_control_center_http_endpoints_work() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let cfg_path = std::env::temp_dir().join("omnikvm_ui_cfg_test.json");
        let _ = std::fs::remove_file(&cfg_path);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let path_clone = cfg_path.clone();
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, build_router(Some(path_clone))).await;
        });

        async fn http_get(addr: std::net::SocketAddr, path: &str) -> String {
            let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
            let req = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
            s.write_all(req.as_bytes()).await.unwrap();
            let mut buf = Vec::new();
            s.read_to_end(&mut buf).await.unwrap();
            String::from_utf8_lossy(&buf).into_owned()
        }

        let health = http_get(addr, "/api/health").await;
        assert!(health.contains("\"status\":\"ok\""));

        let html = http_get(addr, "/").await;
        assert!(html.contains("200 OK") && html.contains("OmniKVM Control Plane"));

        let pairing = http_get(addr, "/api/pairing").await;
        assert!(pairing.contains("omnikvm://pair") && pairing.contains("<svg"));

        let peers = http_get(addr, "/api/peers").await;
        assert!(peers.contains("\"discovered\":[]"));

        let topo = http_get(addr, "/api/topology").await;
        assert!(topo.contains("Linux-Host-4K") && topo.contains("Win11-Work-FHD"));

        server.abort();
        let _ = std::fs::remove_file(&cfg_path);
    }
}
