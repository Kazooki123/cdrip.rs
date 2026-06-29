//! # Hidden Track One Audio (HTOA)
//! 
//! HTOA is audio data hidden in the Index 00 pregap of Track 1 — the region
//! *BEFORE* normal playback begins. CD players skip it entirely; you have to
//! manually rewind past 0:00 to hear it. Classic 90s/2000s CD easter egg.
//!
//! ## How it works
//!
//! The audio program area of a CD starts at LBA 0 (Track 1, Index 01).
//! Before that is a mandatory 2-second lead-in (150 sectors, LBA -150 to -1)
//! that the drive firmware normally uses for positioning. HTOA lives here —
//! or in an *extended* pregap if the artist requested more than 2 seconds.
//!
//! ## Detection heuristic
//!
//! From the Hydrogenaudio wiki: if Track 1's pregap is **> 6 seconds**
//! (> 450 sectors), there is likely real audio rather than silence.
//! We also verify by sampling a few sectors and checking for non-zero bytes.
//!
//! ## Drive compatibility
//!
//! Not all drives support reading before LBA 0. We try, catch the error,
//! and report the drive as HTOA-incapable — no crash, no panic.
//!
//! Check this to see if your driver supports HTOA:
//! https://www.daefeatures.co.uk/search?htoa=Yes
//! 
//! ## Output
//!
//! Extracted HTOA is written as `track00.flac` (or `.wav`) so it sorts
//! naturally before `track01` in any file manager.
//! 
//! Reference:
//! https://wiki.hydrogenaudio.org/index.php?title=HTOA

#[allow(unused)]

use crate::{
    encoder::{make_encoder, OutputFormat},
    error::{CdripError, Result},
    toc::DiscToc,
};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

// Pregap thresholds
/// Standard 2-second lead-in in sectors (75 sectors/sec × 2 sec).
/// All CDs have at least this many pregap sectors; it's usually silence.
pub const STANDARD_PREGAP_SECTORS: u32 = 150;
pub const HTOA_LIKELY_THRESHOLD_SECTORS: u32 = 450;

/// Minimum non-zero bytes in a sampled sector to consider it non-silent.
/// A sector is 2352 bytes; requiring 64 non-zero bytes filters out
/// quantisation noise and DC offset while catching real audio.
const SILENCE_THRESHOLD_BYTES: usize = 64;

// PUBLIC TYPES
/// Result of an HTOA detection pass.
#[derive(Debug, Clone, PartialEq)]
pub enum HtoaStatus {
    DriveUnsupported,
    NoPregap,
    SilentPregap { sectors: u32 },
    HtoaDetected { sectors: u32, duration_secs: f32 },
}

impl std::fmt::Display for HtoaStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HtoaStatus::DriveUnsupported =>
                write!(f, "Drive does not support pregap reads (HTOA unavailable)"),
            HtoaStatus::NoPregap =>
                write!(f, "No extended pregap — standard 2-second lead-in only"),
            HtoaStatus::SilentPregap { sectors } =>
                write!(f, "Pregap present ({} sectors / {:.1}s) but silent",
                    sectors, *sectors as f32 / 75.0),
            HtoaStatus::HtoaDetected { sectors, duration_secs } =>
                write!(f, "HTOA detected! {} sectors ({:.1}s of hidden audio)",
                    sectors, duration_secs),
        }
    }
}

// Detection
/// Probe for HTOA by reading the pregap length from the TOC and sampling
/// a few sectors before LBA 0.
/// `device_path` is the raw device string (e.g. `/dev/sr0`, `\\.\D:`).
/// Does **not** extract audio — call [`extract_htoa`] for that.
pub fn detect_htoa(device_path: &str, toc: &DiscToc) -> HtoaStatus {
    let pregap_sectors = pregap_sector_count(toc);

    debug!(
        "Track 1 pregap: {} sectors ({:.1}s)",
        pregap_sectors,
        pregap_sectors as f32 / 75.0
    );

    if pregap_sectors <= STANDARD_PREGAP_SECTORS {
        return HtoaStatus::NoPregap;
    }

    // We have an extended pregap, try to read a sample sector to check for
    // actual audio content and to test drive compatibility.
    // HTOA sectors live at LBA -(pregap_sectors) to -1 relative to track start.
    // In absolute terms that's 0 - pregap_sectors .. 0.
    // We sample from the middle of the extended region.
    let sample_lba_offset = pregap_sectors / 2;

    match read_pregap_sector(device_path, sample_lba_offset) {
        Err(e) => {
            warn!("Pregap sector read failed — drive likely unsupported: {}", e);
            HtoaStatus::DriveUnsupported
        }
        Ok(sector) => {
            let duration_secs = pregap_sectors as f32 / 75.0;

            if is_silent(&sector) {
                // One sector was silent — sample a few more before concluding
                let has_audio = probe_additional_sectors(device_path, pregap_sectors);
                if has_audio {
                    HtoaStatus::HtoaDetected { sectors: pregap_sectors, duration_secs }
                } else {
                    HtoaStatus::SilentPregap { sectors: pregap_sectors }
                }
            } else {
                info!("Non-silent pregap sector found — HTOA confirmed");
                HtoaStatus::HtoaDetected { sectors: pregap_sectors, duration_secs }
            }
        }
    }
}

