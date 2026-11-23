use crate::{HalError, InputEvent, SystemInputInjector};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct Win32HookControl {
    remote_active: Arc<AtomicBool>,
    cursor_locked: Arc<AtomicBool>,
}

impl Win32HookControl {
    pub fn is_remote_active(&self) -> bool {
        self.remote_active.load(Ordering::Relaxed)
    }

    pub fn set_remote_active(&self, active: bool) {
        self.remote_active.store(active, Ordering::SeqCst);
    }

    pub fn set_cursor_locked(&self, locked: bool) {
        self.cursor_locked.store(locked, Ordering::SeqCst);
    }
}

#[cfg(target_os = "windows")]
mod win32_impl {
    use super::*;
    use crate::edge_kvm::{EdgeSwitchState, MoveDecision, ScreenEdge};
    use crate::win_keymap::{evdev_to_win_scancode, win_scancode_to_evdev};
    use crate::SystemInputHook;
    use std::mem::size_of;
    use std::sync::atomic::AtomicU32;
    use std::sync::{Mutex, OnceLock};
    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
        KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL,
        MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
        MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL,
        MOUSEINPUT, SendInput,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetCursorPos, GetMessageW, GetSystemMetrics,
        KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT,
        PostThreadMessageW, SM_CXSCREEN, SM_CYSCREEN, SetCursorPos, SetWindowsHookExW,
        TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP,
        WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL,
        WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    struct HookRuntime {
        cb: Option<Box<dyn Fn(InputEvent) + Send + 'static>>,
        edge_state: EdgeSwitchState,
        ctrl: Win32HookControl,
        suppress_warp: bool,
    }

    static RUNTIME: OnceLock<Mutex<HookRuntime>> = OnceLock::new();

