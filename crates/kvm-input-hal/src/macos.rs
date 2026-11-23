use crate::{HalError, InputEvent, SystemInputInjector};

#[cfg(target_os = "macos")]
mod darwin_impl {
    use super::*;
    use crate::SystemInputHook;
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Debug, Clone, Copy)]
    pub struct CGPoint {
        pub x: f64,
        pub y: f64,
    }

    type CGEventRef = *mut c_void;
    type CGEventSourceRef = *mut c_void;
    type CFMachPortRef = *mut c_void;

    const K_CG_HID_EVENT_TAP: u32 = 0;
    const K_CG_EVENT_LEFT_MOUSE_DOWN: u32 = 1;
    const K_CG_EVENT_LEFT_MOUSE_UP: u32 = 2;
    const K_CG_EVENT_RIGHT_MOUSE_DOWN: u32 = 3;
    const K_CG_EVENT_RIGHT_MOUSE_UP: u32 = 4;
    const K_CG_EVENT_MOUSE_MOVED: u32 = 5;
    const K_CG_EVENT_OTHER_MOUSE_DOWN: u32 = 25;
    const K_CG_EVENT_OTHER_MOUSE_UP: u32 = 26;
    const K_CG_MOUSE_EVENT_DELTA_X: u32 = 4;
    const K_CG_MOUSE_EVENT_DELTA_Y: u32 = 5;
    const K_CG_SCROLL_WHEEL_EVENT_UNIT_LINE: u32 = 1;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventCreateMouseEvent(
            source: CGEventSourceRef,
            mouse_type: u32,
            mouse_cursor_position: CGPoint,
            mouse_button: u32,
        ) -> CGEventRef;
        fn CGEventCreateKeyboardEvent(
            source: CGEventSourceRef,
            virtual_key: u16,
            key_down: bool,
        ) -> CGEventRef;
        fn CGEventCreateScrollWheelEvent(
            source: CGEventSourceRef,
            units: u32,
            wheel_count: u32,
            wheel1: i32,
            wheel2: i32,
        ) -> CGEventRef;
        fn CGEventSetIntegerValueField(event: CGEventRef, field: u32, value: i64);
        fn CGEventPost(tap: u32, event: CGEventRef);
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: *const c_void);
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }

    pub struct MacOsInjector;

    impl MacOsInjector {
        pub fn is_accessibility_trusted() -> bool {
            unsafe { AXIsProcessTrusted() }
        }

        unsafe fn post_and_release(ev: CGEventRef) -> Result<(), HalError> {
            if ev.is_null() {
                return Err(HalError::Injection("CGEventCreate returned null".into()));
            }
            CGEventPost(K_CG_HID_EVENT_TAP, ev);
            CFRelease(ev as *const c_void);
            Ok(())
        }
    }

    impl SystemInputInjector for MacOsInjector {
        fn inject(&self, event: InputEvent) -> Result<(), HalError> {
            unsafe {
                match event {
                    InputEvent::MouseMove { dx, dy } => {
                        let ev = CGEventCreateMouseEvent(
                            std::ptr::null_mut(),
                            K_CG_EVENT_MOUSE_MOVED,
                            CGPoint { x: 0.0, y: 0.0 },
                            0,
                        );
                        if !ev.is_null() {
                            CGEventSetIntegerValueField(ev, K_CG_MOUSE_EVENT_DELTA_X, dx as i64);
                            CGEventSetIntegerValueField(ev, K_CG_MOUSE_EVENT_DELTA_Y, dy as i64);
                        }
                        Self::post_and_release(ev)
                    }
                    InputEvent::MouseMoveAbsolute { x, y } => {
                        let ev = CGEventCreateMouseEvent(
                            std::ptr::null_mut(),
                            K_CG_EVENT_MOUSE_MOVED,
                            CGPoint {
                                x: x as f64,
                                y: y as f64,
                            },
                            0,
                        );
                        Self::post_and_release(ev)
                    }
                    InputEvent::MouseButton { button, pressed } => {
                        let (ev_type, btn) = match (button, pressed) {
                            (0, true) => (K_CG_EVENT_LEFT_MOUSE_DOWN, 0),
                            (0, false) => (K_CG_EVENT_LEFT_MOUSE_UP, 0),
                            (1, true) => (K_CG_EVENT_RIGHT_MOUSE_DOWN, 1),
                            (1, false) => (K_CG_EVENT_RIGHT_MOUSE_UP, 1),
                            (2, true) => (K_CG_EVENT_OTHER_MOUSE_DOWN, 2),
                            (2, false) => (K_CG_EVENT_OTHER_MOUSE_UP, 2),
                            _ => return Err(HalError::Unsupported("unknown macOS mouse button")),
                        };
                        let ev = CGEventCreateMouseEvent(
                            std::ptr::null_mut(),
                            ev_type,
                            CGPoint { x: 0.0, y: 0.0 },
                            btn,
                        );
                        Self::post_and_release(ev)
                    }
                    InputEvent::MouseWheel { dx, dy } => {
                        let ev = CGEventCreateScrollWheelEvent(
                            std::ptr::null_mut(),
                            K_CG_SCROLL_WHEEL_EVENT_UNIT_LINE,
                            2,
                            dy as i32,
                            dx as i32,
                        );
                        Self::post_and_release(ev)
                    }
                    InputEvent::Key { keycode, pressed } => {
                        let ev =
                            CGEventCreateKeyboardEvent(std::ptr::null_mut(), keycode as u16, pressed);
                        Self::post_and_release(ev)
                    }
                }
            }
        }
    }

    pub struct MacOsEventTap {
        _mach_port: Option<CFMachPortRef>,
    }

    unsafe impl Send for MacOsEventTap {}

    impl Default for MacOsEventTap {
        fn default() -> Self {
            Self { _mach_port: None }
        }
    }

    impl SystemInputHook for MacOsEventTap {
        fn start_capture<F>(&mut self, _callback: F) -> Result<(), HalError>
        where
            F: Fn(InputEvent) + Send + 'static,
        {
            if !MacOsInjector::is_accessibility_trusted() {
                return Err(HalError::DeviceOpen(
                    "macOS Accessibility permission (AXIsProcessTrusted) required".into(),
                ));
            }
            Ok(())
        }

        fn stop_capture(&mut self) {
            if let Some(port) = self._mach_port.take() {
                unsafe { CFRelease(port as *const c_void) };
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub use darwin_impl::{MacOsEventTap, MacOsInjector};

#[cfg(not(target_os = "macos"))]
pub struct MacOsInjector;

#[cfg(not(target_os = "macos"))]
impl SystemInputInjector for MacOsInjector {
    fn inject(&self, _event: InputEvent) -> Result<(), HalError> {
        Err(HalError::Unsupported("CoreGraphics CGEvent is macOS-only"))
    }
}
