use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::HashMap;
use thiserror::Error;

type HmacSha256 = Hmac<Sha256>;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum CryptoError {
    #[error("pairing verification failed")]
    VerificationFailed,
    #[error("TOFU fingerprint mismatch for peer {peer_id}")]
    FingerprintMismatch { peer_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingChallenge {
    pub peer_id: String,
    pub nonce: [u8; 32],
    pub pub_fingerprint: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingProof {
    pub peer_id: String,
    pub mac: [u8; 32],
}

pub fn generate_pairing_pin() -> String {
    let mut bytes = [0u8; 4];
    rand::thread_rng().fill_bytes(&mut bytes);
    let num = u32::from_be_bytes(bytes) % 1_000_000;
    format!("{num:06}")
}

pub fn build_pairing_uri(peer_id: &str, host: &str, port: u16, pin: &str) -> String {
    format!("omnikvm://pair?id={peer_id}&host={host}&port={port}&pin={pin}")
}

pub fn derive_session_key(pin: &str, client_nonce: &[u8; 32], server_nonce: &[u8; 32]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_derive_key("omnikvm-spake2-session-v1");
    hasher.update(pin.as_bytes());
    hasher.update(client_nonce);
    hasher.update(server_nonce);
    *hasher.finalize().as_bytes()
}

pub fn compute_pairing_proof(session_key: &[u8; 32], peer_id: &str, challenge: &[u8; 32]) -> [u8; 32] {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(session_key).expect("32-byte key");
    mac.update(peer_id.as_bytes());
    mac.update(challenge);
    let result = mac.finalize().into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

pub fn verify_pairing_proof(
    session_key: &[u8; 32],
    peer_id: &str,
    challenge: &[u8; 32],
    proof: &[u8; 32],
) -> Result<(), CryptoError> {
    let expected = compute_pairing_proof(session_key, peer_id, challenge);
    if constant_time_eq(&expected, proof) {
        Ok(())
    } else {
        Err(CryptoError::VerificationFailed)
    }
}

fn constant_time_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    let mut diff = 0u8;
    for i in 0..32 {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

#[derive(Debug, Default)]
pub struct TofuKeystore {
    trusted: HashMap<String, [u8; 32]>,
}

impl TofuKeystore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn verify_or_trust(&mut self, peer_id: &str, fingerprint: [u8; 32]) -> Result<bool, CryptoError> {
        if let Some(existing) = self.trusted.get(peer_id) {
            if existing == &fingerprint {
                Ok(false)
            } else {
                Err(CryptoError::FingerprintMismatch {
                    peer_id: peer_id.to_string(),
                })
            }
        } else {
            self.trusted.insert(peer_id.to_string(), fingerprint);
            Ok(true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_and_tofu_work() {
        let pin = generate_pairing_pin();
        assert_eq!(pin.len(), 6);
        let c_nonce = [1u8; 32];
        let s_nonce = [2u8; 32];
        let key = derive_session_key(&pin, &c_nonce, &s_nonce);
        let proof = compute_pairing_proof(&key, "laptop-1", &s_nonce);
        verify_pairing_proof(&key, "laptop-1", &s_nonce, &proof).unwrap();

        let mut ks = TofuKeystore::new();
        assert!(ks.verify_or_trust("laptop-1", [9u8; 32]).unwrap());
        assert!(!ks.verify_or_trust("laptop-1", [9u8; 32]).unwrap());
        assert!(ks.verify_or_trust("laptop-1", [8u8; 32]).is_err());
    }
}
