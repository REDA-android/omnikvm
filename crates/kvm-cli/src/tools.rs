//! Extra CLI tooling: DDC/CI monitor switching, clipboard / OSC 52 helpers and the
//! live evdev -> authenticated QUIC forwarder with hotkey handling.

use crate::secure::{connect_secure, forward_stream, SecureOpts};
use anyhow::{bail, Result};
use kvm_clipboard_sync::osc52::encode_osc52;
use kvm_clipboard_sync::semantic::classify_clipboard_bytes;
use kvm_input_hal::ddc::{build_input_switch_frame, MonitorInputSource, VCP_INPUT_SOURCE};
use kvm_input_hal::ddc_i2c::{list_i2c_buses, set_vcp};
#[cfg(not(windows))]
use kvm_input_hal::linux_capture::EvdevCapture;
use kvm_input_hal::{InputEvent, SystemInputHook};
use kvm_omni_core::hotkeys::{HotkeyAction, HotkeyManager};
use std::io::Read;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

pub fn ddc_switch(input: &str, bus: Option<&Path>, dry_run: bool) -> Result<()> {
    let Some(src) = MonitorInputSource::parse(input) else {
        bail!("unknown input '{input}' (expected dp1, dp2, hdmi1, hdmi2, usbc)");
    };
    let frame = build_input_switch_frame(src);
    println!("DDC/CI Set VCP 0x60 -> {src:?}: [{}]", hex(&frame));
    if dry_run {
        return Ok(());
    }
    let Some(bus) = bus else {
        let buses = list_i2c_buses();
        bail!(
            "pass --bus <path>; candidate buses: {:?} (load `i2c-dev`, see `ddcutil detect`)",
            buses
        );
    };
    set_vcp(bus, VCP_INPUT_SOURCE, src as u16)?;
    println!("Switched monitor on {} to {src:?}", bus.display());
    Ok(())
}

pub fn ddc_list() {
    let buses = list_i2c_buses();
    if buses.is_empty() {
        println!("No /dev/i2c-* buses (run `sudo modprobe i2c-dev`).");
    }
    for b in buses {
        println!("{}", b.display());
    }
    for src in MonitorInputSource::ALL {
        println!("  input {:<13} VCP 0x60 = 0x{:02x}", format!("{src:?}"), src as u16);
    }
}

/// Reads stdin, prints the semantic format and optionally an OSC 52 sequence.
pub fn clipboard_from_stdin(osc52: bool) -> Result<()> {
    let mut data = Vec::new();
    std::io::stdin().read_to_end(&mut data)?;
    let fmt = classify_clipboard_bytes(&data);
    if osc52 {
        print!("{}", encode_osc52(&data));
        eprintln!("OSC 52 emitted ({} bytes, {fmt:?})", data.len());
    } else {
        println!("{} bytes, format {fmt:?}, blake3 {}", data.len(), &blake3::hash(&data).to_hex()[..16]);
    }
    Ok(())
}

/// Hotkey-aware filter: returns `false` when the event must be swallowed locally.
pub fn filter_event(hk: &mut HotkeyManager, locked: &mut bool, ev: InputEvent) -> (bool, Option<HotkeyAction>) {
    let InputEvent::Key { keycode, pressed } = ev else {
        return (!*locked, None);
    };
    match hk.on_key(keycode, pressed) {
        Some(HotkeyAction::ToggleCursorLock) => {
            *locked = !*locked;
            (false, Some(HotkeyAction::ToggleCursorLock))
        }
        Some(a) => (false, Some(a)),
        None => (!*locked || !pressed, None),
    }
}

pub async fn forward_device(
    device: Option<PathBuf>,
    peer: SocketAddr,
    opts: SecureOpts,
    grab: bool,
) -> Result<()> {
    let (endpoint, conn, mut sealer) = connect_secure(peer, &opts).await?;
    let (tx, rx) = tokio::sync::mpsc::channel::<Option<InputEvent>>(1024);
    let hk = std::sync::Mutex::new((HotkeyManager::with_defaults(), false));
    let on_event = move |ev: InputEvent| {
        let mut g = hk.lock().unwrap();
        let (hkm, locked) = &mut *g;
        let (forward, action) = filter_event(hkm, locked, ev);
        if let Some(a) = action {
            eprintln!("hotkey: {a:?} (cursor lock = {locked})");
        }
        if action == Some(HotkeyAction::ReleaseCapture) {
            let _ = tx.blocking_send(None);
        } else if forward {
            let _ = tx.blocking_send(Some(ev));
        }
    };

    #[cfg(windows)]
    let mut cap = {
        let _ = (device, grab);
        println!("Authenticated with {peer}; capturing Windows mouse/keyboard (Ctrl+Alt+Esc to stop, Ctrl+Alt+L lock)");
        let mut h = kvm_input_hal::windows::Win32Hook::default();
        h.start_capture(on_event)?;
        h
    };
    #[cfg(not(windows))]
    let mut cap = {
        let dev = device.ok_or_else(|| anyhow::anyhow!("--device /dev/input/eventX is required on Linux"))?;
        println!("Authenticated with {peer}; forwarding {} (Ctrl+Alt+Esc to stop, Ctrl+Alt+L lock)", dev.display());
        let mut h = EvdevCapture::new(&dev, grab);
        h.start_capture(on_event)?;
        h
    };

    let (ftx, frx) = tokio::sync::mpsc::channel(1024);
    let pump = tokio::spawn(async move {
        let mut rx = rx;
        while let Some(Some(ev)) = rx.recv().await {
            if ftx.send(ev).await.is_err() {
                break;
            }
        }
    });
    let sent = forward_stream(&conn, &mut sealer, frx).await?;
    let _ = pump.await;
    cap.stop_capture();
    conn.close(0u32.into(), b"released");
    endpoint.wait_idle().await;
    println!("Forwarded {sent} event(s); capture released.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkey_filter_swallows_chords_and_honours_lock() {
        let mut hk = HotkeyManager::with_defaults();
        let mut locked = false;
        let key = |k, p| InputEvent::Key { keycode: k, pressed: p };
        assert!(filter_event(&mut hk, &mut locked, InputEvent::MouseMove { dx: 1, dy: 0 }).0);
        filter_event(&mut hk, &mut locked, key(29, true));
        filter_event(&mut hk, &mut locked, key(56, true));
        let (fwd, act) = filter_event(&mut hk, &mut locked, key(38, true));
        assert!(!fwd && act == Some(HotkeyAction::ToggleCursorLock) && locked);
        assert!(!filter_event(&mut hk, &mut locked, InputEvent::MouseMove { dx: 1, dy: 0 }).0);
        assert!(filter_event(&mut hk, &mut locked, key(56, false)).0); // releases still flow
        assert!(ddc_switch("hdmi2", None, true).is_ok());
        assert!(ddc_switch("vga", None, true).is_err());
        assert_eq!(hex(&[0x51, 0x84]), "51 84");
    }
}
