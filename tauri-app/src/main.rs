#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use kvm_omni_core::crypto::{build_pairing_uri, generate_pairing_pin};
use tauri::tray::TrayIconBuilder;

#[tauri::command]
fn get_pairing_uri() -> String {
    let pin = generate_pairing_pin();
    build_pairing_uri("omnikvm-desktop", "127.0.0.1", 4433, &pin)
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let _tray = TrayIconBuilder::new()
                .tooltip("OmniKVM Hybrid Software KVM")
                .build(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_pairing_uri])
        .run(tauri::generate_context!())
        .expect("error while running OmniKVM Tauri v2 desktop app");
}
