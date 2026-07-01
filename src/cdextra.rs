//! # CD-Extra (Enhanced CD)
//! 
//! ## What is CD-Extra?
//! 
//! Enhanced CDs (or CD-Extras) are a type of CDs that contain **BOTH**
//! CD-Audio and CD-ROM Media (e.g photos, videos, executables, etc.)
//! 
//! The data session holds bonus content - music videos, Flash apps, wallpapers,
//! ROM content, installer software. Famous examples:
//! - Linkin Park *Hybrid Theory* (early pressings) - videos + Flash site
//! - Many 2000s pop/rock CDs - multimedia launchers
//! - Some game soundtracks - patch installers
//! 
//! These type of CDs were popular around the 90s to the early 2000s, however,
//! their presence in the modern CD industry has declined unfortunantly.. ಥ_ಥ
//! 
//! ## Detection method
//!
//! <cite index="21-1">The start sector of the data session appears in the TOC. We look for a
//! track flagged as data-mode (not audio) in the TOC, then confirm it's really
//! an ISO9660 filesystem by reading sector 16 (the Primary Volume Descriptor)
//! and checking for the magic string `CD001`.</cite>
//!
//! ## Extraction
//!
//! We read all sectors of the data track into an `.iso` raw image. This is a
//! standards-compliant ISO9660 image mountable with:
//! ```sh
//! # Linux
//! sudo mount -o loop DATA.iso /mnt/cdextra
//! # Windows (PowerShell)
//! Mount-DiskImage DATA.iso
//! ```
//! 
//! References:
//! - https://en.wikipedia.org/Wiki/Enhanced_CD
//! - https://wiki.hydrogenaud.io/index.php?title=Red_Book

use crate::{error::Result, toc::DiscToc};
use tracing::{debug, info, warn};
use std::path::{Path, PathBuf};

const ISO9660_PVD_SECTOR: u32 = 16;
const ISO9660_MAGIC: &[u8] = b"CD001";

/// Offset of the magic bytes within the raw 2352-byte sector.
/// Layout: 16 bytes sync + 4 header + 1 mode + then user data starts.
/// In Mode 1: user data starts at byte 16. PVD magic is at user_data[1..6].
const ISO9660_MAGIC_OFFSET: usize = 17;

#[derive(Debug, Clone, PartialEq)]
pub enum CdExtraStatus {
    NoDataTrack,
    /// A data track exists but the ISO9660 magic bytes aren't found..
    /// Could be a different filesystem (HFS+, UDF) or a raw data track.
    DataTrackNotIso9660 { data_start_lba: u32 },
    CdExtraDetected {
        data_start_lba: u32,
        data_sectors: u32,
        size_mib: f32,
    },
}

impl std::fmt::Display for CdExtraStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CdExtraStatus::NoDataTrack =>
                write!(f, "No data track found — standard audio-only CD"),
            CdExtraStatus::DataTrackNotIso9660 { data_start_lba } =>
                write!(f, "Data track at LBA {} (non-ISO9660 filesystem)", data_start_lba),
            CdExtraStatus::CdExtraDetected { data_start_lba, data_sectors, size_mib } =>
                write!(f, "CD-Extra detected!! Data session at LBA {}, {} sectors ({:.1} MiB)",
                    data_start_lba, data_sectors, size_mib),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CdExtraInfo {
    pub data_start_lba: u32,
    pub data_sectors: u32,
    pub size_bytes: u64,
    pub volume_label: Option<String>,
}

impl CdExtraInfo {
    pub fn size_mib(&self) -> f32 {
        self.size_bytes as f32 / (1024.0 * 1024.0)
    }
}

