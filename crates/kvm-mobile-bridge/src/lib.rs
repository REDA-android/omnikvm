use serde::{Deserialize, Serialize};

/// Standard USB/BLE HID Boot Mouse + Keyboard Report Descriptor (Report ID 1 = Mouse, 2 = Keyboard).
pub const BLE_HID_REPORT_MAP: &[u8] = &[
    // Report ID 1: Mouse
    0x05, 0x01, // Usage Page (Generic Desktop)
    0x09, 0x02, // Usage (Mouse)
    0xA1, 0x01, // Collection (Application)
    0x85, 0x01, //   Report ID (1)
    0x09, 0x01, //   Usage (Pointer)
    0xA1, 0x00, //   Collection (Physical)
    0x05, 0x09, //     Usage Page (Buttons)
    0x19, 0x01, //     Usage Minimum (1)
    0x29, 0x03, //     Usage Maximum (3)
    0x15, 0x00, //     Logical Minimum (0)
    0x25, 0x01, //     Logical Maximum (1)
    0x95, 0x03, //     Report Count (3)
    0x75, 0x01, //     Report Size (1)
    0x81, 0x02, //     Input (Data, Variable, Absolute)
    0x95, 0x01, //     Report Count (1)
    0x75, 0x05, //     Report Size (5)
    0x81, 0x01, //     Input (Constant)
    0x05, 0x01, //     Usage Page (Generic Desktop)
    0x09, 0x30, //     Usage (X)
    0x09, 0x31, //     Usage (Y)
    0x09, 0x38, //     Usage (Wheel)
    0x15, 0x81, //     Logical Minimum (-127)
    0x25, 0x7F, //     Logical Maximum (127)
    0x75, 0x08, //     Report Size (8)
    0x95, 0x03, //     Report Count (3)
    0x81, 0x06, //     Input (Data, Variable, Relative)
    0xC0, //   End Collection
    0xC0, // End Collection
    // Report ID 2: Keyboard
    0x05, 0x01, // Usage Page (Generic Desktop)
    0x09, 0x06, // Usage (Keyboard)
    0xA1, 0x01, // Collection (Application)
    0x85, 0x02, //   Report ID (2)
    0x05, 0x07, //   Usage Page (Key Codes)
    0x19, 0xE0, //   Usage Minimum (224)
    0x29, 0xE7, //   Usage Maximum (231)
    0x15, 0x00, //   Logical Minimum (0)
    0x25, 0x01, //   Logical Maximum (1)
    0x75, 0x01, //   Report Size (1)
    0x95, 0x08, //   Report Count (8)
    0x81, 0x02, //   Input (Data, Variable, Absolute)
    0x95, 0x01, //   Report Count (1)
    0x75, 0x08, //   Report Size (8)
    0x81, 0x01, //   Input (Constant)
    0x95, 0x06, //   Report Count (6)
    0x75, 0x08, //   Report Size (8)
    0x15, 0x00, //   Logical Minimum (0)
    0x25, 0x65, //   Logical Maximum (101)
    0x05, 0x07, //   Usage Page (Key Codes)
    0x19, 0x00, //   Usage Minimum (0)
    0x29, 0x65, //   Usage Maximum (101)
    0x81, 0x00, //   Input (Data, Array)
    0xC0, // End Collection
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BleMouseReport {
    pub buttons: u8,
    pub dx: i8,
    pub dy: i8,
    pub wheel: i8,
}

impl BleMouseReport {
    pub fn to_gatt_frame(&self) -> [u8; 5] {
        [
            0x01,
            self.buttons & 0x07,
            self.dx as u8,
            self.dy as u8,
            self.wheel as u8,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BleKeyboardReport {
    pub modifiers: u8,
    pub keys: [u8; 6],
}

impl BleKeyboardReport {
    pub fn to_gatt_frame(&self) -> [u8; 9] {
        [
            0x02,
            self.modifiers,
            0x00,
            self.keys[0],
            self.keys[1],
            self.keys[2],
            self.keys[3],
            self.keys[4],
            self.keys[5],
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedBleHidFrame {
    Mouse(BleMouseReport),
    Keyboard(BleKeyboardReport),
}

/// Test/verification GATT HID controller that validates incoming BLE HID frames.
#[derive(Debug, Default)]
pub struct BleHidTestController {
    pub accepted_frames: Vec<DecodedBleHidFrame>,
}

impl BleHidTestController {
    pub fn accept_gatt_frame(&mut self, frame: &[u8]) -> Result<DecodedBleHidFrame, &'static str> {
        match frame.first().copied() {
            Some(0x01) if frame.len() == 5 => {
                let r = BleMouseReport {
                    buttons: frame[1] & 0x07,
                    dx: frame[2] as i8,
                    dy: frame[3] as i8,
                    wheel: frame[4] as i8,
                };
                self.accepted_frames.push(DecodedBleHidFrame::Mouse(r));
                Ok(DecodedBleHidFrame::Mouse(r))
            }
            Some(0x02) if frame.len() == 9 => {
                let r = BleKeyboardReport {
                    modifiers: frame[1],
                    keys: [frame[3], frame[4], frame[5], frame[6], frame[7], frame[8]],
                };
                self.accepted_frames.push(DecodedBleHidFrame::Keyboard(r));
                Ok(DecodedBleHidFrame::Keyboard(r))
            }
            _ => Err("invalid BLE HID GATT frame"),
        }
    }
}

/// Scrcpy-compatible Android control message encoder (type 0 = Keycode, 2 = Touch, 3 = Scroll).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrcpyControlMsg {
    InjectKeycode {
        action: u8,
        keycode: u32,
        repeat: u32,
        metastate: u32,
    },
    InjectTouch {
        action: u8,
        pointer_id: u64,
        x: i32,
        y: i32,
        screen_w: u16,
        screen_h: u16,
        pressure: u16,
        buttons: u32,
    },
    InjectScroll {
        x: i32,
        y: i32,
        screen_w: u16,
        screen_h: u16,
        hscroll: i16,
        vscroll: i16,
        buttons: u32,
    },
}

impl ScrcpyControlMsg {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(32);
        match *self {
            Self::InjectKeycode {
                action,
                keycode,
                repeat,
                metastate,
            } => {
                out.push(0x00);
                out.push(action);
                out.extend_from_slice(&keycode.to_be_bytes());
                out.extend_from_slice(&repeat.to_be_bytes());
                out.extend_from_slice(&metastate.to_be_bytes());
            }
            Self::InjectTouch {
                action,
                pointer_id,
                x,
                y,
                screen_w,
                screen_h,
                pressure,
                buttons,
            } => {
                out.push(0x02);
                out.push(action);
                out.extend_from_slice(&pointer_id.to_be_bytes());
                out.extend_from_slice(&x.to_be_bytes());
                out.extend_from_slice(&y.to_be_bytes());
                out.extend_from_slice(&screen_w.to_be_bytes());
                out.extend_from_slice(&screen_h.to_be_bytes());
                out.extend_from_slice(&pressure.to_be_bytes());
                out.extend_from_slice(&0u32.to_be_bytes());
                out.extend_from_slice(&buttons.to_be_bytes());
            }
            Self::InjectScroll {
                x,
                y,
                screen_w,
                screen_h,
                hscroll,
                vscroll,
                buttons,
            } => {
                out.push(0x03);
                out.extend_from_slice(&x.to_be_bytes());
                out.extend_from_slice(&y.to_be_bytes());
                out.extend_from_slice(&screen_w.to_be_bytes());
                out.extend_from_slice(&screen_h.to_be_bytes());
                out.extend_from_slice(&hscroll.to_be_bytes());
                out.extend_from_slice(&vscroll.to_be_bytes());
                out.extend_from_slice(&buttons.to_be_bytes());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ble_hid_frame_and_scrcpy_encoding() {
        let mut controller = BleHidTestController::default();

        let mouse = BleMouseReport {
            buttons: 0x01,
            dx: -10,
            dy: 20,
            wheel: 0,
        };
        assert_eq!(
            controller.accept_gatt_frame(&mouse.to_gatt_frame()).unwrap(),
            DecodedBleHidFrame::Mouse(mouse)
        );

        let kbd = BleKeyboardReport {
            modifiers: 0x02,
            keys: [0x04, 0, 0, 0, 0, 0],
        };
        assert_eq!(
            controller.accept_gatt_frame(&kbd.to_gatt_frame()).unwrap(),
            DecodedBleHidFrame::Keyboard(kbd)
        );
        assert_eq!(controller.accepted_frames.len(), 2);

        let touch = ScrcpyControlMsg::InjectTouch {
            action: 2,
            pointer_id: 1,
            x: 540,
            y: 960,
            screen_w: 1080,
            screen_h: 2400,
            pressure: 0xFFFF,
            buttons: 1,
        };
        let encoded = touch.encode();
        assert_eq!(encoded[0], 0x02);
        assert_eq!(encoded.len(), 32);

        let scroll = ScrcpyControlMsg::InjectScroll {
            x: 540,
            y: 960,
            screen_w: 1080,
            screen_h: 2400,
            hscroll: 0,
            vscroll: 1,
            buttons: 0,
        };
        assert_eq!(scroll.encode()[0], 0x03);
    }
}
