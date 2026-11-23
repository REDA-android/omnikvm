use kvm_input_hal::mock::MockHal;
use kvm_input_hal::{InputEvent, SystemInputInjector};
use kvm_omni_core::protocol::{
    flags, KeyEventPayload, MouseButtonPayload, MouseMoveAbsolutePayload, MouseMoveDeltaPayload,
    MouseWheelPayload, OmniPacket, PayloadType,
};
use std::sync::Arc;

pub fn event_to_packet(seq: u32, ev: InputEvent) -> Option<OmniPacket> {
    match ev {
        InputEvent::MouseMove { dx, dy } => OmniPacket::from_typed(
            flags::URGENT_HID,
            1,
            seq,
            0,
            PayloadType::MouseMoveDelta,
            &MouseMoveDeltaPayload {
                dx: dx.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
                dy: dy.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
            },
        )
        .ok(),
        InputEvent::MouseMoveAbsolute { x, y } => OmniPacket::from_typed(
            flags::URGENT_HID,
            1,
            seq,
            0,
            PayloadType::MouseMoveAbsolute,
            &MouseMoveAbsolutePayload { x, y },
        )
        .ok(),
        InputEvent::MouseButton { button, pressed } => OmniPacket::from_typed(
            flags::URGENT_HID,
            1,
            seq,
            0,
            PayloadType::MouseButton,
            &MouseButtonPayload { button, pressed },
        )
        .ok(),
        InputEvent::MouseWheel { dx, dy } => OmniPacket::from_typed(
            flags::URGENT_HID,
            1,
            seq,
            0,
            PayloadType::MouseWheel,
            &MouseWheelPayload { dx, dy },
        )
        .ok(),
        InputEvent::Key { keycode, pressed } => OmniPacket::from_typed(
            flags::URGENT_HID,
            1,
            seq,
            0,
            PayloadType::KeyEvent,
            &KeyEventPayload {
                scan_code: keycode,
                key_state: u8::from(pressed),
                layout_modifiers: 0,
            },
        )
        .ok(),
    }
}

pub fn packet_to_event(pkt: &OmniPacket) -> Option<InputEvent> {
    match pkt.payload_type {
        PayloadType::MouseMoveDelta => pkt
            .decode_payload::<MouseMoveDeltaPayload>()
            .ok()
            .map(|p| InputEvent::MouseMove {
                dx: p.dx as i32,
                dy: p.dy as i32,
            }),
        PayloadType::MouseMoveAbsolute => pkt
            .decode_payload::<MouseMoveAbsolutePayload>()
            .ok()
            .map(|p| InputEvent::MouseMoveAbsolute { x: p.x, y: p.y }),
        PayloadType::MouseButton => pkt
            .decode_payload::<MouseButtonPayload>()
            .ok()
            .map(|p| InputEvent::MouseButton {
                button: p.button,
                pressed: p.pressed,
            }),
        PayloadType::MouseWheel => pkt
            .decode_payload::<MouseWheelPayload>()
            .ok()
            .map(|p| InputEvent::MouseWheel { dx: p.dx, dy: p.dy }),
        PayloadType::KeyEvent => pkt
            .decode_payload::<KeyEventPayload>()
            .ok()
            .map(|p| InputEvent::Key {
                keycode: p.scan_code,
                pressed: p.key_state != 0,
            }),
        _ => None,
    }
}

pub fn make_injector(mock: bool) -> Arc<dyn SystemInputInjector> {
    if mock {
        return Arc::new(MockHal::new());
    }
    #[cfg(windows)]
    {
        Arc::new(kvm_input_hal::windows::Win32Injector)
    }
    #[cfg(not(windows))]
    {
        match kvm_input_hal::linux::UinputDevice::open_default() {
            Ok(d) => Arc::new(d),
            Err(_) => Arc::new(MockHal::new()),
        }
    }
}
