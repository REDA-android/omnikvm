use serde::{Deserialize, Serialize};
use std::convert::TryInto;
use thiserror::Error;

pub const PROTOCOL_MAGIC: [u8; 2] = [0x4F, 0x4B]; // 'O', 'K'
pub const PROTOCOL_VERSION: u8 = 0x01;
pub const HEADER_LEN: usize = 23;
pub const TRAILER_LEN: usize = 4;
pub const MIN_PACKET_LEN: usize = HEADER_LEN + TRAILER_LEN;

pub mod flags {
    pub const NONE: u8 = 0x00;
    pub const URGENT_HID: u8 = 0x01;
    pub const ENCRYPTED: u8 = 0x02;
    pub const COMPRESSED: u8 = 0x04;
}

#[derive(Error, Debug, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("invalid magic bytes")]
    InvalidMagic,
    #[error("unsupported protocol version: {0}")]
    UnsupportedVersion(u8),
    #[error("unknown payload type: 0x{0:02x}")]
    UnknownPayloadType(u8),
    #[error("CRC32C checksum mismatch")]
    InvalidChecksum,
    #[error("buffer too short: need at least {0} bytes")]
    BufferTooShort(usize),
    #[error("bincode serialization error: {0}")]
    CodecError(String),
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PayloadType {
    MouseMoveDelta = 0x01,
    MouseMoveAbsolute = 0x02,
    MouseButton = 0x03,
    MouseWheel = 0x04,
    KeyEvent = 0x05,
    ClipboardChunk = 0x10,
    FileStreamChunk = 0x20,
    Heartbeat = 0xFF,
}

