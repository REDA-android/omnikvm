use serde::{Deserialize, Serialize};

/// DDC/CI VCP Feature Codes (MCCS 2.2a standard).
pub const VCP_BRIGHTNESS: u8 = 0x10;
pub const VCP_INPUT_SOURCE: u8 = 0x60;
pub const DDC_DEST_ADDR: u8 = 0x6E;
pub const DDC_HOST_ADDR: u8 = 0x51;

/// Standard MCCS 2.2a Monitor Input Source values for VCP `0x60`.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MonitorInputSource {
    DisplayPort1 = 0x0F,
    DisplayPort2 = 0x10,
    Hdmi1 = 0x11,
    Hdmi2 = 0x12,
    UsbC = 0x1B,
}

/// Builds a standards-compliant DDC/CI Set VCP Feature I2C payload:
/// `[0x51, 0x84, 0x03, vcp_code, high_byte, low_byte, checksum]`
/// where `checksum = 0x6E ^ bytes[0..6]`.
pub fn build_set_vcp_frame(vcp_code: u8, value: u16) -> [u8; 7] {
    let hi = (value >> 8) as u8;
    let lo = (value & 0xFF) as u8;
    let mut frame = [DDC_HOST_ADDR, 0x84, 0x03, vcp_code, hi, lo, 0x00];
    let mut chk = DDC_DEST_ADDR;
    for &b in &frame[..6] {
        chk ^= b;
    }
    frame[6] = chk;
    frame
}

impl MonitorInputSource {
    pub const ALL: [MonitorInputSource; 5] = [
        Self::DisplayPort1,
        Self::DisplayPort2,
        Self::Hdmi1,
        Self::Hdmi2,
        Self::UsbC,
    ];

    /// Parses CLI/HTTP friendly names: `dp1`, `dp2`, `hdmi1`, `hdmi2`, `usbc`.
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().replace(['-', '_', ' '], "").as_str() {
            "dp1" | "displayport1" => Some(Self::DisplayPort1),
            "dp2" | "displayport2" => Some(Self::DisplayPort2),
            "hdmi1" => Some(Self::Hdmi1),
            "hdmi2" => Some(Self::Hdmi2),
            "usbc" | "typec" => Some(Self::UsbC),
            _ => None,
        }
    }
}

pub fn build_input_switch_frame(input: MonitorInputSource) -> [u8; 7] {
    build_set_vcp_frame(VCP_INPUT_SOURCE, input as u16)
}

pub fn verify_ddc_checksum(frame: &[u8; 7]) -> bool {
    let mut chk = DDC_DEST_ADDR;
    for &b in &frame[..6] {
        chk ^= b;
    }
    chk == frame[6]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ddc_vcp_input_switch_frame_has_valid_mccs_checksum() {
        let frame = build_input_switch_frame(MonitorInputSource::Hdmi1);
        assert_eq!(frame[0], 0x51);
        assert_eq!(frame[3], VCP_INPUT_SOURCE);
        assert_eq!(frame[5], 0x11);
        assert!(verify_ddc_checksum(&frame));

        let dp = build_input_switch_frame(MonitorInputSource::DisplayPort1);
        assert_eq!(dp[5], 0x0F);
        assert!(verify_ddc_checksum(&dp));

        let bright = build_set_vcp_frame(VCP_BRIGHTNESS, 75);
        assert_eq!(bright[3], VCP_BRIGHTNESS);
        assert_eq!(bright[5], 75);
        assert!(verify_ddc_checksum(&bright));
    }

    #[test]
    fn parses_friendly_input_names() {
        assert_eq!(MonitorInputSource::parse("HDMI-1"), Some(MonitorInputSource::Hdmi1));
        assert_eq!(MonitorInputSource::parse("dp2"), Some(MonitorInputSource::DisplayPort2));
        assert_eq!(MonitorInputSource::parse("usb_c"), Some(MonitorInputSource::UsbC));
        assert_eq!(MonitorInputSource::parse("vga"), None);
    }
}
