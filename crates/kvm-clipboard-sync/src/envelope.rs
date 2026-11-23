use crate::semantic::{classify_clipboard_bytes, ClipboardFormat};
use crate::streaming::{xor_keystream_inplace, StreamError};
use serde::{Deserialize, Serialize};

/// Semantic encrypted clipboard envelope for cross-OS clipboard synchronization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardEnvelope {
    pub transfer_id: u64,
    pub format: ClipboardFormat,
    pub blake3_digest: [u8; 32],
    pub ciphertext: Vec<u8>,
}

impl ClipboardEnvelope {
    pub fn seal(transfer_id: u64, plaintext: &[u8], session_key: &[u8; 32]) -> Self {
        let format = classify_clipboard_bytes(plaintext);
        let blake3_digest = *blake3::hash(plaintext).as_bytes();
        let mut ciphertext = plaintext.to_vec();
        xor_keystream_inplace(session_key, transfer_id, 0, &mut ciphertext);
        Self {
            transfer_id,
            format,
            blake3_digest,
            ciphertext,
        }
    }

    pub fn open(&self, session_key: &[u8; 32]) -> Result<Vec<u8>, StreamError> {
        let mut buf = self.ciphertext.clone();
        xor_keystream_inplace(session_key, self.transfer_id, 0, &mut buf);
        let actual = *blake3::hash(&buf).as_bytes();
        if actual != self.blake3_digest {
            return Err(StreamError::HashMismatch);
        }
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_envelope_seal_open_and_tamper_detection() {
        let key = [0x99u8; 32];
        let code = b"func main() { fmt.Println(\"omnikvm\") }";
        let env = ClipboardEnvelope::seal(42, code, &key);
        assert_eq!(
            env.format,
            ClipboardFormat::CodeSnippet {
                language_hint: "go".into()
            }
        );
        assert_ne!(env.ciphertext.as_slice(), code);
        let plain = env.open(&key).unwrap();
        assert_eq!(plain.as_slice(), code);

        let mut tampered = env.clone();
        tampered.ciphertext[0] ^= 0xFF;
        assert_eq!(tampered.open(&key), Err(StreamError::HashMismatch));
    }
}
