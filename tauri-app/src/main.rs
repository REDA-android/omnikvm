#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::mpsc::sync_channel;
use tauri::tray::TrayIconBuilder;
use tauri::{Url, WebviewUrl, WebviewWindowBuilder};

fn start_backend() -> SocketAddr {
    let (tx, rx) = sync_channel(1);
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to build Tokio runtime");
        rt.block_on(async move {
            let addr = kvm_tray_ui::spawn_embedded_server(
                "127.0.0.1:18080",
                Some(PathBuf::from("omnikvm.json")),
            )
            .await
            .expect("failed to bind embedded Control Center");
            let _ = tx.send(addr);
            std::future::pending::<()>().await;
        });
    });
    rx.recv().expect("embedded server did not report address")
}

fn main() {
    let addr = start_backend();
    let url: Url = format!("http://{addr}")
        .parse()
        .expect("valid localhost URL");

    tauri::Builder::default()
        .setup(move |app| {
            let _tray = TrayIconBuilder::new()
                .tooltip("OmniKVM — KVM logiciel chiffré")
                .build(app)?;
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("OmniKVM Control Plane")
                .inner_size(1160.0, 780.0)
                .resizable(true)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running OmniKVM desktop app");
}
