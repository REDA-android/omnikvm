use serde::{Deserialize, Serialize};

/// Standard USB/BLE HID Boot Mouse + Keyboard Report Descriptor (Report ID 1 = Mouse, 2 = Keyboard).
pub const BLE_HID_REPORT_MAP: &[u8] = &[
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

/// Scrcpy-compatible Android control message encoder (type 2 = InjectTouchEvent).
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
                out.push(0x00); // SC_CONTROL_MSG_TYPE_INJECT_KEYCODE
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
                out.push(0x02); // SC_CONTROL_MSG_TYPE_INJECT_TOUCH_EVENT
                out.push(action);
                out.extend_from_slice(&pointer_id.to_be_bytes());
                out.extend_from_slice(&x.to_be_bytes());
                out.extend_from_slice(&y.to_be_bytes());
                out.extend_from_slice(&screen_w.to_be_bytes());
                out.extend_from_slice(&screen_h.to_be_bytes());
                out.extend_from_slice(&pressure.to_be_bytes());
                out.extend_from_slice(&0u32.to_be_bytes()); // action_button
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
        let report = BleMouseReport {
            buttons: 0x01,
            dx: -10,
            dy: 20,
            wheel: 0,
        };
        let frame = report.to_gatt_frame();
        assert_eq!(frame[0], 0x01);
        assert_eq!(frame[1], 0x01);
        assert_eq!(frame[2] as i8, -10);
        assert_eq!(frame[3] as i8, 20);

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
    }
}
