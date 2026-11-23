use crate::{HalError, InputEvent, SystemInputInjector};

/// Linux X11 XTest backend descriptor & coordinate/button translator.
/// On X11 sessions (`DISPLAY` set), translates OmniKVM InputEvents to X11 button/keycode tuples.
#[derive(Debug, Clone)]
pub struct X11XTestBackend {
    pub display_name: String,
}

impl X11XTestBackend {
    pub fn from_env() -> Result<Self, HalError> {
        let display_name = std::env::var("DISPLAY")
            .map_err(|_| HalError::DeviceOpen("DISPLAY environment variable not set".into()))?;
        Ok(Self { display_name })
    }

    /// Maps OmniKVM mouse button index (0=left, 1=right, 2=middle) to X11 button number (1..=3).
    pub fn map_x11_button(button: u8) -> Option<u32> {
        match button {
            0 => Some(1),
            1 => Some(3),
            2 => Some(2),
            _ => None,
        }
    }

    /// Translates Linux evdev scancode to X11 keycode (standard +8 offset).
    pub fn evdev_to_x11_keycode(evdev_code: u32) -> u32 {
        evdev_code.saturating_add(8)
    }
}

impl SystemInputInjector for X11XTestBackend {
    fn inject(&self, event: InputEvent) -> Result<(), HalError> {
        match event {
            InputEvent::MouseButton { button, .. } if Self::map_x11_button(button).is_none() => {
                Err(HalError::Unsupported("unknown X11 mouse button"))
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x11_button_and_keycode_mapping() {
        assert_eq!(X11XTestBackend::map_x11_button(0), Some(1));
        assert_eq!(X11XTestBackend::map_x11_button(1), Some(3));
        assert_eq!(X11XTestBackend::map_x11_button(2), Some(2));
        assert_eq!(X11XTestBackend::evdev_to_x11_keycode(30), 38);
    }
}
