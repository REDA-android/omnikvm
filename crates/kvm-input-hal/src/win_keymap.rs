//! Bidirectional mapping between Windows Set-1 hardware scancodes (+ `LLKHF_EXTENDED`
//! / `KEYEVENTF_EXTENDEDKEY`) and canonical Linux evdev keycodes used on the OmniKVM wire.
//!
//! Standard keys `1..=88` (letters, digits, F1..F12, Shift, LCtrl, LAlt, Space, Enter, Esc)
//! share identical codes between Set-1 and evdev. Navigation keys, Right-Ctrl/Alt, and
//! Windows/Meta keys use the `E0` extended prefix on Windows and distinct codes `96..=127`
//! on evdev.

/// Converts a Windows `KBDLLHOOKSTRUCT` `(scanCode, extended)` pair to canonical evdev keycode.
pub fn win_scancode_to_evdev(scan: u32, extended: bool) -> u32 {
    if !extended {
        return scan;
    }
    match scan & 0xFF {
        0x1C => 96,  // KEY_KPENTER
        0x1D => 97,  // KEY_RIGHTCTRL
        0x35 => 98,  // KEY_KPSLASH
        0x37 => 99,  // KEY_SYSRQ (PrintScreen)
        0x38 => 100, // KEY_RIGHTALT (AltGr)
        0x47 => 102, // KEY_HOME
        0x48 => 103, // KEY_UP
        0x49 => 104, // KEY_PAGEUP
        0x4B => 105, // KEY_LEFT
        0x4D => 106, // KEY_RIGHT
        0x4F => 107, // KEY_END
        0x50 => 108, // KEY_DOWN
        0x51 => 109, // KEY_PAGEDOWN
        0x52 => 110, // KEY_INSERT
        0x53 => 111, // KEY_DELETE
        0x5B => 125, // KEY_LEFTMETA (Left Win)
        0x5C => 126, // KEY_RIGHTMETA (Right Win)
        0x5D => 127, // KEY_COMPOSE (Menu)
        other => other,
    }
}

/// Converts a canonical evdev keycode to a Windows `(wScan, extended)` pair for `SendInput`.
pub fn evdev_to_win_scancode(evdev: u32) -> (u16, bool) {
    match evdev {
        96 => (0x1C, true),
        97 => (0x1D, true),
        98 => (0x35, true),
        99 => (0x37, true),
        100 => (0x38, true),
        102 => (0x47, true),
        103 => (0x48, true),
        104 => (0x49, true),
        105 => (0x4B, true),
        106 => (0x4D, true),
        107 => (0x4F, true),
        108 => (0x50, true),
        109 => (0x51, true),
        110 => (0x52, true),
        111 => (0x53, true),
        125 => (0x5B, true),
        126 => (0x5C, true),
        127 => (0x5D, true),
        code => ((code & 0xFFFF) as u16, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_standard_and_extended_keys() {
        // Standard keys: Esc(1), A(30), L(38), LCtrl(29), LAlt(56), F12(88)
        for ev in [1u32, 28, 29, 30, 38, 42, 56, 57, 88] {
            let (scan, ext) = evdev_to_win_scancode(ev);
            assert!(!ext);
            assert_eq!(win_scancode_to_evdev(scan as u32, ext), ev);
        }
        // Extended keys: RCtrl(97), RAlt(100), Up/Left/Right/Down, Delete(111), LWin(125)
        for ev in [96u32, 97, 100, 102, 103, 105, 106, 107, 108, 110, 111, 125, 126] {
            let (scan, ext) = evdev_to_win_scancode(ev);
            assert!(ext, "expected extended flag for evdev {ev}");
            assert_eq!(win_scancode_to_evdev(scan as u32, ext), ev);
        }
    }
}
