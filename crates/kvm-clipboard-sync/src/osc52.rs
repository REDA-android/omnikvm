use base64::engine::general_purpose::STANDARD;
use base64::Engine;

/// Encodes arbitrary bytes into an ANSI OSC 52 terminal clipboard escape sequence
/// (`\x1b]52;c;<base64>\x07`), enabling clipboard sync over SSH/tmux sessions
/// (inspired by gitlab.com/timvisee/rust-clipboard-ext).
pub fn encode_osc52(data: &[u8]) -> String {
    let b64 = STANDARD.encode(data);
    format!("\x1b]52;c;{b64}\x07")
}

/// Decodes an ANSI OSC 52 clipboard sequence back into raw bytes.
pub fn decode_osc52(seq: &str) -> Option<Vec<u8>> {
    let body = seq.strip_prefix("\x1b]52;c;")?;
    let b64 = body
        .strip_suffix('\x07')
        .or_else(|| body.strip_suffix("\x1b\\"))?;
    STANDARD.decode(b64).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc52_encode_decode_roundtrip() {
        let msg = b"omnikvm-ssh-clipboard-sync";
        let seq = encode_osc52(msg);
        assert!(seq.starts_with("\x1b]52;c;"));
        assert!(seq.ends_with('\x07'));
        assert_eq!(decode_osc52(&seq).unwrap(), msg);
    }
}
