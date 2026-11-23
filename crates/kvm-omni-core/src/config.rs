use crate::topology::ScreenRect;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("config IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("config parse error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OmniConfig {
    pub peer_id: String,
    pub quic_bind: String,
    pub ui_bind: String,
    pub hysteresis_px: i32,
    pub corner_dead_zone_px: i32,
    pub screens: Vec<ScreenRect>,
    /// Persisted TOFU peer_id -> hex Blake3 certificate fingerprint.
    pub trusted_peers: HashMap<String, String>,
}

impl Default for OmniConfig {
    fn default() -> Self {
        Self {
            peer_id: "omnikvm-node".into(),
            quic_bind: "0.0.0.0:4433".into(),
            ui_bind: "127.0.0.1:8080".into(),
            hysteresis_px: 4,
            corner_dead_zone_px: 16,
            screens: vec![
                ScreenRect {
                    id: "Linux-Host-4K".into(),
                    x: 0,
                    y: 0,
                    width: 3840,
                    height: 2160,
                    dpi_scale: 2.0,
                },
                ScreenRect {
                    id: "Win11-Work-FHD".into(),
                    x: 3840,
                    y: 540,
                    width: 1920,
                    height: 1080,
                    dpi_scale: 1.0,
                },
            ],
            trusted_peers: HashMap::new(),
        }
    }
}

impl OmniConfig {
    pub fn load_or_default(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_save_and_reload_roundtrip() {
        let tmp = std::env::temp_dir().join("omnikvm_config_test.json");
        let mut cfg = OmniConfig::default();
        cfg.trusted_peers
            .insert("laptop-1".into(), "deadbeef01234567".into());
        cfg.save(&tmp).unwrap();

        let loaded = OmniConfig::load_or_default(&tmp).unwrap();
        let _ = std::fs::remove_file(&tmp);
        assert_eq!(cfg, loaded);
    }
}