fn probe_additional_sectors(device_path: &str, pregap_sectors: u32) -> bool {
    let probe_count = 5u32;
    for i in 1..=probe_count {
        let offset = (pregap_sectors * i) / (probe_count + 1);
        if let Ok(sector) = read_pregap_sector(device_path, offset) {
            if !is_silent(&sector) {
                return true;
            }
        }
    }
    false
}

// Extraction
/// Extract HTOA audio and encode it to `output_dir/track00.<ext>`.
/// Returns `Ok(Some(path))` if HTOA was extracted successfully,
/// `Ok(None)` if the drive doesn't support it or there's nothing there,
/// `Err(_)` on I/O or encoding failure.
pub fn extract_htoa(
    device_path: &str,
    toc: &DiscToc,
    format: OutputFormat,
    output_dir: &Path,
) -> Result<Option<PathBuf>> {
    let pregap_sectors = pregap_sector_count(toc);

    if pregap_sectors <= STANDARD_PREGAP_SECTORS {
        info!("No extended pregap — skipping HTOA extraction");
        return Ok(None);
    }

    info!(
        "Extracting HTOA: {} pregap sectors ({:.1}s)",
        pregap_sectors,
        pregap_sectors as f32 / 75.0
    );

    // Read all pregap sectors into a PCM buffer.
    // We'll skip the last 150 (standard lead-in) and instead read the extended portion.
    let extended_sectors = pregap_sectors.saturating_sub(STANDARD_PREGAP_SECTORS);
    let mut pcm: Vec<u8> = Vec::with_capacity(extended_sectors as usize * 2352);

    for offset in (1..=extended_sectors).rev() {
        match read_pregap_sector(device_path, offset) {
            Ok(sector) => pcm.extend_from_slice(&sector),
            Err(e) => {
                if pcm.is_empty() {
                    warn!("First pregap sector failed — drive unsupported: {}", e);
                    return Ok(None);
                }
                warn!("Pregap sector at offset -{} failed, substituting silence: {}", offset, e);
                pcm.extend_from_slice(&[0u8; 2352]);
            }
        }
    }

    if pcm.is_empty() {
        return Ok(None);
    }

    // Check if what we read is actually non-silent
    if pcm.iter().filter(|&&b| b != 0).count() < SILENCE_THRESHOLD_BYTES * 4 {
        info!("HTOA PCM buffer is effectively silent — not writing output file");
        return Ok(None);
    }

    let filename = format!("track00.{}", format.extension());
    let output_path = output_dir.join(&filename);
    let encoder = make_encoder(format);

    encoder.encode(0, &pcm, &output_path)?;

    info!("HTOA extracted → {}", output_path.display());
    Ok(Some(output_path))
}

// Pregap length
/// Calculate the Track 1 pregap size in sectors from the TOC.
///
/// Track 1's Index 01 starts at `track.start_lba`. The disc program area
/// starts at LBA 0 by convention. So the pregap size = `track.start_lba`
/// (which equals the number of sectors before Index 01).
///
/// Most discs have `start_lba = 0` (no extended pregap). HTOA discs have
/// `start_lba > 0`, with the pregap sectors occupying negative LBAs.
pub fn pregap_sector_count(toc: &DiscToc) -> u32 {
    toc.tracks
        .first()
        .map(|t| t.start_lba)
        .unwrap_or(0)
        .max(STANDARD_PREGAP_SECTORS)
}

// Low-level sector read
/// Read one 2352-byte sector from the pregap region.
/// `lba_offset` is how many sectors *before* LBA 0 to read (1 = LBA -1).
/// This requires drive support for negative/pre-zero LBA reads via SCSI.
/// Many drives silently refuse and return an error — that's expected.
fn read_pregap_sector(device_path: &str, lba_offset: u32) -> std::result::Result<Vec<u8>, String> {
    #[cfg(target_os = "linux")]
    {
        linux::read_sector_before_zero(device_path, lba_offset)
    }

    #[cfg(target_os = "windows")]
    {
        windows::read_sector_before_zero(device_path, lba_offset)
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        Err("Pregap sector reads not supported on this platform".to_string())
    }
}