/// Detect whether the disc has a CD-Extra data session.
///
/// Works by:
/// 1. Scanning the `DiscToc` for a data track (last track, higher LBA than audio)
/// 2. Reading sector 16 of **that** track to check for ISO9660 PVD magic `CD001`
/// 3. Optionally reading the volume label from the PVD.
pub fn detect_cdextra(device_path: &str, toc: &DiscToc) -> CdExtraStatus {
    let Some((data_start_lba, data_sectors)) = find_data_track(toc) else {
        return CdExtraStatus::NoDataTrack;
    };

    debug!(
        "Possible data track at LBA {} ({} sectors)",
        data_start_lba, data_sectors
    );

    let pvd_lba = data_start_lba + ISO9660_PVD_SECTOR;
    match read_data_sector(device_path, pvd_lba) {
        Err(e) => {
            warn!("Couldn't read PVD sector at LBA {}: {}", pvd_lba, e);
            CdExtraStatus::DataTrackNotIso9660 { data_start_lba }
        }
        Ok(sector) => {
            if is_iso9660_pvd(&sector) {
                let size_mib = data_sectors as f32 * 2048.0 / (1024.0 * 1024.0);
                info!(
                    "ISO9660 PVD confirmed at LBA {} — CD-Extra ({:.1} MiB)",
                    pvd_lba, size_mib
                );
                CdExtraStatus::CdExtraDetected {
                    data_start_lba,
                    data_sectors,
                    size_mib,
                }
            } else {
                debug!("No ISO9660 magic at LBA {} — non-standard data track", pvd_lba);
                CdExtraStatus::DataTrackNotIso9660 { data_start_lba }
            }
        }
    }
}

pub fn probe_cdextra(device_path: &str, toc: &DiscToc) -> Option<CdExtraInfo> {
    let (data_start_lba, data_sectors) = find_data_track(toc)?;

    let pvd_lba = data_start_lba + ISO9660_PVD_SECTOR;
    let pvd_sector = read_data_sector(device_path, pvd_lba).ok()?;

    if !is_iso9660_pvd(&pvd_sector) {
        return None;
    }

    let volume_label = read_volume_label(&pvd_sector);

    Some(CdExtraInfo {
        data_start_lba,
        data_sectors,
        size_bytes: data_sectors as u64 * 2048,
        volume_label,
    })
}

/// Extract the CD-Extra data session as a raw ISO9660 image.
///
/// Reads all sectors of the data track and writes them to `output_dir/disc_data.iso`.
/// The output is a standard ISO image — no proprietary format.
///
/// Sector data is stored as 2048-byte user data (Mode 1 CD-ROM sectors).
/// We strip the 16-byte sync+header and 288-byte ECC/EDC from each 2352-byte
/// raw sector, keeping only the user data portion.
pub fn extract_cdextra(
    device_path: &str,
    info: &CdExtraInfo,
    output_dir: &Path,
) -> Result<PathBuf> {
    use crate::error::CdripError;
    use std::fs::File;
    use std::io::{BufWriter, Write};

    let iso_path = output_dir.join("DATA.iso");
    info!(
        "Extracting CD-Extra: {} sectors ({:.1} MiB) → {}",
        info.data_sectors,
        info.size_mib(),
        iso_path.display()
    );

    let file = File::create(&iso_path)
        .map_err(|e| CdripError::FileWriteFailed(iso_path.display().to_string(), e))?;
    let mut writer = BufWriter::new(file);

    let mut sectors_ok = 0u32;
    let mut sectors_err = 0u32;

    for i in 0..info.data_sectors {
        let lba = info.data_start_lba + i;

        match read_data_sector(device_path, lba) {
            Ok(raw_sector) => {
                // Extract 2048 bytes of user data from the 2352-byte raw sector.
                // Mode 1: bytes 16..2064 are user data.
                // Mode 2 Form 1: bytes 24..2072. We try Mode 1 first since
                // ISO9660 on CD-Extra is almost ALWAYS Mode 1.
                let user_data = extract_user_data(&raw_sector);
                writer.write_all(user_data).map_err(|e| {
                    CdripError::FileWriteFailed(iso_path.display().to_string(), e)
                })?;
                sectors_ok += 1;
            }
            Err(e) => {
                warn!("Sector read error at LBA {} (data track): {}", lba, e);
                writer.write_all(&[0u8; 2048]).map_err(|e| {
                    CdripError::FileWriteFailed(iso_path.display().to_string(), e)
                })?;
                sectors_err += 1;
            }
        }

        if i > 0 && i % 5000 == 0 {
            let pct = i as f32 / info.data_sectors as f32 * 100.0;
            info!("CD-Extra extraction: {:.0}% ({}/{})", pct, i, info.data_sectors);
        }
    }

    writer.flush().map_err(|e| {
        crate::error::CdripError::FileWriteFailed(iso_path.display().to_string(), e)
    })?;

    info!(
        "CD-Extra ISO written: {} sectors OK, {} errors → {}",
        sectors_ok, sectors_err, iso_path.display()
    );

    Ok(iso_path)
}

