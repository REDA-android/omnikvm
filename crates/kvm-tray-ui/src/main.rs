use kvm_discovery_mdns::{PeerAnnouncement, SERVICE_TYPE};
use kvm_omni_core::crypto::{build_pairing_uri, generate_pairing_pin};
use qrcode::render::svg;
use qrcode::QrCode;
use serde_json::json;
use std::io::{Read, Write};
use std::net::TcpListener;

const INDEX_HTML: &str = include_str!("../../../web/index.html");
const STYLES_CSS: &str = include_str!("../../../web/styles.css");
const APP_JS: &str = include_str!("../../../web/app.js");

pub fn render_pairing_qr_svg(uri: &str) -> String {
    let code = QrCode::new(uri.as_bytes()).expect("valid QR data");
    code.render::<svg::Color>()
        .min_dimensions(180, 180)
        .build()
}

pub fn build_pairing_json() -> String {
    let pin = generate_pairing_pin();
    let uri = build_pairing_uri("omnikvm-tray", "127.0.0.1", 4433, &pin);
    let svg = render_pairing_qr_svg(&uri);
    json!({
        "pin": pin,
        "uri": uri,
        "qr_svg": svg,
        "service": SERVICE_TYPE
    })
    .to_string()
}

pub fn build_peers_json() -> String {
    let local = PeerAnnouncement::new("omnikvm-tray", "localhost", 4433, 4434, "tofu-local");
    json!({
        "service": SERVICE_TYPE,
        "local": local
    })
    .to_string()
}

fn http_response(content_type: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn serve_control_ui(bind_addr: &str) -> std::io::Result<()> {
    let listener = TcpListener::bind(bind_addr)?;
    println!("OmniKVM Tray Control UI listening on http://{bind_addr}");
    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(s) => s,
            Err(_) => continue,
        };
        let mut buf = [0u8; 1024];
        let n = stream.read(&mut buf).unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..n]);
        let path = req
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("/");

        let resp = match path {
            "/styles.css" => http_response("text/css", STYLES_CSS),
            "/app.js" => http_response("application/javascript", APP_JS),
            "/api/pairing" => http_response("application/json", &build_pairing_json()),
            "/api/peers" => http_response("application/json", &build_peers_json()),
            _ => http_response("text/html", INDEX_HTML),
        };
        let _ = stream.write_all(resp.as_bytes());
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == "--serve") {
        let addr = args.get(pos + 1).map(String::as_str).unwrap_or("127.0.0.1:8080");
        if let Err(e) = serve_control_ui(addr) {
            eprintln!("server error: {e}");
            std::process::exit(1);
        }
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
        let pairing = build_pairing_json();
        assert!(pairing.contains("omnikvm://pair"));
        let peers = build_peers_json();
        assert!(peers.contains("_omnikvm._udp.local."));
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