impl TryFrom<u8> for PayloadType {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(Self::MouseMoveDelta),
            0x02 => Ok(Self::MouseMoveAbsolute),
            0x03 => Ok(Self::MouseButton),
            0x04 => Ok(Self::MouseWheel),
            0x05 => Ok(Self::KeyEvent),
            0x10 => Ok(Self::ClipboardChunk),
            0x20 => Ok(Self::FileStreamChunk),
            0xFF => Ok(Self::Heartbeat),
            other => Err(ProtocolError::UnknownPayloadType(other)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MouseMoveDeltaPayload {
    pub dx: i16,
    pub dy: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MouseMoveAbsolutePayload {
    pub x: u16,
    pub y: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MouseButtonPayload {
    pub button: u8,
    pub pressed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MouseWheelPayload {
    pub dx: i16,
    pub dy: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyEventPayload {
    pub scan_code: u32,
    pub key_state: u8, // 0 = Up, 1 = Down, 2 = Repeat
    pub layout_modifiers: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatPayload {
    pub sent_at_us: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OmniPacket {
    pub flags: u8,
    pub stream_id: u32,
    pub sequence: u32,
    pub timestamp_us: u64,
    pub payload_type: PayloadType,
    pub payload: Vec<u8>,
}

impl OmniPacket {
    pub fn new(
        flags: u8,
        stream_id: u32,
        sequence: u32,
        timestamp_us: u64,
        payload_type: PayloadType,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            flags,
            stream_id,
            sequence,
            timestamp_us,
            payload_type,
            payload,
        }
    }

    pub fn from_typed<T: Serialize>(
        flags: u8,
        stream_id: u32,
        sequence: u32,
        timestamp_us: u64,
        payload_type: PayloadType,
        value: &T,
    ) -> Result<Self, ProtocolError> {
        let payload =
            bincode::serialize(value).map_err(|e| ProtocolError::CodecError(e.to_string()))?;
        Ok(Self::new(
            flags,
            stream_id,
            sequence,
            timestamp_us,
            payload_type,
            payload,
        ))
    }

    pub fn decode_payload<T: for<'de> Deserialize<'de>>(&self) -> Result<T, ProtocolError> {
        bincode::deserialize(&self.payload).map_err(|e| ProtocolError::CodecError(e.to_string()))
    }

    /// Encode directly into a caller-supplied buffer to avoid heap allocations on hot paths.
    pub fn encode_into(&self, buf: &mut Vec<u8>) {
        buf.clear();
        buf.reserve(MIN_PACKET_LEN + self.payload.len());
        buf.extend_from_slice(&PROTOCOL_MAGIC);
        buf.push(PROTOCOL_VERSION);
        buf.push(self.flags);
        buf.extend_from_slice(&self.stream_id.to_be_bytes());
        buf.extend_from_slice(&self.sequence.to_be_bytes());
        buf.extend_from_slice(&self.timestamp_us.to_be_bytes());
        buf.push(self.payload_type as u8);
        buf.extend_from_slice(&(self.payload.len() as u16).to_be_bytes());
        buf.extend_from_slice(&self.payload);

        let crc = crc32c::crc32c(buf);
        buf.extend_from_slice(&crc.to_be_bytes());
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(MIN_PACKET_LEN + self.payload.len());
        self.encode_into(&mut buf);
        buf
    }

    pub fn deserialize(src: &[u8]) -> Result<Self, ProtocolError> {
        if src.len() < MIN_PACKET_LEN {
            return Err(ProtocolError::BufferTooShort(MIN_PACKET_LEN));
        }
        if src[0..2] != PROTOCOL_MAGIC {
            return Err(ProtocolError::InvalidMagic);
        }
        if src[2] != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion(src[2]));
        }

        let flags = src[3];
        let stream_id = u32::from_be_bytes(src[4..8].try_into().unwrap());
        let sequence = u32::from_be_bytes(src[8..12].try_into().unwrap());
        let timestamp_us = u64::from_be_bytes(src[12..20].try_into().unwrap());
        let payload_type = PayloadType::try_from(src[20])?;
        let payload_len = u16::from_be_bytes(src[21..23].try_into().unwrap()) as usize;

        let total_len = HEADER_LEN + payload_len + TRAILER_LEN;
        if src.len() < total_len {
            return Err(ProtocolError::BufferTooShort(total_len));
        }

        let expected_crc =
            u32::from_be_bytes(src[HEADER_LEN + payload_len..total_len].try_into().unwrap());
        let actual_crc = crc32c::crc32c(&src[0..HEADER_LEN + payload_len]);
        if expected_crc != actual_crc {
            return Err(ProtocolError::InvalidChecksum);
        }

        let payload = src[HEADER_LEN..HEADER_LEN + payload_len].to_vec();
        Ok(Self {
            flags,
            stream_id,
            sequence,
            timestamp_us,
            payload_type,
            payload,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn roundtrip_mouse_delta() {
        let pkt = OmniPacket::from_typed(
            flags::URGENT_HID,
            1,
            42,
            1_234_567,
            PayloadType::MouseMoveDelta,
            &MouseMoveDeltaPayload { dx: -12, dy: 8 },
        )
        .unwrap();

        let bytes = pkt.serialize();
        let decoded = OmniPacket::deserialize(&bytes).unwrap();
        assert_eq!(pkt, decoded);
        let payload: MouseMoveDeltaPayload = decoded.decode_payload().unwrap();
        assert_eq!(payload, MouseMoveDeltaPayload { dx: -12, dy: 8 });
    }

    #[test]
    fn rejects_corrupted_crc() {
        let pkt = OmniPacket::new(flags::NONE, 0, 1, 99, PayloadType::Heartbeat, vec![1, 2, 3]);
        let mut bytes = pkt.serialize();
        bytes[10] ^= 0xFF;
        assert_eq!(
            OmniPacket::deserialize(&bytes),
            Err(ProtocolError::InvalidChecksum)
        );
    }

    #[test]
    fn perf_million_roundtrips() {
        let pkt = OmniPacket::from_typed(
            flags::URGENT_HID,
            7,
            999,
            5_000_000,
            PayloadType::MouseMoveDelta,
            &MouseMoveDeltaPayload { dx: 4, dy: -3 },
        )
        .unwrap();
        let mut buf = Vec::with_capacity(64);
        let start = Instant::now();
        for _ in 0..1_000_000 {
            pkt.encode_into(&mut buf);
            let parsed = OmniPacket::deserialize(&buf).unwrap();
            std::hint::black_box(parsed);
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_secs_f64() < 5.0,
            "1M roundtrips took {:?}",
            elapsed
        );
    }
}
