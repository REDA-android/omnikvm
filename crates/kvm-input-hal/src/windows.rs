use crate::{HalError, InputEvent, SystemInputInjector};

#[cfg(target_os = "windows")]
mod win32_impl {
    use super::*;
    use crate::SystemInputHook;
    use std::mem::size_of;
    use std::sync::{Arc, Mutex, OnceLock};
    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP,
        KEYEVENTF_SCANCODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN,
        MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
        MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, HHOOK, KBDLLHOOKSTRUCT, MSLLHOOKSTRUCT, SetWindowsHookExW,
        UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN,
        WM_LBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_RBUTTONUP,
    };

    type HookCallback = Arc<Mutex<Option<Box<dyn Fn(InputEvent) + Send + 'static>>>>;
    static GLOBAL_CB: OnceLock<HookCallback> = OnceLock::new();

    fn dispatch_event(ev: InputEvent) {
        if let Some(cell) = GLOBAL_CB.get() {
            if let Ok(guard) = cell.lock() {
                if let Some(cb) = guard.as_ref() {
                    cb(ev);
                }
            }
        }
    }

    unsafe extern "system" fn low_level_mouse_proc(
        n_code: i32,
        w_param: WPARAM,
        l_param: LPARAM,
    ) -> LRESULT {
        if n_code >= 0 {
            let info = &*(l_param as *const MSLLHOOKSTRUCT);
            match w_param as u32 {
                WM_MOUSEMOVE => dispatch_event(InputEvent::MouseMoveAbsolute {
                    x: info.pt.x.clamp(0, u16::MAX as i32) as u16,
                    y: info.pt.y.clamp(0, u16::MAX as i32) as u16,
                }),
                WM_LBUTTONDOWN => {
                    dispatch_event(InputEvent::MouseButton { button: 0, pressed: true })
                }
                WM_LBUTTONUP => {
                    dispatch_event(InputEvent::MouseButton { button: 0, pressed: false })
                }
                WM_RBUTTONDOWN => {
                    dispatch_event(InputEvent::MouseButton { button: 1, pressed: true })
                }
                WM_RBUTTONUP => {
                    dispatch_event(InputEvent::MouseButton { button: 1, pressed: false })
                }
                _ => {}
            }
        }
        CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param)
    }

    unsafe extern "system" fn low_level_kbd_proc(
        n_code: i32,
        w_param: WPARAM,
        l_param: LPARAM,
    ) -> LRESULT {
        if n_code >= 0 {
            let info = &*(l_param as *const KBDLLHOOKSTRUCT);
            let pressed = (w_param as u32) == WM_KEYDOWN;
            if pressed || (w_param as u32) == WM_KEYUP {
                dispatch_event(InputEvent::Key {
                    keycode: info.scanCode,
                    pressed,
                });
            }
        }
        CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param)
    }

    pub struct Win32Hook {
        mouse_hook: Option<HHOOK>,
        kbd_hook: Option<HHOOK>,
    }

    unsafe impl Send for Win32Hook {}

    impl Default for Win32Hook {
        fn default() -> Self {
            Self {
                mouse_hook: None,
                kbd_hook: None,
            }
        }
    }

    impl SystemInputHook for Win32Hook {
        fn start_capture<F>(&mut self, callback: F) -> Result<(), HalError>
        where
            F: Fn(InputEvent) + Send + 'static,
        {
            let cell = GLOBAL_CB.get_or_init(|| Arc::new(Mutex::new(None)));
            *cell.lock().unwrap() = Some(Box::new(callback));
            unsafe {
                let mh = SetWindowsHookExW(WH_MOUSE_LL, Some(low_level_mouse_proc), std::ptr::null_mut(), 0);
                let kh = SetWindowsHookExW(WH_KEYBOARD_LL, Some(low_level_kbd_proc), std::ptr::null_mut(), 0);
                if mh.is_null() || kh.is_null() {
                    if !mh.is_null() {
                        UnhookWindowsHookEx(mh);
                    }
                    if !kh.is_null() {
                        UnhookWindowsHookEx(kh);
                    }
                    return Err(HalError::DeviceOpen("SetWindowsHookExW failed".into()));
                }
                self.mouse_hook = Some(mh);
                self.kbd_hook = Some(kh);
            }
            Ok(())
        }

        fn stop_capture(&mut self) {
            unsafe {
                if let Some(mh) = self.mouse_hook.take() {
                    UnhookWindowsHookEx(mh);
                }
                if let Some(kh) = self.kbd_hook.take() {
                    UnhookWindowsHookEx(kh);
                }
            }
        }
    }

    pub struct Win32Injector;

    impl Win32Injector {
        fn send_one(input: INPUT) -> Result<(), HalError> {
            let sent = unsafe { SendInput(1, &input, size_of::<INPUT>() as i32) };
            if sent == 1 {
                Ok(())
            } else {
                Err(HalError::Injection("Win32 SendInput blocked or failed".into()))
            }
        }
    }

    impl SystemInputInjector for Win32Injector {
        fn inject(&self, event: InputEvent) -> Result<(), HalError> {
            let input = match event {
                InputEvent::MouseMove { dx, dy } => INPUT {
                    r#type: INPUT_MOUSE,
                    Anonymous: INPUT_0 {
                        mi: MOUSEINPUT {
                            dx,
                            dy,
                            mouseData: 0,
                            dwFlags: MOUSEEVENTF_MOVE,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                },
                InputEvent::MouseMoveAbsolute { x, y } => INPUT {
                    r#type: INPUT_MOUSE,
                    Anonymous: INPUT_0 {
                        mi: MOUSEINPUT {
                            dx: x as i32,
                            dy: y as i32,
                            mouseData: 0,
                            dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                },
                InputEvent::MouseButton { button, pressed } => {
                    let dw_flags = match (button, pressed) {
                        (0, true) => MOUSEEVENTF_LEFTDOWN,
                        (0, false) => MOUSEEVENTF_LEFTUP,
                        (1, true) => MOUSEEVENTF_RIGHTDOWN,
                        (1, false) => MOUSEEVENTF_RIGHTUP,
                        (2, true) => MOUSEEVENTF_MIDDLEDOWN,
                        (2, false) => MOUSEEVENTF_MIDDLEUP,
                        _ => return Err(HalError::Unsupported("unknown mouse button")),
                    };
                    INPUT {
                        r#type: INPUT_MOUSE,
                        Anonymous: INPUT_0 {
                            mi: MOUSEINPUT {
                                dx: 0,
                                dy: 0,
                                mouseData: 0,
                                dwFlags: dw_flags,
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    }
                }
                InputEvent::MouseWheel { dx, dy } => {
                    let (dw_flags, data) = if dy != 0 {
                        (MOUSEEVENTF_WHEEL, (dy as i32 * 120) as u32)
                    } else {
                        (MOUSEEVENTF_HWHEEL, (dx as i32 * 120) as u32)
                    };
                    INPUT {
                        r#type: INPUT_MOUSE,
                        Anonymous: INPUT_0 {
                            mi: MOUSEINPUT {
                                dx: 0,
                                dy: 0,
                                mouseData: data,
                                dwFlags: dw_flags,
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    }
                }
                InputEvent::Key { keycode, pressed } => INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: 0,
                            wScan: keycode as u16,
                            dwFlags: if pressed {
                                KEYEVENTF_SCANCODE
                            } else {
                                KEYEVENTF_SCANCODE | KEYEVENTF_KEYUP
                            },
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                },
            };
            Self::send_one(input)
        }
    }
}

#[cfg(target_os = "windows")]
pub use win32_impl::{Win32Hook, Win32Injector};

#[cfg(not(target_os = "windows"))]
pub struct Win32Injector;

#[cfg(not(target_os = "windows"))]
impl SystemInputInjector for Win32Injector {
    fn inject(&self, _event: InputEvent) -> Result<(), HalError> {
        Err(HalError::Unsupported("Win32 SendInput is Windows-only"))
    }
}