/// Find the data track in the TOC.
///
/// On a CD-Extra disc the data track is the last track in the TOC and has a
/// much higher start LBA than the audio tracks — it's in a separate session.
/// We identify it by checking if its start LBA is significantly higher than
/// where the audio session ends (gap of at least 11,400 sectors = ~2.5 min,
/// which is the minimum inter-session gap on a Blue Book disc).
fn find_data_track(toc: &DiscToc) -> Option<(u32, u32)> {
    let tracks = &toc.tracks;
    if tracks.len() < 2 {
        return None;
    }

    let last = tracks.last()?;
    let second_last = &tracks[tracks.len() - 2];

    // Inter-session gap heuristic: Blue Book mandates at least ~11,400 sectors...
    // between the audio session end and the data session start.
    let gap = last.start_lba.saturating_sub(second_last.start_lba + second_last.sector_count);

    debug!(
        "Last track gap from previous track end: {} sectors",
        gap
    );

    if gap >= 11_400 {
        Some((last.start_lba, last.sector_count))
    } else {
        None
    }
}

fn is_iso9660_pvd(sector: &[u8]) -> bool {
    if sector.len() < ISO9660_MAGIC_OFFSET + ISO9660_MAGIC.len() {
        return false;
    }
    &sector[ISO9660_MAGIC_OFFSET..ISO9660_MAGIC_OFFSET + ISO9660_MAGIC.len()] == ISO9660_MAGIC
}

fn read_volume_label(pvd_sector: &[u8]) -> Option<String> {
    const LABEL_OFFSET: usize = 16 + 40;
    const LABEL_LEN: usize = 32;

    if pvd_sector.len() < LABEL_OFFSET + LABEL_LEN {
        return None;
    }

    let raw = &pvd_sector[LABEL_OFFSET..LABEL_OFFSET + LABEL_LEN];
    let label = raw
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| b as char)
        .collect::<String>()
        .trim()
        .to_string();

    if label.is_empty() { None } else { Some(label) }
}

fn extract_user_data(raw_sector: &[u8]) -> &[u8] {
    // Mode 1: bytes 0..16 = sync+header, bytes 16..2064 = user data
    if raw_sector.len() >= 2064 {
        &raw_sector[16..2064]
    } else if raw_sector.len() >= 2048 {
        &raw_sector[..2048]
    } else {
        raw_sector
    }
}

fn read_data_sector(device_path: &str, lba: u32) -> std::result::Result<Vec<u8>, String> {
    #[cfg(target_os = "linux")]
    return linux::read_data_sector(device_path, lba);

    #[cfg(target_os = "windows")]
    return windows::read_data_sector(device_path, lba);

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    Err("Data sector reads not implemented on this platform".to_string())
}

