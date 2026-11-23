use thiserror::Error;

pub const ESP_FRAME_SYNC: [u8; 2] = [0xAA, 0x55];

#[derive(Error, Debug, PartialEq, Eq)]
pub enum Esp32Error {
    #[error("invalid sync header")]
    InvalidSync,
    #[error("frame too short")]
    TooShort,
    #[error("checksum mismatch")]
    ChecksumMismatch,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EspHidCommand {
    MouseRel = 0x01,
    KeyboardBoot = 0x02,
    ConsumerKey = 0x03,
    Ping = 0xFE,
}

/// Encodes an esparrier-compatible USB-CDC / BLE serial HID frame:
/// `[0xAA, 0x55, cmd, len, payload..., crc32c_be(4)]`
pub fn encode_esp_frame(cmd: EspHidCommand, payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(8 + payload.len());
    buf.extend_from_slice(&ESP_FRAME_SYNC);
    buf.push(cmd as u8);
    buf.push(payload.len() as u8);
    buf.extend_from_slice(payload);
    let crc = crc32c::crc32c(&buf);
    buf.extend_from_slice(&crc.to_be_bytes());
    buf
}

pub fn decode_esp_frame(raw: &[u8]) -> Result<(u8, Vec<u8>), Esp32Error> {
    if raw.len() < 8 {
        return Err(Esp32Error::TooShort);
    }
    if raw[0..2] != ESP_FRAME_SYNC {
        return Err(Esp32Error::InvalidSync);
    }
    let cmd = raw[2];
    let len = raw[3] as usize;
    if raw.len() < 4 + len + 4 {
        return Err(Esp32Error::TooShort);
    }
    let expected = u32::from_be_bytes(raw[4 + len..8 + len].try_into().unwrap());
    let actual = crc32c::crc32c(&raw[0..4 + len]);
    if expected != actual {
        return Err(Esp32Error::ChecksumMismatch);
    }
    Ok((cmd, raw[4..4 + len].to_vec()))
}

#[cfg(test)]
extern "C" {
    fn omnikvm_esp_dispatch_frame(buf: *const u8, len: usize) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn esp32_frame_roundtrip_and_c_dispatch() {
        let frame = encode_esp_frame(EspHidCommand::MouseRel, &[0x01, 0x0A, 0xFE, 0x00]);
        let (cmd, payload) = decode_esp_frame(&frame).unwrap();
        assert_eq!(cmd, EspHidCommand::MouseRel as u8);
        assert_eq!(payload, vec![0x01, 0x0A, 0xFE, 0x00]);

        let rc = unsafe { omnikvm_esp_dispatch_frame(frame.as_ptr(), frame.len()) };
        assert_eq!(rc, 0);
    }
}
