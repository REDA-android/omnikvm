use crate::{HalError, InputEvent, SystemInputInjector};

#[cfg(target_os = "windows")]
mod win32_impl {
    use super::*;
    use crate::SystemInputHook;
    use std::mem::size_of;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};
    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP,
        KEYEVENTF_SCANCODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN,
        MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
        MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT,
        PostThreadMessageW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx,
        WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP,
        WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN,
        WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    type HookCallback = Arc<Mutex<Option<Box<dyn Fn(InputEvent) + Send + 'static>>>>;
    static GLOBAL_CB: OnceLock<HookCallback> = OnceLock::new();
    static LAST_PT: OnceLock<Mutex<Option<(i32, i32)>>> = OnceLock::new();

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
                WM_MOUSEMOVE => {
                    let pt_cell = LAST_PT.get_or_init(|| Mutex::new(None));
                    if let Ok(mut last) = pt_cell.lock() {
                        let cur = (info.pt.x, info.pt.y);
                        if let Some((lx, ly)) = *last {
                            let dx = cur.0 - lx;
                            let dy = cur.1 - ly;
                            if dx != 0 || dy != 0 {
                                dispatch_event(InputEvent::MouseMove { dx, dy });
                            }
                        }
                        *last = Some(cur);
                    }
                }
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
                WM_MBUTTONDOWN => {
                    dispatch_event(InputEvent::MouseButton { button: 2, pressed: true })
                }
                WM_MBUTTONUP => {
                    dispatch_event(InputEvent::MouseButton { button: 2, pressed: false })
                }
                WM_MOUSEWHEEL => {
                    let delta = ((info.mouseData >> 16) as i16) / 120;
                    if delta != 0 {
                        dispatch_event(InputEvent::MouseWheel { dx: 0, dy: delta });
                    }
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
            let msg = w_param as u32;
            let pressed = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let released = msg == WM_KEYUP || msg == WM_SYSKEYUP;
            if pressed || released {
                dispatch_event(InputEvent::Key {
                    keycode: info.scanCode,
                    pressed,
                });
            }
        }
        CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param)
    }

    pub struct Win32Hook {
        thread_id: Arc<AtomicU32>,
        worker: Option<std::thread::JoinHandle<()>>,
    }

    impl Default for Win32Hook {
        fn default() -> Self {
            Self {
                thread_id: Arc::new(AtomicU32::new(0)),
                worker: None,
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
            if let Ok(mut pt) = LAST_PT.get_or_init(|| Mutex::new(None)).lock() {
                *pt = None;
            }
            let tid_slot = self.thread_id.clone();
            let (tx, rx) = std::sync::mpsc::sync_channel::<Result<(), HalError>>(1);
            self.worker = Some(std::thread::spawn(move || unsafe {
                let tid = GetCurrentThreadId();
                tid_slot.store(tid, Ordering::SeqCst);
                let mh = SetWindowsHookExW(WH_MOUSE_LL, Some(low_level_mouse_proc), std::ptr::null_mut(), 0);
                let kh = SetWindowsHookExW(WH_KEYBOARD_LL, Some(low_level_kbd_proc), std::ptr::null_mut(), 0);
                if mh.is_null() || kh.is_null() {
                    if !mh.is_null() {
                        UnhookWindowsHookEx(mh);
                    }
                    if !kh.is_null() {
                        UnhookWindowsHookEx(kh);
                    }
                    let _ = tx.send(Err(HalError::DeviceOpen("SetWindowsHookExW failed".into())));
                    return;
                }
                let _ = tx.send(Ok(()));
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                UnhookWindowsHookEx(mh);
                UnhookWindowsHookEx(kh);
                tid_slot.store(0, Ordering::SeqCst);
            }));
            rx.recv()
                .map_err(|_| HalError::DeviceOpen("hook thread exited".into()))?
        }

        fn stop_capture(&mut self) {
            let tid = self.thread_id.load(Ordering::SeqCst);
            if tid != 0 {
                unsafe {
                    PostThreadMessageW(tid, WM_QUIT, 0, 0);
                }
            }
            if let Some(h) = self.worker.take() {
                let _ = h.join();
            }
            if let Some(cell) = GLOBAL_CB.get() {
                if let Ok(mut g) = cell.lock() {
                    *g = None;
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
