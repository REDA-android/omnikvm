pub mod ddc;
pub mod ddc_i2c;
pub mod edge_kvm;
pub mod linux;
pub mod linux_capture;
pub mod macos;
pub mod mock;
pub mod wayland;
pub mod win_clipboard;
pub mod win_keymap;
pub mod windows;
pub mod x11;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum HalError {
    #[error("device open failed: {0}")]
    DeviceOpen(String),
    #[error("injection failed: {0}")]
    Injection(String),
    #[error("unsupported platform backend: {0}")]
    Unsupported(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputEvent {
    MouseMove { dx: i32, dy: i32 },
    MouseMoveAbsolute { x: u16, y: u16 },
    MouseButton { button: u8, pressed: bool },
    MouseWheel { dx: i16, dy: i16 },
    Key { keycode: u32, pressed: bool },
}

pub trait SystemInputHook: Send {
    fn start_capture<F>(&mut self, callback: F) -> Result<(), HalError>
    where
        F: Fn(InputEvent) + Send + 'static;

    fn stop_capture(&mut self);
}

pub trait SystemInputInjector: Send + Sync {
    fn inject(&self, event: InputEvent) -> Result<(), HalError>;

    fn inject_mouse_move(&self, dx: i32, dy: i32) -> Result<(), HalError> {
        self.inject(InputEvent::MouseMove { dx, dy })
    }

    fn inject_key(&self, keycode: u32, pressed: bool) -> Result<(), HalError> {
        self.inject(InputEvent::Key { keycode, pressed })
    }
}