    fn runtime() -> &'static Mutex<HookRuntime> {
        RUNTIME.get_or_init(|| {
            Mutex::new(HookRuntime {
                cb: None,
                edge_state: EdgeSwitchState::new(ScreenEdge::Right, 1920, 1080),
                ctrl: Win32HookControl::default(),
                suppress_warp: false,
            })
        })
    }

    unsafe extern "system" fn low_level_mouse_proc(
        n_code: i32,
        w_param: WPARAM,
        l_param: LPARAM,
    ) -> LRESULT {
        if n_code >= 0 {
            let info = &*(l_param as *const MSLLHOOKSTRUCT);
            // Never capture synthetic SendInput events (prevents loopback feedback).
            if (info.flags & (LLMHF_INJECTED | 0x02)) != 0 {
                return CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param);
            }
            if let Ok(mut rt) = runtime().lock() {
                if rt.suppress_warp && w_param as u32 == WM_MOUSEMOVE {
                    rt.suppress_warp = false;
                    return CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param);
                }
                rt.edge_state.cursor_locked = rt.ctrl.cursor_locked.load(Ordering::Relaxed);
                let ctrl_active = rt.ctrl.remote_active.load(Ordering::Relaxed);
                if rt.edge_state.edge != ScreenEdge::Immediate {
                    rt.edge_state.remote_active = ctrl_active;
                }

                match w_param as u32 {
                    WM_MOUSEMOVE => {
                        let mut cur = POINT { x: info.pt.x, y: info.pt.y };
                        let _ = GetCursorPos(&mut cur);
                        match rt.edge_state.on_mouse_move(info.pt.x, info.pt.y, cur.x, cur.y) {
                            MoveDecision::PassLocal => {}
                            MoveDecision::EnterRemote { park_x, park_y } => {
                                rt.ctrl.set_remote_active(true);
                                rt.suppress_warp = true;
                                let _ = SetCursorPos(park_x, park_y);
                                return 1;
                            }
                            MoveDecision::SendRemoteDelta { dx, dy } => {
                                if let Some(cb) = rt.cb.as_ref() {
                                    cb(InputEvent::MouseMove { dx, dy });
                                }
                                return 1;
                            }
                            MoveDecision::SwallowNoop => return 1,
                            MoveDecision::ReturnLocal { local_x, local_y } => {
                                rt.ctrl.set_remote_active(false);
                                rt.suppress_warp = true;
                                let _ = SetCursorPos(local_x, local_y);
                                return 1;
                            }
                        }
                    }
                    msg if rt.edge_state.remote_active => {
                        let ev = match msg {
                            WM_LBUTTONDOWN => Some(InputEvent::MouseButton { button: 0, pressed: true }),
                            WM_LBUTTONUP => Some(InputEvent::MouseButton { button: 0, pressed: false }),
                            WM_RBUTTONDOWN => Some(InputEvent::MouseButton { button: 1, pressed: true }),
                            WM_RBUTTONUP => Some(InputEvent::MouseButton { button: 1, pressed: false }),
                            WM_MBUTTONDOWN => Some(InputEvent::MouseButton { button: 2, pressed: true }),
                            WM_MBUTTONUP => Some(InputEvent::MouseButton { button: 2, pressed: false }),
                            WM_MOUSEWHEEL => {
                                let d = ((info.mouseData >> 16) as i16) / 120;
                                (d != 0).then_some(InputEvent::MouseWheel { dx: 0, dy: d })
                            }
                            _ => None,
                        };
                        if let Some(ev) = ev {
                            if let Some(cb) = rt.cb.as_ref() {
                                cb(ev);
                            }
                            return 1;
                        }
                    }
                    _ => {}
                }
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
            if (info.flags & (LLKHF_INJECTED | 0x02)) != 0 {
                return CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param);
            }
            let msg = w_param as u32;
            let pressed = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let released = msg == WM_KEYUP || msg == WM_SYSKEYUP;
            if pressed || released {
                let ext = (info.flags & LLKHF_EXTENDED) != 0;
                let keycode = win_scancode_to_evdev(info.scanCode, ext);
                if let Ok(rt) = runtime().lock() {
                    if rt.edge_state.remote_active {
                        if let Some(cb) = rt.cb.as_ref() {
                            cb(InputEvent::Key { keycode, pressed });
                        }
                        return 1;
                    }
                }
            }
        }
        CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param)
    }

    pub struct Win32Hook {
        edge: ScreenEdge,
        ctrl: Win32HookControl,
        thread_id: Arc<AtomicU32>,
        worker: Option<std::thread::JoinHandle<()>>,
    }

    impl Default for Win32Hook {
        fn default() -> Self {
            Self::with_edge(ScreenEdge::Right)
        }
    }

    impl Win32Hook {
        pub fn with_edge(edge: ScreenEdge) -> Self {
            let ctrl = Win32HookControl::default();
            ctrl.set_remote_active(edge == ScreenEdge::Immediate);
            Self {
                edge,
                ctrl,
                thread_id: Arc::new(AtomicU32::new(0)),
                worker: None,
            }
        }

        pub fn control(&self) -> Win32HookControl {
            self.ctrl.clone()
        }
    }

    impl SystemInputHook for Win32Hook {
        fn start_capture<F>(&mut self, callback: F) -> Result<(), HalError>
        where
            F: Fn(InputEvent) + Send + 'static,
        {
            let (w, h) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
            {
                let mut rt = runtime().lock().unwrap();
                rt.cb = Some(Box::new(callback));
                rt.edge_state = EdgeSwitchState::new(self.edge, w, h);
                rt.ctrl = self.ctrl.clone();
                rt.suppress_warp = false;
            }
            let tid_slot = self.thread_id.clone();
            let (tx, rx) = std::sync::mpsc::sync_channel::<Result<(), HalError>>(1);
            self.worker = Some(std::thread::spawn(move || unsafe {
                tid_slot.store(GetCurrentThreadId(), Ordering::SeqCst);
                let mh = SetWindowsHookExW(WH_MOUSE_LL, Some(low_level_mouse_proc), std::ptr::null_mut(), 0);
                let kh = SetWindowsHookExW(WH_KEYBOARD_LL, Some(low_level_kbd_proc), std::ptr::null_mut(), 0);
                if mh.is_null() || kh.is_null() {
                    if !mh.is_null() { UnhookWindowsHookEx(mh); }
                    if !kh.is_null() { UnhookWindowsHookEx(kh); }
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
            rx.recv().map_err(|_| HalError::DeviceOpen("hook thread exited".into()))?
        }

        fn stop_capture(&mut self) {
            self.ctrl.set_remote_active(false);
            let tid = self.thread_id.load(Ordering::SeqCst);
            if tid != 0 {
                unsafe { PostThreadMessageW(tid, WM_QUIT, 0, 0); }
            }
            if let Some(h) = self.worker.take() {
                let _ = h.join();
            }
            if let Ok(mut rt) = runtime().lock() {
                rt.cb = None;
                rt.edge_state.remote_active = false;
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
                        mi: MOUSEINPUT { dx, dy, mouseData: 0, dwFlags: MOUSEEVENTF_MOVE, time: 0, dwExtraInfo: 0 },
                    },
                },
                InputEvent::MouseMoveAbsolute { x, y } => INPUT {
                    r#type: INPUT_MOUSE,
                    Anonymous: INPUT_0 {
                        mi: MOUSEINPUT {
                            dx: x as i32, dy: y as i32, mouseData: 0,
                            dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE, time: 0, dwExtraInfo: 0,
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
                            mi: MOUSEINPUT { dx: 0, dy: 0, mouseData: 0, dwFlags: dw_flags, time: 0, dwExtraInfo: 0 },
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
                            mi: MOUSEINPUT { dx: 0, dy: 0, mouseData: data, dwFlags: dw_flags, time: 0, dwExtraInfo: 0 },
                        },
                    }
                }
                InputEvent::Key { keycode, pressed } => {
                    let (w_scan, ext) = evdev_to_win_scancode(keycode);
                    let mut flags = KEYEVENTF_SCANCODE;
                    if ext {
                        flags |= KEYEVENTF_EXTENDEDKEY;
                    }
                    if !pressed {
                        flags |= KEYEVENTF_KEYUP;
                    }
                    INPUT {
                        r#type: INPUT_KEYBOARD,
                        Anonymous: INPUT_0 {
                            ki: KEYBDINPUT { wVk: 0, wScan: w_scan, dwFlags: flags, time: 0, dwExtraInfo: 0 },
                        },
                    }
                }
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
