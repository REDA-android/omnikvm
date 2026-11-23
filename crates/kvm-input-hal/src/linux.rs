use crate::{HalError, InputEvent, SystemInputInjector};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinuxSessionBackend {
    X11XTest,
    WaylandLibei,
    UinputKernel,
}

pub fn detect_session_backend() -> LinuxSessionBackend {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        LinuxSessionBackend::WaylandLibei
    } else if std::env::var_os("DISPLAY").is_some() {
        LinuxSessionBackend::X11XTest
    } else {
        LinuxSessionBackend::UinputKernel
    }
}

/// Linux `uinput` ioctl request numbers (x86_64/aarch64 `linux/uinput.h`).
pub mod uinput_ioctl {
    pub const UI_DEV_CREATE: u64 = 0x5501;
    pub const UI_DEV_DESTROY: u64 = 0x5502;
    pub const UI_SET_EVBIT: u64 = 0x4004_5564;
    pub const UI_SET_KEYBIT: u64 = 0x4004_5565;
    pub const UI_SET_RELBIT: u64 = 0x4004_5566;
    pub const UI_DEV_SETUP: u64 = 0x405c_5503;
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct UinputSetup {
    pub bustype: u16,
    pub vendor: u16,
    pub product: u16,
    pub version: u16,
    pub name: [u8; 80],
    pub ff_effects_max: u32,
}

impl UinputSetup {
    pub fn omnikvm_virtual(name: &str) -> Self {
        let mut buf = [0u8; 80];
        let bytes = name.as_bytes();
        let n = bytes.len().min(79);
        buf[..n].copy_from_slice(&bytes[..n]);
        Self {
            bustype: 0x06, // BUS_VIRTUAL
            vendor: 0x4F4B,
            product: 0x0001,
            version: 1,
            name: buf,
            ff_effects_max: 0,
        }
    }
}

/// Raw Linux `struct input_event` (24 bytes on 64-bit).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawInputEvent {
    pub tv_sec: i64,
    pub tv_usec: i64,
    pub ev_type: u16,
    pub code: u16,
    pub value: i32,
}

impl RawInputEvent {
    pub fn new(ev_type: u16, code: u16, value: i32) -> Self {
        Self {
            tv_sec: 0,
            tv_usec: 0,
            ev_type,
            code,
            value,
        }
    }

    pub fn to_bytes(&self) -> [u8; 24] {
        let mut out = [0u8; 24];
        out[0..8].copy_from_slice(&self.tv_sec.to_ne_bytes());
        out[8..16].copy_from_slice(&self.tv_usec.to_ne_bytes());
        out[16..18].copy_from_slice(&self.ev_type.to_ne_bytes());
        out[18..20].copy_from_slice(&self.code.to_ne_bytes());
        out[20..24].copy_from_slice(&self.value.to_ne_bytes());
        out
    }
}

pub struct UinputDevice {
    file: Mutex<File>,
}

impl UinputDevice {
    pub fn open_default() -> Result<Self, HalError> {
        Self::open_path(Path::new("/dev/uinput"))
    }

    pub fn open_path(path: &Path) -> Result<Self, HalError> {
        let file = OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(|e| HalError::DeviceOpen(format!("{}: {e}", path.display())))?;
        Ok(Self {
            file: Mutex::new(file),
        })
    }

    pub fn emit_raw(&self, ev_type: u16, code: u16, value: i32) -> Result<(), HalError> {
        let ev = RawInputEvent::new(ev_type, code, value);
        let mut guard = self.file.lock().unwrap();
        guard
            .write_all(&ev.to_bytes())
            .map_err(|e| HalError::Injection(e.to_string()))?;
        Ok(())
    }
}

impl SystemInputInjector for UinputDevice {
    fn inject(&self, event: InputEvent) -> Result<(), HalError> {
        const EV_SYN: u16 = 0x00;
        const EV_KEY: u16 = 0x01;
        const EV_REL: u16 = 0x02;
        const REL_X: u16 = 0x00;
        const REL_Y: u16 = 0x01;
        const REL_WHEEL: u16 = 0x08;
        const SYN_REPORT: u16 = 0x00;

        match event {
            InputEvent::MouseMove { dx, dy } => {
                if dx != 0 {
                    self.emit_raw(EV_REL, REL_X, dx)?;
                }
                if dy != 0 {
                    self.emit_raw(EV_REL, REL_Y, dy)?;
                }
                self.emit_raw(EV_SYN, SYN_REPORT, 0)
            }
            InputEvent::MouseWheel { dy, .. } => {
                self.emit_raw(EV_REL, REL_WHEEL, dy as i32)?;
                self.emit_raw(EV_SYN, SYN_REPORT, 0)
            }
            InputEvent::MouseButton { button, pressed } => {
                let code = 0x110u16 + button as u16;
                self.emit_raw(EV_KEY, code, i32::from(pressed))?;
                self.emit_raw(EV_SYN, SYN_REPORT, 0)
            }
            InputEvent::Key { keycode, pressed } => {
                self.emit_raw(EV_KEY, keycode as u16, i32::from(pressed))?;
                self.emit_raw(EV_SYN, SYN_REPORT, 0)
            }
            InputEvent::MouseMoveAbsolute { .. } => Err(HalError::Unsupported(
                "absolute uinput requires ABS_X/ABS_Y setup",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uinput_writes_valid_evdev_frames_to_sink() {
        let tmp = std::env::temp_dir().join("omnikvm_uinput_test.bin");
        let _ = File::create(&tmp).unwrap();
        let dev = UinputDevice::open_path(&tmp).unwrap();
        dev.inject(InputEvent::MouseMove { dx: 12, dy: -4 }).unwrap();
        dev.inject(InputEvent::Key {
            keycode: 30,
            pressed: true,
        })
        .unwrap();
        let bytes = std::fs::read(&tmp).unwrap();
        let _ = std::fs::remove_file(&tmp);
        // MouseMove (REL_X, REL_Y, SYN) = 3 * 24 bytes; Key (EV_KEY, SYN) = 2 * 24 bytes => 120 bytes
        assert_eq!(bytes.len(), 120);
        let setup = UinputSetup::omnikvm_virtual("OmniKVM Virtual Mouse");
        assert_eq!(setup.vendor, 0x4F4B);
    }
}
