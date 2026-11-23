# OmniKVM — Hybrid Multi-OS Software KVM

Modular, offline-first software KVM written in **Rust (90%)**, **Go (10%)**, and **C** (ESP32-S3 USB HID dongle).

## Workspace Crates
- `crates/kvm-omni-core` — Zero-copy binary wire protocol v1 (`CRC32C`), `Noise_IK_25519_ChaChaPoly_BLAKE2s` (`snow`), SPAKE2+/TOFU crypto with hex persistence, session-keyed MAC + 64-entry anti-replay `datagram_auth`, configurable global `hotkeys`, persistent `OmniConfig`, and multi-monitor DPI topology solver with cursor lock.
- `crates/kvm-input-hal` — Cross-platform input HAL (`Linux` `/dev/uinput` kernel ioctls + `EvdevCapture` `/dev/input/event*` with `EVIOCGRAB`, `X11 XTest`, `Wayland libei/reis`, `Windows` `SendInput`/`SetWindowsHookExW`, `macOS` `CoreGraphics`, `DDC/CI` VCP `0x60` + `/dev/i2c-N` writer, and `MockHal`).
- `crates/kvm-transport-quic` — Low-latency `quinn` QUIC endpoint with persistent TLS identity (`node_cert.der`/`node_key.der`), TLS-bound TOFU verification, mutual SPAKE2+ handshake, and sealed anti-replay HID datagrams.
- `crates/kvm-transport-webrtc` — WebRTC DataChannel profiles and `< 50 ms` `FailoverCoordinator`.
- `crates/kvm-discovery-mdns` — LAN IPv4 multicast (`224.0.0.251`) peer discovery.
- `crates/kvm-clipboard-sync` — Semantic clipboard classifier, encrypted `ClipboardEnvelope`, ANSI **OSC 52** terminal clipboard sync (SSH/tmux), and 64 KB Blake3-XOF streaming (500 MB verified).
- `crates/kvm-mobile-bridge` — BLE GATT HID mouse/keyboard report maps and `scrcpy` control message encoder.
- `crates/kvm-esp32-hid` — Host framing & `-Werror` ESP32-S3 C firmware build (`firmware/esp32-s3/`).
- `crates/kvm-cli` — Authenticated QUIC HID daemon (`serve --pin`, `connect --pin`, `forward --device`), unified `OmniSessionRouter`, encrypted QUIC file transfer (`send-file`, `recv-file`), `ddc-switch`, `ddc-list`, `clipboard --osc52`, `pair`, `bench`, and `topology-check`.
- `crates/kvm-tray-ui` — Lightweight Axum SSE Control Center (`web/`) with live LAN mDNS discovery, drag-and-drop topology, server-backed Cursor Lock (`/api/cursor-lock`), measured pipeline latency SSE, DDC/CI monitor input switch (`/api/ddc/switch`), Clipboard & OSC 52 Lab (`/api/clipboard`), SVG QR pairing, and `OmniConfig` disk persistence.
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

# Start authenticated QUIC HID receiver (SPAKE2+/TOFU + sealed anti-replay datagrams)
cargo run -p kvm-cli -- serve --bind 127.0.0.1:4433 --pin 482910 --config ~/.omnikvm.json --mock

# Switch monitor input over DDC/CI (dry-run prints MCCS VCP 0x60 frame)
cargo run -p kvm-cli -- ddc-switch --input hdmi1 --dry-run

# Classify stdin and emit an SSH/tmux OSC 52 clipboard sequence
echo 'fn main() {}' | cargo run -p kvm-cli -- clipboard --osc52
```