fn is_silent(sector: &[u8]) -> bool {
    sector.iter().filter(|&&b| b != 0).count() < SILENCE_THRESHOLD_BYTES
}

// Linux SCSI implementation
#[cfg(target_os = "linux")]
mod linux {
    /// Read a single 2352-byte CD-DA sector from a negative LBA.
    /// Uses the SCSI READ CD command (0xBE) with an absolute LBA address.
    /// The negative LBA is encoded as a large u32 via two's complement:
    /// LBA -1 = 0xFFFFFFFF, LBA -150 = 0xFFFFFF6A, etc.
    pub fn read_sector_before_zero(device: &str, offset: u32) -> Result<Vec<u8>, String> {
        use std::{fs::OpenOptions, os::unix::io::AsRawFd};

        let file = OpenOptions::new()
            .read(true)
            .open(device)
            .map_err(|e| format!("open {}: {}", device, e))?;

        let fd = file.as_raw_fd();

        // Twos complement negative LBA: -offset = 0x100000000 - offset
        let lba: u32 = 0u32.wrapping_sub(offset);
        let lba_bytes = lba.to_be_bytes();

        // READ CD CDB (12 bytes):
        // 0xBE  Expected sector type=1 (CD-DA)  LBA[4]  Length=1  Flags  Subchannel=0
        let mut cdb = [0u8; 12];
        cdb[0] = 0xBE;         // READ CD opcode
        cdb[1] = 0x02;         // Expected sector type: CD-DA
        cdb[2] = lba_bytes[0]; // LBA MSB
        cdb[3] = lba_bytes[1];
        cdb[4] = lba_bytes[2];
        cdb[5] = lba_bytes[3]; // LBA LSB
        cdb[6] = 0x00;         // Transfer length MSB (1 sect)
        cdb[7] = 0x00;
        cdb[8] = 0x01;         // Transfer length LSB
        cdb[9] = 0xF8;         // Sync+Header+SubHeader+UserData+EDC/ECC
        cdb[10] = 0x00;
        cdb[11] = 0x00;        // No subchannel

        let mut buf = vec![0u8; 2352];
        let mut sense = [0u8; 32];

        #[repr(C)]
        struct SgIoHdr {
            interface_id: i32, dxfer_direction: i32, cmd_len: u8,
            mx_sb_len: u8, iovec_count: u16, dxfer_len: u32,
            dxferp: *mut u8, cmdp: *const u8, sbp: *mut u8,
            timeout: u32, flags: u32, pack_id: i32,
            usr_ptr: *mut std::ffi::c_void, status: u8, masked_status: u8,
            msg_status: u8, sb_len_wr: u8, host_status: u16,
            driver_status: u16, resid: i32, duration: u32, info: u32,
        }

        let hdr = SgIoHdr {
            interface_id: b'S' as i32, dxfer_direction: -3,
            cmd_len: 12, mx_sb_len: 32, iovec_count: 0,
            dxfer_len: 2352, dxferp: buf.as_mut_ptr(),
            cmdp: cdb.as_ptr(), sbp: sense.as_mut_ptr(),
            timeout: 5000, flags: 0, pack_id: 0,
            usr_ptr: std::ptr::null_mut(),
            status: 0, masked_status: 0, msg_status: 0, sb_len_wr: 0,
            host_status: 0, driver_status: 0, resid: 0, duration: 0, info: 0,
        };

        unsafe extern "C" { fn ioctl(fd: i32, req: u64, ...) -> i32; }

        let ret = unsafe { ioctl(fd, 0x2285, &hdr as *const SgIoHdr) };

        if ret != 0 || hdr.status != 0 {
            return Err(format!(
                "READ CD at LBA -{} failed: ioctl={} scsi_status=0x{:02x}",
                offset, ret, hdr.status
            ));
        }

        Ok(buf)
    }
}

