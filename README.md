# OmniKVM — Hybrid Multi-OS Software KVM

Modular, offline-first software KVM written in **Rust (90%)**, **Go (10%)**, and **C** (ESP32-S3 USB HID dongle).

## Workspace Crates
- `crates/kvm-omni-core` — Zero-copy binary wire protocol v1 (`CRC32C`), SPAKE2+-style pairing/TOFU, and multi-monitor DPI topology solver.
- `crates/kvm-input-hal` — Cross-platform input HAL (`Linux` uinput/X11/Wayland `libei`, `Windows` `SendInput`/`SetWindowsHookExW`, `macOS` `CoreGraphics`, and `MockHal`).
- `crates/kvm-transport-quic` — Low-latency `quinn` QUIC endpoint with TOFU certificates.
- `crates/kvm-transport-webrtc` — WebRTC DataChannel profiles and `< 50 ms` failover coordinator.
- `crates/kvm-discovery-mdns` — LAN IPv4 multicast (`224.0.0.251`) peer discovery.
- `crates/kvm-clipboard-sync` — Semantic clipboard classifier and 64 KB Blake3-XOF encrypted streaming.
- `crates/kvm-mobile-bridge` — BLE GATT HID report maps and `scrcpy` control message encoder.
- `crates/kvm-esp32-hid` — Host framing & `-Werror` C firmware build (`firmware/esp32-s3/`).
- `crates/kvm-cli` — CLI & live QUIC HID daemon (`serve`, `connect`, `pair`, `bench`, `topology-check`).
- `crates/kvm-tray-ui` — Lightweight Axum SSE Control Center (`web/`) with SVG QR pairing.
- `tauri-app/` — Tauri v2 desktop wrapper with native system tray.
- `services/omnikvm-mdns-go` — Companion Go mDNS beacon helper.

## Quick Start
```bash
# Run all workspace tests in release mode
cargo test --workspace --release

# Check strict lints
cargo clippy --workspace --all-targets -- -D warnings

# Launch Control Center UI
cargo run -p kvm-tray-ui -- --serve 127.0.0.1:8080

# Start QUIC HID receiver (mock HAL fallback if /dev/uinput is absent)
cargo run -p kvm-cli -- serve --bind 127.0.0.1:4433 --mock
```
