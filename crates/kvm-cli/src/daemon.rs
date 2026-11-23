use anyhow::Result;
use kvm_input_hal::{InputEvent, SystemInputInjector};
use kvm_omni_core::protocol::{
    flags, KeyEventPayload, MouseButtonPayload, MouseMoveAbsolutePayload, MouseMoveDeltaPayload,
    MouseWheelPayload, OmniPacket, PayloadType,
};
use kvm_transport_quic::{
    make_client_endpoint, make_server_endpoint, recv_hid_datagram, send_hid_datagram,
};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

pub fn event_to_packet(seq: u32, ev: InputEvent) -> Result<OmniPacket> {
    let ts = now_us();
    let pkt = match ev {
        InputEvent::MouseMove { dx, dy } => OmniPacket::from_typed(
            flags::URGENT_HID,
            1,
            seq,
            ts,
            PayloadType::MouseMoveDelta,
            &MouseMoveDeltaPayload {
                dx: dx.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
                dy: dy.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
            },
        )?,
        InputEvent::MouseMoveAbsolute { x, y } => OmniPacket::from_typed(
            flags::URGENT_HID,
            1,
            seq,
            ts,
            PayloadType::MouseMoveAbsolute,
            &MouseMoveAbsolutePayload { x, y },
        )?,
        InputEvent::MouseButton { button, pressed } => OmniPacket::from_typed(
            flags::URGENT_HID,
            1,
            seq,
            ts,
            PayloadType::MouseButton,
            &MouseButtonPayload { button, pressed },
        )?,
        InputEvent::MouseWheel { dx, dy } => OmniPacket::from_typed(
            flags::URGENT_HID,
            1,
            seq,
            ts,
            PayloadType::MouseWheel,
            &MouseWheelPayload { dx, dy },
        )?,
        InputEvent::Key { keycode, pressed } => OmniPacket::from_typed(
            flags::URGENT_HID,
            1,
            seq,
            ts,
            PayloadType::KeyEvent,
            &KeyEventPayload {
                scan_code: keycode,
                key_state: u8::from(pressed),
                layout_modifiers: 0,
            },
        )?,
    };
    Ok(pkt)
}

pub fn packet_to_event(pkt: &OmniPacket) -> Result<Option<InputEvent>> {
    let ev = match pkt.payload_type {
        PayloadType::MouseMoveDelta => {
            let p: MouseMoveDeltaPayload = pkt.decode_payload()?;
            Some(InputEvent::MouseMove {
                dx: p.dx as i32,
                dy: p.dy as i32,
            })
        }
        PayloadType::MouseMoveAbsolute => {
            let p: MouseMoveAbsolutePayload = pkt.decode_payload()?;
            Some(InputEvent::MouseMoveAbsolute { x: p.x, y: p.y })
        }
        PayloadType::MouseButton => {
            let p: MouseButtonPayload = pkt.decode_payload()?;
            Some(InputEvent::MouseButton {
                button: p.button,
                pressed: p.pressed,
            })
        }
        PayloadType::MouseWheel => {
            let p: MouseWheelPayload = pkt.decode_payload()?;
            Some(InputEvent::MouseWheel { dx: p.dx, dy: p.dy })
        }
        PayloadType::KeyEvent => {
            let p: KeyEventPayload = pkt.decode_payload()?;
            Some(InputEvent::Key {
                keycode: p.scan_code,
                pressed: p.key_state != 0,
            })
        }
        _ => None,
    };
    Ok(ev)
}

pub async fn run_server(
    bind_addr: SocketAddr,
    injector: Arc<dyn SystemInputInjector>,
    max_events: Option<usize>,
) -> Result<(SocketAddr, tokio::task::JoinHandle<Result<usize>>)> {
    let (endpoint, fp) = make_server_endpoint(bind_addr)?;
    let local_addr = endpoint.local_addr()?;
    println!("OmniKVM QUIC Server listening on {local_addr} (TOFU fp: {:02x?})", &fp[..4]);

    let handle = tokio::spawn(async move {
        let mut count = 0usize;
        if let Some(incoming) = endpoint.accept().await {
            let conn = incoming.await?;
            loop {
                match recv_hid_datagram(&conn).await {
                    Ok(pkt) => {
                        if let Some(ev) = packet_to_event(&pkt)? {
                            injector.inject(ev)?;
                            count += 1;
                            if max_events.is_some_and(|m| count >= m) {
                                break;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        }
        Ok(count)
    });

    Ok((local_addr, handle))
}

pub async fn send_events_to_peer(remote_addr: SocketAddr, events: &[InputEvent]) -> Result<usize> {
    let endpoint = make_client_endpoint("0.0.0.0:0".parse()?)?;
    let conn = endpoint.connect(remote_addr, "localhost")?.await?;
    for (idx, &ev) in events.iter().enumerate() {
        let pkt = event_to_packet(idx as u32, ev)?;
        send_hid_datagram(&conn, &pkt)?;
    }
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    conn.close(0u32.into(), b"done");
    Ok(events.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kvm_input_hal::mock::MockHal;

    #[tokio::test]
    async fn e2e_quic_hid_loopback() {
        let hal = Arc::new(MockHal::new());
        let events = vec![
            InputEvent::MouseMove { dx: 15, dy: -8 },
            InputEvent::MouseButton {
                button: 0,
                pressed: true,
            },
            InputEvent::Key {
                keycode: 30,
                pressed: true,
            },
            InputEvent::Key {
                keycode: 30,
                pressed: false,
            },
        ];

        let (addr, server_handle) = run_server("127.0.0.1:0".parse().unwrap(), hal.clone(), Some(events.len()))
            .await
            .unwrap();

        let sent = send_events_to_peer(addr, &events).await.unwrap();
        assert_eq!(sent, events.len());

        let received = tokio::time::timeout(std::time::Duration::from_secs(3), server_handle)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(received, events.len());
        assert_eq!(hal.take_events(), events);
    }
}