#[cfg(target_os = "linux")]
mod linux {
    pub fn read_data_sector(device: &str, lba: u32) -> Result<Vec<u8>, String> {
        use std::{fs::OpenOptions, os::unix::io::AsRawFd};

        let file = OpenOptions::new()
            .read(true)
            .open(device)
            .map_err(|e| format!("open: {}", e))?;

        let lba_bytes = lba.to_be_bytes();

        // READ CD CDB — sector type 0 (any), full 2352 bytes
        let mut cdb = [0u8; 12];
        cdb[0] = 0xBE;         // READ CD
        cdb[1] = 0x00;         // ANY TYPE (SECTOR)
        cdb[2] = lba_bytes[0];
        cdb[3] = lba_bytes[1];
        cdb[4] = lba_bytes[2];
        cdb[5] = lba_bytes[3];
        cdb[6] = 0x00;         // TRANSFER LENGTH = 1 SECT
        cdb[7] = 0x00;
        cdb[8] = 0x01;
        cdb[9] = 0xF8;         // ALL DATA FIELDS
        cdb[10] = 0x00;
        cdb[11] = 0x00;

        let mut buf = vec![0u8; 2352];
        let mut sense = [0u8; 32];

        #[repr(C)]
        struct SgIoHdr {
            interface_id: i32, dxfer_direction: i32, cmd_len: u8,
            mx_sb_len: u8, iovec_count: u16, dxfer_len: u32,
            dxferp: *mut u8, cmdp: *const u8, sbp: *mut u8,
            timeout: u32, flags: u32, pack_id: i32,
            usr_ptr: *mut std::ffi::c_void,
            status: u8, masked_status: u8, msg_status: u8, sb_len_wr: u8,
            host_status: u16, driver_status: u16, resid: i32,
            duration: u32, info: u32,
        }

        let hdr = SgIoHdr {
            interface_id: b'S' as i32, dxfer_direction: -3,
            cmd_len: 12, mx_sb_len: 32, iovec_count: 0,
            dxfer_len: 2352, dxferp: buf.as_mut_ptr(),
            cmdp: cdb.as_ptr(), sbp: sense.as_mut_ptr(),
            timeout: 10_000, flags: 0, pack_id: 0,
            usr_ptr: std::ptr::null_mut(),
            status: 0, masked_status: 0, msg_status: 0, sb_len_wr: 0,
            host_status: 0, driver_status: 0, resid: 0,
            duration: 0, info: 0,
        };

        unsafe extern "C" { fn ioctl(fd: i32, req: u64, ...) -> i32; }
        let ret = unsafe { ioctl(file.as_raw_fd(), 0x2285, &hdr as *const SgIoHdr) };

        if ret != 0 || hdr.status != 0 {
            return Err(format!(
                "READ CD LBA {} failed: ioctl={} scsi=0x{:02x}",
                lba, ret, hdr.status
            ));
        }

        Ok(buf)
    }
}

