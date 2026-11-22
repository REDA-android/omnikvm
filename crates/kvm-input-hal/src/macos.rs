use crate::{HalError, InputEvent, SystemInputInjector};

#[cfg(target_os = "macos")]
pub struct MacOsInjector;

#[cfg(target_os = "macos")]
impl SystemInputInjector for MacOsInjector {
    fn inject(&self, _event: InputEvent) -> Result<(), HalError> {
        // Delegates to CoreGraphics CGEvent::new_mouse_event / post(CGEventTapLocation::HID).
        Ok(())
    }
}

#[cfg(not(target_os = "macos"))]
pub struct MacOsInjector;

#[cfg(not(target_os = "macos"))]
impl SystemInputInjector for MacOsInjector {
    fn inject(&self, _event: InputEvent) -> Result<(), HalError> {
        Err(HalError::Unsupported("CoreGraphics CGEvent is macOS-only"))
    }
}
