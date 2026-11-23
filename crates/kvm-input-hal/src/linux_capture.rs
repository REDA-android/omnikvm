//! Linux evdev capture half of the HAL: reads 24-byte `struct input_event` records from
//! `/dev/input/event*` (or any byte stream in tests), folds REL_X/REL_Y deltas per
//! `SYN_REPORT`, and emits [`InputEvent`]s. Optional `EVIOCGRAB` gives exclusive
//! access so local apps stop seeing input while the cursor lives on a remote screen.

use crate::{HalError, InputEvent, SystemInputHook};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_REL: u16 = 0x02;
const REL_X: u16 = 0x00;
const REL_Y: u16 = 0x01;
const REL_HWHEEL: u16 = 0x06;
const REL_WHEEL: u16 = 0x08;
const BTN_LEFT: u16 = 0x110;
const BTN_TASK: u16 = 0x117;
pub const EVENT_SIZE: usize = 24;

/// Stateful decoder from raw evdev records to OmniKVM [`InputEvent`]s.
#[derive(Debug, Default)]
pub struct EvdevDecoder {
    dx: i32,
    dy: i32,
}

impl EvdevDecoder {
    pub fn feed(&mut self, ev_type: u16, code: u16, value: i32) -> Vec<InputEvent> {
        let mut out = Vec::new();
        match (ev_type, code) {
            (EV_REL, REL_X) => self.dx += value,
            (EV_REL, REL_Y) => self.dy += value,
            (EV_REL, REL_WHEEL) => out.push(InputEvent::MouseWheel { dx: 0, dy: value as i16 }),
            (EV_REL, REL_HWHEEL) => out.push(InputEvent::MouseWheel { dx: value as i16, dy: 0 }),
            (EV_KEY, c) if (BTN_LEFT..=BTN_TASK).contains(&c) => out.push(InputEvent::MouseButton {
                button: (c - BTN_LEFT) as u8,
                pressed: value != 0,
            }),
            // value 2 == autorepeat: forwarded as a repeated press.
            (EV_KEY, c) => out.push(InputEvent::Key {
                keycode: c as u32,
                pressed: value != 0,
            }),
            (EV_SYN, _) => {
                if self.dx != 0 || self.dy != 0 {
                    out.push(InputEvent::MouseMove { dx: self.dx, dy: self.dy });
                }
                self.dx = 0;
                self.dy = 0;
            }
            _ => {}
        }
        out
    }

    /// Parses one native-endian 24-byte `struct input_event` record.
    pub fn feed_bytes(&mut self, rec: &[u8; EVENT_SIZE]) -> Vec<InputEvent> {
        let ev_type = u16::from_ne_bytes([rec[16], rec[17]]);
        let code = u16::from_ne_bytes([rec[18], rec[19]]);
        let value = i32::from_ne_bytes([rec[20], rec[21], rec[22], rec[23]]);
        self.feed(ev_type, code, value)
    }
}

pub struct EvdevCapture {
    path: PathBuf,
    grab: bool,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl EvdevCapture {
    pub fn new(path: impl AsRef<Path>, grab: bool) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            grab,
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
        }
    }

    /// Waits for the reader thread to finish (EOF or stop).
    pub fn join(&mut self) {
        if let Some(h) = self.worker.take() {
            let _ = h.join();
        }
    }
}

#[cfg(target_os = "linux")]
fn set_grab(file: &std::fs::File, on: bool) -> Result<(), HalError> {
    use std::os::unix::io::AsRawFd;
    const EVIOCGRAB: libc::c_ulong = 0x4004_4590;
    // SAFETY: fd valid for the lifetime of `file`; EVIOCGRAB takes an int by value.
    let rc = unsafe { libc::ioctl(file.as_raw_fd(), EVIOCGRAB, libc::c_int::from(on)) };
    if rc < 0 {
        return Err(HalError::DeviceOpen(format!("EVIOCGRAB: {}", std::io::Error::last_os_error())));
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn set_grab(_file: &std::fs::File, _on: bool) -> Result<(), HalError> {
    Err(HalError::Unsupported("EVIOCGRAB is Linux-only"))
}

impl SystemInputHook for EvdevCapture {
    fn start_capture<F>(&mut self, callback: F) -> Result<(), HalError>
    where
        F: Fn(InputEvent) + Send + 'static,
    {
        let mut file = std::fs::File::open(&self.path)
            .map_err(|e| HalError::DeviceOpen(format!("{}: {e}", self.path.display())))?;
        if self.grab {
            set_grab(&file, true)?;
        }
        self.stop.store(false, Ordering::SeqCst);
        let stop = self.stop.clone();
        let grab = self.grab;
        self.worker = Some(std::thread::spawn(move || {
            let mut dec = EvdevDecoder::default();
            let mut rec = [0u8; EVENT_SIZE];
            while !stop.load(Ordering::SeqCst) {
                if file.read_exact(&mut rec).is_err() {
                    break;
                }
                for ev in dec.feed_bytes(&rec) {
                    callback(ev);
                }
            }
            if grab {
                let _ = set_grab(&file, false);
            }
        }));
        Ok(())
    }

    /// Signals the reader to stop; it exits after the next record (devices emit
    /// SYN/MSC traffic continuously, so this is effectively immediate).
    fn stop_capture(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linux::RawInputEvent;
    use std::sync::Mutex;

    #[test]
    fn decoder_folds_rel_deltas_per_syn_report() {
        let mut d = EvdevDecoder::default();
        assert!(d.feed(EV_REL, REL_X, 3).is_empty());
        assert!(d.feed(EV_REL, REL_X, 2).is_empty());
        assert!(d.feed(EV_REL, REL_Y, -4).is_empty());
        assert_eq!(d.feed(EV_SYN, 0, 0), vec![InputEvent::MouseMove { dx: 5, dy: -4 }]);
        assert!(d.feed(EV_SYN, 0, 0).is_empty());
        assert_eq!(
            d.feed(EV_KEY, 0x111, 1),
            vec![InputEvent::MouseButton { button: 1, pressed: true }]
        );
        assert_eq!(d.feed(EV_REL, REL_WHEEL, -1), vec![InputEvent::MouseWheel { dx: 0, dy: -1 }]);
    }

    #[test]
    fn capture_reads_evdev_stream_from_file() {
        let path = std::env::temp_dir().join(format!("omnikvm_evdev_{}", std::process::id()));
        let mut bytes = Vec::new();
        for (t, c, v) in [(EV_REL, REL_X, 7), (EV_SYN, 0, 0), (EV_KEY, 30, 1), (EV_KEY, 30, 0)] {
            bytes.extend_from_slice(&RawInputEvent::new(t, c, v).to_bytes());
        }
        std::fs::write(&path, &bytes).unwrap();

        let got = Arc::new(Mutex::new(Vec::new()));
        let sink = got.clone();
        let mut cap = EvdevCapture::new(&path, false);
        cap.start_capture(move |ev| sink.lock().unwrap().push(ev)).unwrap();
        cap.join();
        let _ = std::fs::remove_file(&path);
        assert_eq!(
            *got.lock().unwrap(),
            vec![
                InputEvent::MouseMove { dx: 7, dy: 0 },
                InputEvent::Key { keycode: 30, pressed: true },
                InputEvent::Key { keycode: 30, pressed: false },
            ]
        );
    }
}
