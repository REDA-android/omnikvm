//! Linux DDC/CI transport over `/dev/i2c-N` (i2c-dev). Monitors answer on 7-bit
//! slave address `0x37`; the host writes the MCCS frame built by [`crate::ddc`]
//! (same wire bytes as `ddcutil setvcp`). Requires the `i2c-dev` kernel module and
//! read/write access to the bus (usually the `i2c` group).

use crate::ddc::build_set_vcp_frame;
use crate::HalError;
use std::path::{Path, PathBuf};

pub const DDC_CI_SLAVE_ADDR: u16 = 0x37;
#[cfg(target_os = "linux")]
const I2C_SLAVE: libc::c_ulong = 0x0703;

/// Lists candidate `/dev/i2c-*` buses sorted by bus number.
pub fn list_i2c_buses() -> Vec<PathBuf> {
    list_i2c_buses_in(Path::new("/dev"))
}

pub fn list_i2c_buses_in(dir: &Path) -> Vec<PathBuf> {
    let mut buses: Vec<(u32, PathBuf)> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter_map(|e| {
                    let name = e.file_name().into_string().ok()?;
                    let n = name.strip_prefix("i2c-")?.parse::<u32>().ok()?;
                    Some((n, e.path()))
                })
                .collect()
        })
        .unwrap_or_default();
    buses.sort_by_key(|(n, _)| *n);
    buses.into_iter().map(|(_, p)| p).collect()
}

/// Writes a Set-VCP-Feature request to the monitor on `bus`.
#[cfg(target_os = "linux")]
pub fn set_vcp(bus: &Path, vcp_code: u8, value: u16) -> Result<[u8; 7], HalError> {
    use std::io::Write;
    use std::os::unix::io::AsRawFd;
    let frame = build_set_vcp_frame(vcp_code, value);
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(bus)
        .map_err(|e| HalError::DeviceOpen(format!("{}: {e}", bus.display())))?;
    // SAFETY: valid fd owned by `f`; I2C_SLAVE takes the 7-bit address by value.
    let rc = unsafe { libc::ioctl(f.as_raw_fd(), I2C_SLAVE, DDC_CI_SLAVE_ADDR as libc::c_ulong) };
    if rc < 0 {
        return Err(HalError::DeviceOpen(format!(
            "I2C_SLAVE 0x37 on {}: {}",
            bus.display(),
            std::io::Error::last_os_error()
        )));
    }
    f.write_all(&frame)
        .map_err(|e| HalError::DeviceOpen(format!("DDC write: {e}")))?;
    // MCCS requires >= 50 ms between consecutive DDC/CI commands.
    std::thread::sleep(std::time::Duration::from_millis(50));
    Ok(frame)
}

#[cfg(not(target_os = "linux"))]
pub fn set_vcp(_bus: &Path, vcp_code: u8, value: u16) -> Result<[u8; 7], HalError> {
    let _ = build_set_vcp_frame(vcp_code, value);
    Err(HalError::DeviceOpen("i2c-dev DDC/CI is Linux-only".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_and_sorts_i2c_buses() {
        let dir = std::env::temp_dir().join(format!("omnikvm_i2c_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for n in ["i2c-10", "i2c-2", "i2c-x", "tty0"] {
            std::fs::write(dir.join(n), b"").unwrap();
        }
        let buses = list_i2c_buses_in(&dir);
        let names: Vec<_> = buses
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(names, vec!["i2c-2", "i2c-10"]);
    }

    #[test]
    fn set_vcp_on_missing_bus_reports_error() {
        assert!(set_vcp(Path::new("/nonexistent/i2c-99"), 0x60, 0x11).is_err());
    }
}