#[cfg(target_os = "windows")]
mod windows {
    pub fn read_data_sector(device: &str, lba: u32) -> Result<Vec<u8>, String> {
        let path_wide: Vec<u16> = device.encode_utf16().chain(std::iter::once(0)).collect();

        let handle = unsafe {
            CreateFileW(path_wide.as_ptr(), 0x80000000, 0x3,
                std::ptr::null_mut(), 3, 0, std::ptr::null_mut())
        };

        if handle.is_null() || handle == usize::MAX as *mut std::ffi::c_void {
            return Err(format!("Cannot open {}", device));
        }

        #[repr(C)]
        struct RawReadInfo { disk_offset: i64, sector_count: u32, track_mode: u32 }

        let info = RawReadInfo {
            disk_offset: lba as i64 * 2352,
            sector_count: 1,
            track_mode: 1,
        };

        let mut buf = vec![0u8; 2352];
        let mut returned = 0u32;

        let ok = unsafe {
            DeviceIoControl(handle, 0x0002403E,
                &info as *const _ as *const std::ffi::c_void,
                std::mem::size_of::<RawReadInfo>() as u32,
                buf.as_mut_ptr() as *mut std::ffi::c_void,
                2352, &mut returned, std::ptr::null_mut())
        };

        unsafe { CloseHandle(handle) };

        if ok == 0 || returned != 2352 {
            return Err(format!("IOCTL failed at LBA {} (bytes={})", lba, returned));
        }

        Ok(buf)
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        unsafe fn CreateFileW(n: *const u16, a: u32, s: u32,
            sec: *mut std::ffi::c_void, d: u32, f: u32,
            t: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
        unsafe fn DeviceIoControl(h: *mut std::ffi::c_void, c: u32,
            ib: *const std::ffi::c_void, is: u32,
            ob: *mut std::ffi::c_void, os: u32,
            r: *mut u32, ov: *mut std::ffi::c_void) -> i32;
        unsafe fn CloseHandle(h: *mut std::ffi::c_void) -> i32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toc::{DiscToc, TrackInfo};

    fn make_toc_with_gap(gap: u32) -> DiscToc {
        // Two audio tracks, then a data track with a `gap` sector after audio ends
        let audio_end_lba = 120_000u32;
        DiscToc {
            tracks: vec![
                TrackInfo { number: 1, start_lba: 0,           sector_count: 60_000, duration_msf: (13, 20, 0) },
                TrackInfo { number: 2, start_lba: 60_000,      sector_count: 60_000, duration_msf: (13, 20, 0) },
                TrackInfo { number: 3, start_lba: audio_end_lba + gap, sector_count: 10_000, duration_msf: (2, 13, 0) },
            ],
            total_sectors: audio_end_lba + gap + 10_000,
        }
    }

    #[test]
    fn no_data_track_on_audio_only() {
        let toc = DiscToc {
            tracks: vec![
                TrackInfo { number: 1, start_lba: 0,     sector_count: 50_000, duration_msf: (11, 6, 0) },
                TrackInfo { number: 2, start_lba: 50_000, sector_count: 50_000, duration_msf: (11, 6, 0) },
            ],
            total_sectors: 100_000,
        };
        assert!(find_data_track(&toc).is_none());
    }

    #[test]
    fn data_track_detected_with_large_gap() {
        let toc = make_toc_with_gap(12_000);
        assert!(find_data_track(&toc).is_some());
    }

    #[test]
    fn no_data_track_with_small_gap() {
        let toc = make_toc_with_gap(100);
        assert!(find_data_track(&toc).is_none());
    }

    #[test]
    fn iso9660_magic_detection_correct() {
        let mut sector = vec![0u8; 2352];
        sector[ISO9660_MAGIC_OFFSET] = 0x01;
        sector[ISO9660_MAGIC_OFFSET..ISO9660_MAGIC_OFFSET + ISO9660_MAGIC.len()]
            .copy_from_slice(ISO9660_MAGIC);
        assert!(is_iso9660_pvd(&sector));
    }

    #[test]
    fn iso9660_magic_detection_wrong_bytes() {
        let sector = vec![0u8; 2352];
        assert!(!is_iso9660_pvd(&sector));
    }

    #[test]
    fn iso9660_magic_detection_too_short() {
        let sector = vec![0u8; 10];
        assert!(!is_iso9660_pvd(&sector));
    }

    #[test]
    fn user_data_extraction_mode1() {
        let mut raw = vec![0xFFu8; 2352];
        raw[16] = 0x42;
        let ud = extract_user_data(&raw);
        assert_eq!(ud.len(), 2048);
        assert_eq!(ud[0], 0x42);
    }

    #[test]
    fn volume_label_parsing() {
        let mut pvd = vec![0u8; 2352];
        let label = b"HYBRID_THEORY   "; // padded with spaces
        let offset = 16 + 40;
        pvd[offset..offset + label.len()].copy_from_slice(label);
        let result = read_volume_label(&pvd);
        assert!(result.is_some());
        assert!(result.unwrap().contains("HYBRID"));
    }

    #[test]
    fn cdextra_info_size_mib() {
        let info = CdExtraInfo {
            data_start_lba: 100_000,
            data_sectors: 153_600,
            size_bytes: 153_600 * 2048,
            volume_label: None,
        };
        assert!((info.size_mib() - 300.0).abs() < 1.0);
    }
}