// Windows SCSI implementation
#[cfg(target_os = "windows")]
mod windows {
    /// Read a single 2352-byte CD-DA sector from a negative LBA on Windows.
    /// Uses IOCTL_CDROM_RAW_READ with RawReadInfo specifying the negative
    /// sector offset. Not all Windows CD drivers expose this correctly.
    pub fn read_sector_before_zero(device: &str, offset: u32) -> Result<Vec<u8>, String> {
        let path_wide: Vec<u16> = device.encode_utf16().chain(std::iter::once(0)).collect();

        let handle = unsafe {
            CreateFileW(
                path_wide.as_ptr(), 0x80000000, 0x3,
                std::ptr::null_mut(), 3, 0, std::ptr::null_mut(),
            )
        };

        if handle.is_null() || handle == usize::MAX as *mut std::ffi::c_void {
            return Err(format!("Cannot open {}", device));
        }

        // RAW_READ_INFO
        #[repr(C)]
        struct RawReadInfo {
            disk_offset: i64,   // Byte offset from start of disc (can be negative-ish)
            sector_count: u32,
            track_mode: u32,    // 0 = CDDA
        }

        // Byte offset: each sector = 2352 bytes. Negative offset from sector 0.
        // We'll use the absolute disc offset: disc starts at sector -150 (lead-in).
        // Pregap sector at offset N before 0 = sector (150 - N) from disc start.
        let disk_offset = (150i64 - offset as i64) * 2352;

        let info = RawReadInfo {
            disk_offset,
            sector_count: 1,
            track_mode: 0,
        };

        let mut buf = vec![0u8; 2352];
        let mut bytes_returned: u32 = 0;

        // IOCTL_CDROM_RAW_READ = 0x0002403E
        let ok = unsafe {
            DeviceIoControl(
                handle, 0x0002403E,
                &info as *const _ as *const std::ffi::c_void,
                std::mem::size_of::<RawReadInfo>() as u32,
                buf.as_mut_ptr() as *mut std::ffi::c_void,
                2352, &mut bytes_returned, std::ptr::null_mut(),
            )
        };

        unsafe { CloseHandle(handle) };

        if ok == 0 || bytes_returned != 2352 {
            return Err(format!(
                "IOCTL_CDROM_RAW_READ at pregap offset -{} failed (bytes={})",
                offset, bytes_returned
            ));
        }

        Ok(buf)
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        unsafe fn CreateFileW(n: *const u16, a: u32, s: u32, sec: *mut std::ffi::c_void,
            d: u32, f: u32, t: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
        unsafe fn DeviceIoControl(h: *mut std::ffi::c_void, c: u32,
            ib: *const std::ffi::c_void, is: u32, ob: *mut std::ffi::c_void,
            os: u32, r: *mut u32, ov: *mut std::ffi::c_void) -> i32;
        unsafe fn CloseHandle(h: *mut std::ffi::c_void) -> i32;
    }
}

// TESTS
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_detection_all_zeros() {
        let sector = vec![0u8; 2352];
        assert!(is_silent(&sector));
    }

    #[test]
    fn silence_detection_real_audio() {
        let mut sector = vec![0u8; 2352];

        for b in &mut sector[..SILENCE_THRESHOLD_BYTES * 2] {
            *b = 0x42;
        }
        assert!(!is_silent(&sector));
    }

    #[test]
    fn silence_detection_just_below_threshold() {
        let mut sector = vec![0u8; 2352];
        for b in &mut sector[..SILENCE_THRESHOLD_BYTES - 1] {
            *b = 0xFF;
        }
        assert!(is_silent(&sector));
    }

    #[test]
    fn pregap_standard_lead_in_returns_min() {
        use crate::toc::{DiscToc, TrackInfo};
        let toc = DiscToc {
            tracks: vec![TrackInfo {
                number: 1,
                start_lba: 0,
                sector_count: 1000,
                duration_msf: (0, 13, 30),
            }],
            total_sectors: 1000,
        };
        assert_eq!(pregap_sector_count(&toc), STANDARD_PREGAP_SECTORS);
    }

    #[test]
    fn pregap_extended_returns_actual() {
        use crate::toc::{DiscToc, TrackInfo};
        let toc = DiscToc {
            tracks: vec![TrackInfo {
                number: 1,
                start_lba: 600,
                sector_count: 1000,
                duration_msf: (0, 13, 30),
            }],
            total_sectors: 1600,
        };
        assert_eq!(pregap_sector_count(&toc), 600);
    }

    #[test]
    fn htoa_status_display() {
        let s = HtoaStatus::HtoaDetected { sectors: 600, duration_secs: 8.0 };
        let display = s.to_string();
        assert!(display.contains("HTOA detected"));
        assert!(display.contains("600 sectors"));
    }

    #[test]
    fn htoa_threshold() {
        assert_eq!(HTOA_LIKELY_THRESHOLD_SECTORS, 450);
    }
}
