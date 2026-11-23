//! Global hotkey chord detection over Linux evdev keycodes (the canonical keycode
//! space used on the OmniKVM wire). Chords such as `ctrl+alt+l` are parsed from
//! config strings and matched on key-down while tracking modifier state, so the
//! chord itself can be swallowed instead of being forwarded to the remote host.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HotkeyAction {
    ToggleCursorLock,
    ReleaseCapture,
    SwitchNextScreen,
    SwitchPrevScreen,
}

const MOD_CTRL: u8 = 1;
const MOD_ALT: u8 = 2;
const MOD_SHIFT: u8 = 4;
const MOD_META: u8 = 8;

fn modifier_bit(keycode: u32) -> Option<u8> {
    match keycode {
        29 | 97 => Some(MOD_CTRL),
        56 | 100 => Some(MOD_ALT),
        42 | 54 => Some(MOD_SHIFT),
        125 | 126 => Some(MOD_META),
        _ => None,
    }
}

fn key_from_name(name: &str) -> Option<u32> {
    let n = name.to_ascii_lowercase();
    let letters = "qwertyuiop\0\0\0\0asdfghjkl\0\0\0\0\0zxcvbnm";
    if n.len() == 1 {
        let c = n.as_bytes()[0];
        if c.is_ascii_lowercase() {
            return letters.find(c as char).map(|i| 16 + i as u32);
        }
        if c.is_ascii_digit() {
            return Some(if c == b'0' { 11 } else { (c - b'1') as u32 + 2 });
        }
    }
    if let Some(f) = n.strip_prefix('f').and_then(|d| d.parse::<u32>().ok()) {
        return match f {
            1..=10 => Some(58 + f),
            11 => Some(87),
            12 => Some(88),
            _ => None,
        };
    }
    match n.as_str() {
        "esc" | "escape" => Some(1),
        "tab" => Some(15),
        "space" => Some(57),
        "enter" => Some(28),
        "left" => Some(105),
        "right" => Some(106),
        "up" => Some(103),
        "down" => Some(108),
        "scrolllock" => Some(70),
        "pause" => Some(119),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chord {
    pub mods: u8,
    pub key: u32,
}

impl Chord {
    /// Parses `"ctrl+alt+l"`, `"Ctrl+Shift+F10"`, `"meta+right"`...
    pub fn parse(spec: &str) -> Option<Self> {
        let mut mods = 0u8;
        let mut key = None;
        for part in spec.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => mods |= MOD_CTRL,
                "alt" => mods |= MOD_ALT,
                "shift" => mods |= MOD_SHIFT,
                "meta" | "super" | "win" | "cmd" => mods |= MOD_META,
                other => key = Some(key_from_name(other)?),
            }
        }
        Some(Self { mods, key: key? })
    }
}

#[derive(Debug, Default)]
pub struct HotkeyManager {
    bindings: Vec<(Chord, HotkeyAction)>,
    mods: u8,
}

impl HotkeyManager {
    /// Default bindings inspired by Input Leap / Deskflow conventions.
    pub fn with_defaults() -> Self {
        let mut m = Self::default();
        for (spec, act) in [
            ("ctrl+alt+l", HotkeyAction::ToggleCursorLock),
            ("ctrl+alt+esc", HotkeyAction::ReleaseCapture),
            ("ctrl+alt+right", HotkeyAction::SwitchNextScreen),
            ("ctrl+alt+left", HotkeyAction::SwitchPrevScreen),
        ] {
            m.bind(spec, act);
        }
        m
    }

    /// Adds/replaces a binding; returns false if the chord string is invalid.
    pub fn bind(&mut self, spec: &str, action: HotkeyAction) -> bool {
        let Some(chord) = Chord::parse(spec) else { return false };
        self.bindings.retain(|(c, a)| *c != chord && *a != action);
        self.bindings.push((chord, action));
        true
    }

    /// Feeds a key transition; returns the triggered action on chord key-down.
    pub fn on_key(&mut self, keycode: u32, pressed: bool) -> Option<HotkeyAction> {
        if let Some(bit) = modifier_bit(keycode) {
            if pressed {
                self.mods |= bit;
            } else {
                self.mods &= !bit;
            }
            return None;
        }
        if !pressed {
            return None;
        }
        self.bindings
            .iter()
            .find(|(c, _)| c.key == keycode && c.mods == self.mods)
            .map(|(_, a)| *a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_chords() {
        assert_eq!(Chord::parse("ctrl+alt+l"), Some(Chord { mods: MOD_CTRL | MOD_ALT, key: 38 }));
        assert_eq!(Chord::parse("Shift+F10").unwrap().key, 68);
        assert_eq!(Chord::parse("meta+1").unwrap().key, 2);
        assert_eq!(Chord::parse("ctrl+a").unwrap().key, 30);
        assert_eq!(Chord::parse("ctrl+z").unwrap().key, 44);
        assert!(Chord::parse("ctrl+alt").is_none());
        assert!(Chord::parse("ctrl+bogus").is_none());
    }

    #[test]
    fn detects_chord_with_modifier_tracking() {
        let mut m = HotkeyManager::with_defaults();
        assert_eq!(m.on_key(38, true), None); // bare L
        m.on_key(29, true);
        m.on_key(56, true);
        assert_eq!(m.on_key(38, true), Some(HotkeyAction::ToggleCursorLock));
        assert_eq!(m.on_key(1, true), Some(HotkeyAction::ReleaseCapture));
        m.on_key(42, true); // extra shift breaks exact match
        assert_eq!(m.on_key(38, true), None);
        m.on_key(42, false);
        m.on_key(56, false);
        assert_eq!(m.on_key(38, true), None);
        assert!(m.bind("ctrl+f12", HotkeyAction::ToggleCursorLock));
        assert_eq!(m.on_key(88, true), Some(HotkeyAction::ToggleCursorLock));
    }
}
