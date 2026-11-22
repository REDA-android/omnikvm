use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const DEFAULT_CHUNK_SIZE: usize = 64 * 1024; // 64 KB

#[derive(Error, Debug, PartialEq, Eq)]
pub enum StreamError {
    #[error("unexpected chunk index {got}, expected {expected}")]
    OutOfOrderChunk { expected: u64, got: u64 },
    #[error("blake3 hash mismatch")]
    HashMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChunk {
    pub transfer_id: u64,
    pub index: u64,
    pub total_chunks: u64,
    pub data: Vec<u8>,
}

/// Derives a 64 KB keystream block using Blake3 XOF keyed mode and XORs in-place.
pub fn xor_keystream_inplace(key: &[u8; 32], transfer_id: u64, index: u64, buf: &mut [u8]) {
    let mut hasher = blake3::Hasher::new_keyed(key);
    hasher.update(&transfer_id.to_be_bytes());
    hasher.update(&index.to_be_bytes());
    let mut reader = hasher.finalize_xof();
    let mut pad = [0u8; 4096];
    for chunk in buf.chunks_mut(pad.len()) {
        reader.fill(&mut pad[..chunk.len()]);
        for (b, k) in chunk.iter_mut().zip(pad.iter()) {
            *b ^= *k;
        }
    }
}

pub fn chunk_bytes_encrypted(
    transfer_id: u64,
    data: &[u8],
    chunk_size: usize,
    key: &[u8; 32],
) -> (Vec<FileChunk>, [u8; 32]) {
    let digest = *blake3::hash(data).as_bytes();
    let cs = chunk_size.max(1);
    let total_chunks = if data.is_empty() {
        1
    } else {
        data.len().div_ceil(cs) as u64
    };

    let chunks = if data.is_empty() {
        vec![FileChunk {
            transfer_id,
            index: 0,
            total_chunks: 1,
            data: Vec::new(),
        }]
    } else {
        data.chunks(cs)
            .enumerate()
            .map(|(i, slice)| {
                let mut enc = slice.to_vec();
                xor_keystream_inplace(key, transfer_id, i as u64, &mut enc);
                FileChunk {
                    transfer_id,
                    index: i as u64,
                    total_chunks,
                    data: enc,
                }
            })
            .collect()
    };

    (chunks, digest)
}

pub struct ChunkReassembler {
    expected_index: u64,
    key: [u8; 32],
    hasher: blake3::Hasher,
    output: Vec<u8>,
}

impl ChunkReassembler {
    pub fn new(key: [u8; 32], capacity_hint: usize) -> Self {
        Self {
            expected_index: 0,
            key,
            hasher: blake3::Hasher::new(),
            output: Vec::with_capacity(capacity_hint),
        }
    }

    pub fn push_chunk(&mut self, mut chunk: FileChunk) -> Result<(), StreamError> {
        if chunk.index != self.expected_index {
            return Err(StreamError::OutOfOrderChunk {
                expected: self.expected_index,
                got: chunk.index,
            });
        }
        xor_keystream_inplace(&self.key, chunk.transfer_id, chunk.index, &mut chunk.data);
        self.hasher.update(&chunk.data);
        self.output.extend_from_slice(&chunk.data);
        self.expected_index += 1;
        Ok(())
    }

    pub fn finalize_and_verify(self, expected_blake3: &[u8; 32]) -> Result<Vec<u8>, StreamError> {
        let actual = *self.hasher.finalize().as_bytes();
        if &actual != expected_blake3 {
            return Err(StreamError::HashMismatch);
        }
        Ok(self.output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic::{classify_clipboard_bytes, ClipboardFormat};
    use std::time::Instant;

    #[test]
    fn semantic_detection_works() {
        assert_eq!(
            classify_clipboard_bytes(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]),
            ClipboardFormat::PngImage
        );
        assert_eq!(
            classify_clipboard_bytes(b"<html><body><div>hi</div></body></html>"),
            ClipboardFormat::HtmlRichText
        );
        assert_eq!(
            classify_clipboard_bytes(b"fn main() { println!(\"hi\"); }"),
            ClipboardFormat::CodeSnippet {
                language_hint: "rust".into()
            }
        );
    }

    #[test]
    fn stream_500mb_encrypted_blake3_verified() {
        let size = 500 * 1024 * 1024; // 500 MB
        let data = vec![0xA5u8; size];
        let key = [0x42u8; 32];
        let expected_digest = *blake3::hash(&data).as_bytes();

        let start = Instant::now();
        let mut reassembler = ChunkReassembler::new(key, size);
        let total_chunks = data.len().div_ceil(DEFAULT_CHUNK_SIZE) as u64;
        for (i, slice) in data.chunks(DEFAULT_CHUNK_SIZE).enumerate() {
            let mut enc = slice.to_vec();
            xor_keystream_inplace(&key, 1, i as u64, &mut enc);
            reassembler
                .push_chunk(FileChunk {
                    transfer_id: 1,
                    index: i as u64,
                    total_chunks,
                    data: enc,
                })
                .unwrap();
        }
        let restored = reassembler.finalize_and_verify(&expected_digest).unwrap();
        println!("500 MB encrypted stream + Blake3 verified in {:?}", start.elapsed());
        assert_eq!(restored.len(), size);
    }
}
