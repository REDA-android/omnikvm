use snow::{Builder, TransportState};
use thiserror::Error;

/// Exact Noise Protocol suite specified in OmniKVM Section 4.4.
pub const NOISE_PATTERN_IK: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2s";
pub const NOISE_PATTERN_XX: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";

#[derive(Error, Debug)]
pub enum NoiseError {
    #[error("noise protocol error: {0}")]
    Snow(#[from] snow::Error),
    #[error("message exceeds Noise 65535-byte limit")]
    MessageTooLarge,
}

pub struct StaticKeypair {
    pub private: Vec<u8>,
    pub public: Vec<u8>,
}

pub fn generate_static_keypair() -> Result<StaticKeypair, NoiseError> {
    let kp = Builder::new(NOISE_PATTERN_IK.parse()?).generate_keypair()?;
    Ok(StaticKeypair {
        private: kp.private,
        public: kp.public,
    })
}

/// Authenticated Noise transport channel (X25519 + ChaCha20-Poly1305 + BLAKE2s).
pub struct NoiseChannel {
    state: TransportState,
}

impl NoiseChannel {
    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, NoiseError> {
        if plaintext.len() + 16 > 65535 {
            return Err(NoiseError::MessageTooLarge);
        }
        let mut out = vec![0u8; plaintext.len() + 16];
        let n = self.state.write_message(plaintext, &mut out)?;
        out.truncate(n);
        Ok(out)
    }

    pub fn decrypt(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, NoiseError> {
        let mut out = vec![0u8; ciphertext.len()];
        let n = self.state.read_message(ciphertext, &mut out)?;
        out.truncate(n);
        Ok(out)
    }

    /// Performs an in-memory Noise_IK handshake when responder's static public key is pinned (TOFU).
    pub fn handshake_ik_pair(
        initiator_priv: &[u8],
        responder_priv: &[u8],
        responder_pub: &[u8],
    ) -> Result<(Self, Self), NoiseError> {
        let mut init = Builder::new(NOISE_PATTERN_IK.parse()?)
            .local_private_key(initiator_priv)
            .remote_public_key(responder_pub)
            .build_initiator()?;
        let mut resp = Builder::new(NOISE_PATTERN_IK.parse()?)
            .local_private_key(responder_priv)
            .build_responder()?;

        let mut buf1 = vec![0u8; 256];
        let mut buf2 = vec![0u8; 256];
        let mut tmp = vec![0u8; 256];

        // -> e, es, s, ss
        let n1 = init.write_message(&[], &mut buf1)?;
        let _ = resp.read_message(&buf1[..n1], &mut tmp)?;
        // <- e, ee, se
        let n2 = resp.write_message(&[], &mut buf2)?;
        let _ = init.read_message(&buf2[..n2], &mut tmp)?;

        Ok((
            Self {
                state: init.into_transport_mode()?,
            },
            Self {
                state: resp.into_transport_mode()?,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_ik_chachapoly_blake2s_roundtrip() {
        let client_kp = generate_static_keypair().unwrap();
        let server_kp = generate_static_keypair().unwrap();

        let (mut client_ch, mut server_ch) = NoiseChannel::handshake_ik_pair(
            &client_kp.private,
            &server_kp.private,
            &server_kp.public,
        )
        .unwrap();

        let msg = b"OmniKVM HID frame payload";
        let ct = client_ch.encrypt(msg).unwrap();
        assert_ne!(ct.as_slice(), msg);
        let pt = server_ch.decrypt(&ct).unwrap();
        assert_eq!(pt.as_slice(), msg);
    }
}
