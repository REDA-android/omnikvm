use kvm_tray_ui::{build_pairing_json, spawn_embedded_server};
use std::path::PathBuf;

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

    let actual = spawn_embedded_server(addr, config_path)
        .await
        .expect("failed to start OmniKVM Control Center");
    let url = format!("http://{actual}");
    println!("OmniKVM Control Center listening on {url}");
    if !args.iter().any(|a| a == "--no-open") {
        open_in_browser(&url);
    }
    let _ = tokio::signal::ctrl_c().await;
}
