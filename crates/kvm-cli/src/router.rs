use crate::daemon::event_to_packet;
use anyhow::Result;
use kvm_clipboard_sync::semantic::{classify_clipboard_bytes, ClipboardFormat};
use kvm_clipboard_sync::streaming::{chunk_bytes_encrypted, DEFAULT_CHUNK_SIZE};
use kvm_input_hal::InputEvent;
use kvm_omni_core::topology::{ScreenTopology, SwitchTransition};
use kvm_transport_quic::Connection;
use kvm_transport_webrtc::{FailoverCoordinator, P2pDataChannel, TransportKind};
use std::time::{Duration, Instant};

/// Unified multi-screen session router coordinating ScreenTopology,
/// primary QUIC datagrams, and <50ms failover to WebRTC/UDP DataChannel.
pub struct OmniSessionRouter {
    pub topology: ScreenTopology,
    pub active_screen: String,
    pub cursor_x: i32,
    pub cursor_y: i32,
    pub failover: FailoverCoordinator,
    seq: u32,
}

impl OmniSessionRouter {
    pub fn new(topology: ScreenTopology, initial_screen: impl Into<String>) -> Self {
        Self {
            topology,
            active_screen: initial_screen.into(),
            cursor_x: 100,
            cursor_y: 100,
            failover: FailoverCoordinator::new(Duration::from_millis(35)),
            seq: 0,
        }
    }

    /// Updates virtual cursor coordinates and returns a transition if an edge was crossed.
    pub fn apply_cursor_delta(&mut self, dx: i32, dy: i32) -> Option<SwitchTransition> {
        self.cursor_x += dx;
        self.cursor_y += dy;
        if let Some(sw) = self
            .topology
            .evaluate_move(&self.active_screen, self.cursor_x, self.cursor_y)
        {
            self.active_screen = sw.target_screen.clone();
            self.cursor_x = sw.entry_x;
            self.cursor_y = sw.entry_y;
            return Some(sw);
        }
        None
    }

    /// Routes an InputEvent over the currently active transport (QUIC primary or WebRTC fallback).
    pub async fn route_event(
        &mut self,
        event: InputEvent,
        quic_conn: Option<&Connection>,
        webrtc_fallback: Option<&P2pDataChannel>,
    ) -> Result<TransportKind> {
        self.seq = self.seq.wrapping_add(1);
        let pkt = event_to_packet(self.seq, event)?;

        if self.failover.active == TransportKind::QuicLan {
            if let Some(conn) = quic_conn {
                if kvm_transport_quic::send_hid_datagram(conn, &pkt).is_ok() {
                    self.failover.record_primary_ack();
                    return Ok(TransportKind::QuicLan);
                }
            }
            self.failover
                .evaluate_health(Instant::now() + Duration::from_millis(50));
        }

        if let Some(dc) = webrtc_fallback {
            dc.send_packet(&pkt).await?;
            return Ok(TransportKind::WebRtcDataChannel);
        }
        anyhow::bail!("no healthy transport available")
    }

    /// Classifies and encrypts a clipboard payload into Blake3-XOF chunks ready for P2P streaming.
    pub fn prepare_clipboard_sync(
        &self,
        transfer_id: u64,
        data: &[u8],
        session_key: &[u8; 32],
    ) -> (ClipboardFormat, usize, [u8; 32]) {
        let fmt = classify_clipboard_bytes(data);
        let (chunks, digest) = chunk_bytes_encrypted(transfer_id, data, DEFAULT_CHUNK_SIZE, session_key);
        (fmt, chunks.len(), digest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::run_server;
    use kvm_input_hal::mock::MockHal;
    use kvm_omni_core::topology::ScreenRect;
    use kvm_transport_quic::make_client_endpoint;
    use kvm_transport_webrtc::DataChannelProfile;
    use std::sync::Arc;
    use tokio::net::UdpSocket;

    #[tokio::test]
    async fn full_session_topology_failover_and_clipboard() {
        let mut topo = ScreenTopology::default();
        topo.add_screen(ScreenRect {
            id: "local-pc".into(),
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            dpi_scale: 1.0,
        });
        topo.add_screen(ScreenRect {
            id: "remote-pc".into(),
            x: 1920,
            y: 0,
            width: 1920,
            height: 1080,
            dpi_scale: 1.0,
        });

        let mut router = OmniSessionRouter::new(topo, "local-pc");
        let sw = router.apply_cursor_delta(1850, 400).unwrap();
        assert_eq!(sw.target_screen, "remote-pc");
        assert_eq!(router.active_screen, "remote-pc");

        // Primary QUIC delivery
        let hal = Arc::new(MockHal::new());
        let (quic_addr, server_handle) = run_server("127.0.0.1:0".parse().unwrap(), hal.clone(), Some(1))
            .await
            .unwrap();
        let client_ep = make_client_endpoint("127.0.0.1:0".parse().unwrap()).unwrap();
        let conn = client_ep.connect(quic_addr, "localhost").unwrap().await.unwrap();

        let used = router
            .route_event(InputEvent::MouseMove { dx: 7, dy: 3 }, Some(&conn), None)
            .await
            .unwrap();
        assert_eq!(used, TransportKind::QuicLan);
        tokio::time::sleep(Duration::from_millis(25)).await;
        conn.close(0u32.into(), b"failover-test");
        let _ = server_handle.await;

        // Trigger <50ms failover to WebRTC/UDP DataChannel
        assert!(router
            .failover
            .evaluate_health(Instant::now() + Duration::from_millis(40)));
        let rx_udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let rx_addr = rx_udp.local_addr().unwrap();
        let dc = P2pDataChannel::bind(
            "127.0.0.1:0".parse().unwrap(),
            rx_addr,
            DataChannelProfile::hid_realtime(),
        )
        .await
        .unwrap();

        let used_fb = router
            .route_event(InputEvent::Key { keycode: 30, pressed: true }, None, Some(&dc))
            .await
            .unwrap();
        assert_eq!(used_fb, TransportKind::WebRtcDataChannel);

        // Semantic encrypted clipboard preparation
        let (fmt, chunks, _digest) = router.prepare_clipboard_sync(
            1,
            b"fn hello() { println!(\"omnikvm\"); }",
            &[0x11u8; 32],
        );
        assert_eq!(
            fmt,
            ClipboardFormat::CodeSnippet {
                language_hint: "rust".into()
            }
        );
        assert_eq!(chunks, 1);
    }
}
