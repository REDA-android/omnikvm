# OmniKVM — Hybrid Multi-OS Software KVM

Modular, offline-first software KVM written in **Rust (90%)**, **Go (10%)**, and **C** (ESP32-S3 USB HID dongle).

## Workspace Crates
- `crates/kvm-omni-core` — Zero-copy binary wire protocol v1 (`CRC32C`), `Noise_IK_25519_ChaChaPoly_BLAKE2s` (`snow`), SPAKE2+/TOFU crypto with hex persistence, session-keyed MAC + 64-entry anti-replay `datagram_auth`, configurable global `hotkeys`, persistent `OmniConfig`, and multi-monitor DPI topology solver with cursor lock.
- `crates/kvm-input-hal` — Cross-platform input HAL:
  - **Windows**: `Win32Hook` (`WH_MOUSE_LL`/`WH_KEYBOARD_LL` with `LLMHF_INJECTED` feedback protection, seamless screen-edge switching `Right`/`Left`/`Top`/`Bottom`, local input swallowing while controlling remote PC, and canonical Set-1 + extended scancode mapping), `Win32Injector` (`SendInput` with `KEYEVENTF_EXTENDEDKEY`), and native UTF-16 clipboard (`win_clipboard`).
  - **Linux / macOS**: `/dev/uinput` + `EvdevCapture` (`EVIOCGRAB`), `X11 XTest`, `Wayland libei/reis`, `CoreGraphics`, and `DDC/CI` `/dev/i2c-N`.
- `crates/kvm-transport-quic` — Low-latency `quinn` QUIC endpoint with persistent TLS identity (`node_cert.der`/`node_key.der`), TLS-bound TOFU verification, mutual SPAKE2+ handshake, and sealed anti-replay HID datagrams.
- `crates/kvm-transport-webrtc` — WebRTC DataChannel profiles and `< 50 ms` `FailoverCoordinator`.
- `crates/kvm-discovery-mdns` — Automatic LAN IPv4 subnet broadcast (`255.255.255.255`) + mDNS multicast (`224.0.0.251`) peer announcer & listener with offline LAN IP detection.
- `crates/kvm-clipboard-sync` — Semantic clipboard classifier, encrypted `ClipboardEnvelope`, ANSI **OSC 52** terminal clipboard sync, and 64 KB Blake3-XOF streaming.
- `crates/kvm-mobile-bridge` — BLE GATT HID mouse/keyboard report maps and `scrcpy` control message encoder.
- `crates/kvm-esp32-hid` — Host framing & `-Werror` ESP32-S3 C firmware build (`firmware/esp32-s3/`).
- `crates/kvm-cli` — Authenticated QUIC HID daemon (`serve --pin`, `connect --pin`, `forward`), unified `OmniSessionRouter`, encrypted QUIC file transfer (`send-file`, `recv-file`), `ddc-switch`, `ddc-list`, `clipboard --osc52`, `pair`, `bench`, and `topology-check`.
- `crates/kvm-tray-ui` — Embedded Control Center library & standalone app (`web/`) with 1-click QUIC Host/Client sessions, seamless edge switching, automatic bidirectional encrypted clipboard sync, live LAN peer auto-discovery, drag-and-drop topology, and SSE telemetry.
- `tauri-app/` — Self-contained native **Tauri v2** desktop app (`omnikvm-desktop.exe`) embedding the full `kvm-tray-ui` QUIC/mDNS/Win32 engine and system tray icon.
- `services/omnikvm-mdns-go` — Companion Go mDNS beacon helper.

## Quick Start (Windows PowerShell)
```powershell
git pull

# Option A — Native Windows Desktop Window (Tauri v2, zero browser needed)
cd tauri-app
cargo build --release
.\target\release\omnikvm-desktop.exe

# Option B — Lightweight Tray / Browser Control Center + CLI
cd ..
cargo build --release -p kvm-tray-ui -p kvm-cli
.\target\release\kvm-tray-ui.exe
```

## Quick Start (Linux / Dev)
```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --release
bash scripts/package-release.sh
```
