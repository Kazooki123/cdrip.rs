use crate::error::{CdripError, Result};
use cd_da_reader::CdReader;
use tracing::{debug, info, warn};

#[derive(Debug, Clone)]
pub struct DriveInfo {
    /// Platform-specific device path, e.g. `/dev/sr0`, `\\.\D:`, `/dev/disk2`
    pub path: String,
    /// Whether a disc is currently inserted (best-effort)
    pub has_disc: bool,
}

impl std::fmt::Display for DriveInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let disc_status = if self.has_disc { "disc present" } else { "no disc" };
        write!(f, "{} ({})", self.path, disc_status)
    }
}

// Drive listing
/// Return all detectable optical drives on the system.
/// On Linux this probes `/dev/sr*` and `/dev/cdrom*`.
/// On Windows it enumerates drive letters with `GetDriveTypeW`.
/// On macOS it checks `/dev/disk*` (IOKit would be ideal but we'll stay pure-Rust ¯\_(ツ)_/¯).
pub fn list_drives() -> Vec<DriveInfo> {
    let mut drives = Vec::new();

    #[cfg(target_os = "linux")]
    {
        drives.extend(probe_linux_drives());
    }

    #[cfg(target_os = "windows")]
    {
        drives.extend(probe_windows_drives());
    }

    #[cfg(target_os = "macos")]
    {
        drives.extend(probe_macos_drives());
    }

    if drives.is_empty() {
        warn!("No optical drives detected on this system");
    } else {
        info!("Detected {} drive(s)", drives.len());
    }

    drives
}

#[cfg(target_os = "linux")]
fn probe_linux_drives() -> Vec<DriveInfo> {
    use std::path::Path;

    let mut drives = Vec::new();

    // Try /dev/sr0 – /dev/sr7 and /dev/cdrom, /dev/cdrom1
    let candidates: Vec<String> = (0..8)
        .map(|i| format!("/dev/sr{}", i))
        .chain((0..4).map(|i| {
            if i == 0 {
                "/dev/cdrom".to_string()
            } else {
                format!("/dev/cdrom{}", i)
            }
        }))
        .collect();

    for path in candidates {
        if Path::new(&path).exists() {
            let has_disc = probe_disc_present(&path);
            debug!("Found drive: {} (disc={})", path, has_disc);
            drives.push(DriveInfo { path, has_disc });
        }
    }

    drives
}

#[cfg(target_os = "windows")]
fn probe_windows_drives() -> Vec<DriveInfo> {
    let mut drives = Vec::new();

    for letter in b'D'..=b'Z' {
        let path = format!("{}:\\", letter as char);
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let drive_type = unsafe { windows_drive_type(wide.as_ptr()) };
        if drive_type == 5 {
            let device_path = format!("\\\\.\\{}:", letter as char);
            let has_disc = probe_disc_present(&device_path);
            debug!("Found CD drive: {} (disc={})", device_path, has_disc);
            drives.push(DriveInfo {
                path: device_path,
                has_disc,
            });
        }
    }

    drives
}

#[cfg(target_os = "windows")]
unsafe fn windows_drive_type(path: *const u16) -> u32 {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDriveTypeW(lp_root_path_name: *const u16) -> u32;
    }
    unsafe { GetDriveTypeW(path) }
}

#[cfg(target_os = "macos")]
fn probe_macos_drives() -> Vec<DriveInfo> {
    use std::path::Path;

    let mut drives = Vec::new();

    // macOS optical drives usually appear as /dev/disk1 – /dev/disk9
    // We'll try to open each with cd-da-reader to confirm IT IS optical.
    for i in 0..10 {
        let path = format!("/dev/disk{}", i);
        if Path::new(&path).exists() {
            // Attempt a lightweight open just to check
            if let Ok(reader) = CdReader::open(&path) {
                let has_disc = reader.read_toc().is_ok();
                debug!("Found drive: {} (disc={})", path, has_disc);
                drives.push(DriveInfo { path, has_disc });
            }
        }
    }

    drives
}

fn probe_disc_present(path: &str) -> bool {
    match CdReader::open(path) {
        Ok(reader) => reader.read_toc().is_ok(),
        Err(_) => false,
    }
}

// Drive opening
pub fn open_drive(path: &str) -> Result<CdReader> {
    CdReader::open(path).map_err(|e| CdripError::DriveOpenFailed(format!("{}: {}", path, e)))
}

pub fn open_default_drive() -> Result<CdReader> {
    if let Ok(reader) = CdReader::open_default() {
        return Ok(reader);
    }

    let drives = list_drives();
    if drives.is_empty() {
        return Err(CdripError::NoDriveFound);
    }

    let target = drives
        .iter()
        .find(|d| d.has_disc)
        .or_else(|| drives.first())
        .ok_or(CdripError::NoDriveFound)?;

    open_drive(&target.path)
}
