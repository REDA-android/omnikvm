# OmniKVM — Hybrid Multi-OS Software KVM

Modular, offline-first software KVM written in **Rust (90%)**, **Go (10%)**, and **C** (ESP32-S3 USB HID dongle).

## Workspace Crates
- `crates/kvm-omni-core` — Zero-copy binary wire protocol v1 (`CRC32C`), `Noise_IK_25519_ChaChaPoly_BLAKE2s` (`snow`), SPAKE2+/TOFU crypto, persistent `OmniConfig`, and multi-monitor DPI topology solver with cursor lock.
- `crates/kvm-input-hal` — Cross-platform input HAL (`Linux` `/dev/uinput` kernel ioctls, `X11 XTest`, `Wayland libei/reis`, `Windows` `SendInput`/`SetWindowsHookExW`, `macOS` `CoreGraphics`, `DDC/CI` VCP `0x60` monitor input switching, and `MockHal`).
- `crates/kvm-transport-quic` — Low-latency `quinn` QUIC endpoint with mutual SPAKE2+/TOFU authenticated handshake.
- `crates/kvm-transport-webrtc` — WebRTC DataChannel profiles and `< 50 ms` `FailoverCoordinator`.
- `crates/kvm-discovery-mdns` — LAN IPv4 multicast (`224.0.0.251`) peer discovery.
- `crates/kvm-clipboard-sync` — Semantic clipboard classifier, encrypted `ClipboardEnvelope`, ANSI **OSC 52** terminal clipboard sync (SSH/tmux), and 64 KB Blake3-XOF streaming (500 MB verified).
- `crates/kvm-mobile-bridge` — BLE GATT HID mouse/keyboard report maps and `scrcpy` control message encoder.
- `crates/kvm-esp32-hid` — Host framing & `-Werror` ESP32-S3 C firmware build (`firmware/esp32-s3/`).
- `crates/kvm-cli` — Live QUIC HID daemon (`serve`, `connect`), unified `OmniSessionRouter`, encrypted QUIC file transfer (`send-file`, `recv-file`), `pair`, `bench`, and `topology-check`.
- `crates/kvm-tray-ui` — Lightweight Axum SSE Control Center (`web/`) with drag-and-drop topology, Cursor Lock, `/api/health`, `/api/peers`, SVG QR pairing, and `OmniConfig` disk persistence.
- `tauri-app/` — Tauri v2 desktop wrapper with native system tray.
- `services/omnikvm-mdns-go` — Companion Go mDNS beacon helper.

## Quick Start
```bash
# Run all workspace tests in release mode
cargo test --workspace --release

# Check strict lints
cargo clippy --workspace --all-targets -- -D warnings

# Build portable static Linux musl + Go release tarball
bash scripts/package-release.sh

# Launch Control Center UI
cargo run -p kvm-tray-ui -- --serve 127.0.0.1:8080 --config ~/.omnikvm.json

# Start QUIC HID receiver (falls back to MockHal if /dev/uinput is absent)
cargo run -p kvm-cli -- serve --bind 127.0.0.1:4433 --mock
```
