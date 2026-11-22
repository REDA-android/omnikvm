use crate::{HalError, InputEvent, SystemInputInjector};
use std::fs::{File, OpenOptions};
use std::io::Write;
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
        let file = OpenOptions::new()
            .write(true)
            .open("/dev/uinput")
            .map_err(|e| HalError::DeviceOpen(format!("/dev/uinput: {e}")))?;
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
