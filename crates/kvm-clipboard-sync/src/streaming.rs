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

pub fn chunk_bytes(transfer_id: u64, data: &[u8], chunk_size: usize) -> (Vec<FileChunk>, [u8; 32]) {
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
            .map(|(i, slice)| FileChunk {
                transfer_id,
                index: i as u64,
                total_chunks,
                data: slice.to_vec(),
            })
            .collect()
    };

    (chunks, digest)
}

pub struct ChunkReassembler {
    expected_index: u64,
    hasher: blake3::Hasher,
    output: Vec<u8>,
}

impl Default for ChunkReassembler {
    fn default() -> Self {
        Self::new()
    }
}

impl ChunkReassembler {
    pub fn new() -> Self {
        Self {
            expected_index: 0,
            hasher: blake3::Hasher::new(),
            output: Vec::new(),
        }
    }

    pub fn push_chunk(&mut self, chunk: FileChunk) -> Result<(), StreamError> {
        if chunk.index != self.expected_index {
            return Err(StreamError::OutOfOrderChunk {
                expected: self.expected_index,
                got: chunk.index,
            });
        }
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

    #[test]
    fn semantic_detection_works() {
        assert_eq!(
            classify_clipboard_bytes(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]),
            ClipboardFormat::PngImage
        );
        assert_eq!(
            classify_clipboard_bytes(b"fn main() { println!(\"hi\"); }"),
            ClipboardFormat::CodeSnippet {
                language_hint: "rust".into()
            }
        );
    }

    #[test]
    fn stream_100mb_buffer_blake3_verified() {
        let size = 100 * 1024 * 1024; // 100 MB
        let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        let (chunks, digest) = chunk_bytes(1, &data, DEFAULT_CHUNK_SIZE);
        let mut reassembler = ChunkReassembler::new();
        for c in chunks {
            reassembler.push_chunk(c).unwrap();
        }
        let restored = reassembler.finalize_and_verify(&digest).unwrap();
        assert_eq!(restored.len(), size);
    }
}
