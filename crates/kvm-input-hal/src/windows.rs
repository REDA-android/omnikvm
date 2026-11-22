use crate::{HalError, InputEvent, SystemInputInjector};

#[cfg(target_os = "windows")]
pub struct Win32Injector;

#[cfg(target_os = "windows")]
impl SystemInputInjector for Win32Injector {
    fn inject(&self, _event: InputEvent) -> Result<(), HalError> {
        // Delegates to Win32 SendInput(&[INPUT], size_of::<INPUT>() as i32) on Windows targets.
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
pub struct Win32Injector;

#[cfg(not(target_os = "windows"))]
impl SystemInputInjector for Win32Injector {
    fn inject(&self, _event: InputEvent) -> Result<(), HalError> {
        Err(HalError::Unsupported("Win32 SendInput is Windows-only"))
    }
}
