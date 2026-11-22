use kvm_omni_core::protocol::OmniPacket;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::net::UdpSocket;

#[derive(Error, Debug)]
pub enum WebRtcTransportError {
    #[error("network error: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol error: {0}")]
    Protocol(#[from] kvm_omni_core::protocol::ProtocolError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportKind {
    QuicLan,
    WebRtcDataChannel,
    BleBridge,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataChannelProfile {
    pub label: String,
    pub ordered: bool,
    pub max_retransmits: Option<u16>,
}

impl DataChannelProfile {
    pub fn hid_realtime() -> Self {
        Self {
            label: "omnikvm-hid".into(),
            ordered: false,
            max_retransmits: Some(0),
        }
    }

    pub fn reliable_sync() -> Self {
        Self {
            label: "omnikvm-sync".into(),
            ordered: true,
            max_retransmits: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IceCandidate {
    pub candidate: String,
    pub sdp_mid: String,
    pub sdp_mline_index: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RtcSignal {
    Offer { sdp: String },
    Answer { sdp: String },
    Candidate(IceCandidate),
}

/// Manages sub-50ms failover between primary QUIC and secondary WebRTC/UDP DataChannel.
#[derive(Debug)]
pub struct FailoverCoordinator {
    pub active: TransportKind,
    pub failover_threshold: Duration,
    last_primary_ack: Instant,
    last_switch: Instant,
    pub switch_count: u64,
}

impl FailoverCoordinator {
    pub fn new(failover_threshold: Duration) -> Self {
        let now = Instant::now();
        Self {
            active: TransportKind::QuicLan,
            failover_threshold,
            last_primary_ack: now,
            last_switch: now,
            switch_count: 0,
        }
    }

    pub fn record_primary_ack(&mut self) {
        self.last_primary_ack = Instant::now();
        if self.active != TransportKind::QuicLan {
            self.active = TransportKind::QuicLan;
            self.last_switch = Instant::now();
            self.switch_count += 1;
        }
    }

    /// Returns true if a failover occurred on this tick.
    pub fn evaluate_health(&mut self, now: Instant) -> bool {
        if self.active == TransportKind::QuicLan
            && now.duration_since(self.last_primary_ack) > self.failover_threshold
        {
            self.active = TransportKind::WebRtcDataChannel;
            self.last_switch = now;
            self.switch_count += 1;
            return true;
        }
        false
    }
}

/// Lightweight P2P UDP DataChannel shim for LAN/ICE-direct packet delivery.
pub struct P2pDataChannel {
    socket: UdpSocket,
    remote: SocketAddr,
    pub profile: DataChannelProfile,
}

impl P2pDataChannel {
    pub async fn bind(
        local: SocketAddr,
        remote: SocketAddr,
        profile: DataChannelProfile,
    ) -> Result<Self, WebRtcTransportError> {
        let socket = UdpSocket::bind(local).await?;
        Ok(Self {
            socket,
            remote,
            profile,
        })
    }

    pub fn local_addr(&self) -> Result<SocketAddr, WebRtcTransportError> {
        Ok(self.socket.local_addr()?)
    }

    pub async fn send_packet(&self, pkt: &OmniPacket) -> Result<(), WebRtcTransportError> {
        let raw = pkt.serialize();
        self.socket.send_to(&raw, self.remote).await?;
        Ok(())
    }

    pub async fn recv_packet(&self) -> Result<OmniPacket, WebRtcTransportError> {
        let mut buf = [0u8; 4096];
        let (n, _) = self.socket.recv_from(&mut buf).await?;
        Ok(OmniPacket::deserialize(&buf[..n])?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kvm_omni_core::protocol::{flags, MouseMoveDeltaPayload, PayloadType};

    #[test]
    fn failover_triggers_under_50ms() {
        let mut coord = FailoverCoordinator::new(Duration::from_millis(35));
        assert_eq!(coord.active, TransportKind::QuicLan);
        let future = Instant::now() + Duration::from_millis(40);
        assert!(coord.evaluate_health(future));
        assert_eq!(coord.active, TransportKind::WebRtcDataChannel);
        coord.record_primary_ack();
        assert_eq!(coord.active, TransportKind::QuicLan);
        assert_eq!(coord.switch_count, 2);
    }

    #[tokio::test]
    async fn datachannel_loopback_packets() {
        let s1 = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let a1 = s1.local_addr().unwrap();
        drop(s1);

        let rx_sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let rx_addr = rx_sock.local_addr().unwrap();

        let tx = P2pDataChannel::bind(a1, rx_addr, DataChannelProfile::hid_realtime())
            .await
            .unwrap();
        let pkt = OmniPacket::from_typed(
            flags::URGENT_HID,
            2,
            10,
            1000,
            PayloadType::MouseMoveDelta,
            &MouseMoveDeltaPayload { dx: 5, dy: 5 },
        )
        .unwrap();
        tx.send_packet(&pkt).await.unwrap();

        let mut buf = [0u8; 512];
        let (n, _) = rx_sock.recv_from(&mut buf).await.unwrap();
        let decoded = OmniPacket::deserialize(&buf[..n]).unwrap();
        assert_eq!(decoded, pkt);
    }
}
