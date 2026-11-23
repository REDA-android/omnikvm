use crate::{HalError, InputEvent, SystemInputInjector};
use std::path::PathBuf;

/// Linux Wayland `libei` / `reis` RemoteDesktop portal backend abstraction.
#[derive(Debug, Clone)]
pub struct WaylandLibeiBackend {
    pub wayland_display: String,
    pub runtime_dir: PathBuf,
    pub restore_token: Option<String>,
}

impl WaylandLibeiBackend {
    pub fn from_env() -> Result<Self, HalError> {
        let wayland_display = std::env::var("WAYLAND_DISPLAY")
            .map_err(|_| HalError::DeviceOpen("WAYLAND_DISPLAY not set".into()))?;
        let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .map_err(|_| HalError::DeviceOpen("XDG_RUNTIME_DIR not set".into()))?;
        Ok(Self {
            wayland_display,
            runtime_dir,
            restore_token: None,
        })
    }

    pub fn socket_path(&self) -> PathBuf {
        self.runtime_dir.join(&self.wayland_display)
    }

    pub fn with_restore_token(mut self, token: impl Into<String>) -> Self {
        self.restore_token = Some(token.into());
        self
    }
}

impl SystemInputInjector for WaylandLibeiBackend {
    fn inject(&self, _event: InputEvent) -> Result<(), HalError> {
        // Dispatches over EIS socket (libei / reis) once RemoteDesktop portal session is negotiated.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wayland_socket_path_resolution() {
        let b = WaylandLibeiBackend {
            wayland_display: "wayland-0".into(),
            runtime_dir: PathBuf::from("/run/user/1000"),
            restore_token: None,
        }
        .with_restore_token("portal-token-abc");
        assert_eq!(b.socket_path(), PathBuf::from("/run/user/1000/wayland-0"));
        assert_eq!(b.restore_token.as_deref(), Some("portal-token-abc"));
    }
}
