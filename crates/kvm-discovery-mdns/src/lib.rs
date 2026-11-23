use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::net::UdpSocket;

pub const SERVICE_TYPE: &str = "_omnikvm._udp.local.";
pub const DEFAULT_DISCOVERY_PORT: u16 = 53539;

#[derive(Error, Debug)]
pub enum DiscoveryError {
    #[error("network IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerAnnouncement {
    pub peer_id: String,
    pub hostname: String,
    pub os: String,
    pub quic_port: u16,
    pub webrtc_port: u16,
    pub fingerprint_hex: String,
    pub timestamp_ms: u64,
}

impl PeerAnnouncement {
    pub fn new(
        peer_id: impl Into<String>,
        hostname: impl Into<String>,
        quic_port: u16,
        webrtc_port: u16,
        fingerprint_hex: impl Into<String>,
    ) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        Self {
            peer_id: peer_id.into(),
            hostname: hostname.into(),
            os: std::env::consts::OS.to_string(),
            quic_port,
            webrtc_port,
            fingerprint_hex: fingerprint_hex.into(),
            timestamp_ms: now,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiscoveredPeer {
    pub announcement: PeerAnnouncement,
    pub addr: SocketAddr,
    pub last_seen_ms: u64,
}

#[derive(Debug, Default)]
pub struct PeerRegistry {
    peers: HashMap<String, DiscoveredPeer>,
}

impl PeerRegistry {
    pub fn upsert(&mut self, announcement: PeerAnnouncement, addr: SocketAddr) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.peers.insert(
            announcement.peer_id.clone(),
            DiscoveredPeer {
                announcement,
                addr,
                last_seen_ms: now,
            },
        );
    }

    pub fn prune_stale(&mut self, ttl: Duration) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let ttl_ms = ttl.as_millis() as u64;
        self.peers
            .retain(|_, p| now.saturating_sub(p.last_seen_ms) <= ttl_ms);
    }

    pub fn list(&self) -> Vec<&DiscoveredPeer> {
        self.peers.values().collect()
    }
}

pub const MDNS_MULTICAST_V4: std::net::Ipv4Addr = std::net::Ipv4Addr::new(224, 0, 0, 251);

pub async fn bind_multicast_listener(port: u16) -> Result<UdpSocket, DiscoveryError> {
    let socket = UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], port))).await?;
    let _ = socket.join_multicast_v4(MDNS_MULTICAST_V4, std::net::Ipv4Addr::UNSPECIFIED);
    Ok(socket)
}

pub async fn send_announcement(
    socket: &UdpSocket,
    target: SocketAddr,
    announcement: &PeerAnnouncement,
) -> Result<(), DiscoveryError> {
    let payload = serde_json::to_vec(announcement)?;
    socket.send_to(&payload, target).await?;
    Ok(())
}

pub async fn recv_announcement(
    socket: &UdpSocket,
) -> Result<(PeerAnnouncement, SocketAddr), DiscoveryError> {
    let mut buf = [0u8; 2048];
    let (len, addr) = socket.recv_from(&mut buf).await?;
    let announcement: PeerAnnouncement = serde_json::from_slice(&buf[..len])?;
    Ok((announcement, addr))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_udp_discovery_roundtrip() {
        let receiver = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let recv_addr = receiver.local_addr().unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();

        let ann = PeerAnnouncement::new("node-a", "workstation", 4433, 4434, "deadbeef");
        send_announcement(&sender, recv_addr, &ann).await.unwrap();

        let (got, _from) = recv_announcement(&receiver).await.unwrap();
        assert_eq!(got.peer_id, "node-a");
        assert_eq!(got.quic_port, 4433);
    }

    #[tokio::test]
    async fn peer_registry_upserts_and_prunes() {
        let mut reg = PeerRegistry::default();
        let ann = PeerAnnouncement::new("node-b", "laptop", 4433, 4434, "cafebabe");
        reg.upsert(ann, "127.0.0.1:4433".parse().unwrap());
        assert_eq!(reg.list().len(), 1);
        reg.prune_stale(Duration::from_secs(60));
        assert_eq!(reg.list().len(), 1);
    }
}
