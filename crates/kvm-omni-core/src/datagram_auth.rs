//! Session-keyed datagram authentication + anti-replay window.
//!
//! After the SPAKE2+/TOFU handshake both peers share a 32-byte session key.
//! Every HID datagram is sealed as `frame || seq_be(8) || tag(16)` where the tag is a
//! keyed Blake3 MAC over `seq || frame`. The receiver verifies the tag in constant time
//! and rejects duplicated / too-old sequence numbers with a 64-entry sliding window
//! (RFC 6479 style), so captured datagrams cannot be replayed on the LAN.

use thiserror::Error;

pub const TAG_LEN: usize = 16;
pub const SEQ_LEN: usize = 8;
pub const OVERHEAD: usize = TAG_LEN + SEQ_LEN;
const WINDOW: u64 = 64;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum DatagramAuthError {
    #[error("sealed datagram too short")]
    TooShort,
    #[error("datagram authentication tag mismatch")]
    BadTag,
    #[error("replayed or stale datagram (seq {0})")]
    Replay(u64),
}

fn derive_mac_key(session_key: &[u8; 32]) -> [u8; 32] {
    blake3::derive_key("omnikvm v1 datagram mac", session_key)
}

fn compute_tag(mac_key: &[u8; 32], seq: u64, frame: &[u8]) -> [u8; TAG_LEN] {
    let mut h = blake3::Hasher::new_keyed(mac_key);
    h.update(&seq.to_be_bytes());
    h.update(frame);
    let mut out = [0u8; TAG_LEN];
    out.copy_from_slice(&h.finalize().as_bytes()[..TAG_LEN]);
    out
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Sender half: monotonically increasing sequence + MAC.
pub struct DatagramSealer {
    mac_key: [u8; 32],
    next_seq: u64,
}

impl DatagramSealer {
    pub fn new(session_key: &[u8; 32]) -> Self {
        Self {
            mac_key: derive_mac_key(session_key),
            next_seq: 1,
        }
    }

    pub fn seal(&mut self, frame: &[u8]) -> Vec<u8> {
        let seq = self.next_seq;
        self.next_seq += 1;
        let mut out = Vec::with_capacity(frame.len() + OVERHEAD);
        out.extend_from_slice(frame);
        out.extend_from_slice(&seq.to_be_bytes());
        out.extend_from_slice(&compute_tag(&self.mac_key, seq, frame));
        out
    }
}

/// Receiver half: tag verification + 64-wide sliding replay window.
pub struct DatagramOpener {
    mac_key: [u8; 32],
    highest: u64,
    bitmap: u64,
}

impl DatagramOpener {
    pub fn new(session_key: &[u8; 32]) -> Self {
        Self {
            mac_key: derive_mac_key(session_key),
            highest: 0,
            bitmap: 0,
        }
    }

    /// Verifies and returns the inner OmniKVM frame slice.
    pub fn open<'a>(&mut self, sealed: &'a [u8]) -> Result<&'a [u8], DatagramAuthError> {
        if sealed.len() < OVERHEAD {
            return Err(DatagramAuthError::TooShort);
        }
        let frame_len = sealed.len() - OVERHEAD;
        let (frame, rest) = sealed.split_at(frame_len);
        let mut seq_b = [0u8; SEQ_LEN];
        seq_b.copy_from_slice(&rest[..SEQ_LEN]);
        let seq = u64::from_be_bytes(seq_b);
        let tag = &rest[SEQ_LEN..];
        if !ct_eq(tag, &compute_tag(&self.mac_key, seq, frame)) {
            return Err(DatagramAuthError::BadTag);
        }
        self.check_and_update(seq)?;
        Ok(frame)
    }

    fn check_and_update(&mut self, seq: u64) -> Result<(), DatagramAuthError> {
        if seq == 0 {
            return Err(DatagramAuthError::Replay(seq));
        }
        if seq > self.highest {
            let shift = seq - self.highest;
            self.bitmap = if shift >= WINDOW { 0 } else { self.bitmap << shift };
            self.bitmap |= 1;
            self.highest = seq;
            return Ok(());
        }
        let offset = self.highest - seq;
        if offset >= WINDOW || (self.bitmap >> offset) & 1 == 1 {
            return Err(DatagramAuthError::Replay(seq));
        }
        self.bitmap |= 1 << offset;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_roundtrip_and_tamper_detection() {
        let key = [7u8; 32];
        let mut s = DatagramSealer::new(&key);
        let mut o = DatagramOpener::new(&key);
        let sealed = s.seal(b"hello-hid");
        assert_eq!(o.open(&sealed).unwrap(), b"hello-hid");

        let mut bad = s.seal(b"move");
        bad[0] ^= 0x01;
        assert_eq!(o.open(&bad), Err(DatagramAuthError::BadTag));

        let mut wrong_key = DatagramOpener::new(&[8u8; 32]);
        assert_eq!(wrong_key.open(&s.seal(b"x")), Err(DatagramAuthError::BadTag));
        assert_eq!(o.open(&[0u8; 4]), Err(DatagramAuthError::TooShort));
    }

    #[test]
    fn replay_window_rejects_duplicates_and_stale_but_allows_reorder() {
        let key = [1u8; 32];
        let mut s = DatagramSealer::new(&key);
        let mut o = DatagramOpener::new(&key);
        let pkts: Vec<Vec<u8>> = (0..100).map(|i| s.seal(&[i as u8])).collect();

        assert!(o.open(&pkts[1]).is_ok()); // seq 2 first
        assert!(o.open(&pkts[0]).is_ok()); // seq 1 reordered: OK
        assert!(matches!(o.open(&pkts[0]), Err(DatagramAuthError::Replay(1))));
        assert!(o.open(&pkts[99]).is_ok()); // jump to seq 100
        assert!(matches!(o.open(&pkts[2]), Err(DatagramAuthError::Replay(3)))); // too old
        assert!(o.open(&pkts[60]).is_ok()); // seq 61 within window
        assert!(o.open(&pkts[60]).is_err());
    }
}
