//! CD-TEXT reading via raw SCSI subchannel data.
//!
//! CD-TEXT is stored in the lead-in area of a disc in R-W subchannels.
//! The `cd-da-reader` crate explicitly does not support it (unreliable on
//! many drives), so this module implements a best-effort parser on top of
//! platform-specific SCSI commands where available.
//!
//! **RELIABILITY WARNING** (same as real-world tooling):
//! Many drives don't support CD-TEXT at all. Many that DO return garbled data...
//! Many CDs (including most modern pressings) don't have CD-TEXT encoded.
//! This module ALWAYS returns `Ok(None)` gracefully when the data isn't there.
//!
//! Platform support:
//! - Linux: READ TOC/PMA/ATIP command (SCSI opcode 0x43, format 5)
//! - Windows: IOCTL_CDROM_READ_TOC_EX with Format = CDROM_READ_TOC_EX_FORMAT_CDTEXT
//! - macOS: Not reliably supported — returns None
//!
//! References:
//! - MMC-3 spec §6.26 READ TOC/PMA/ATIP, Format 5 (*^▽^*)
//! - https://wiki.hydrogenaud.io/index.php?title=CD-Text

#[allow(unused)]
#[allow(dead_code)]

use tracing::{debug, warn};

#[derive(Debug, Clone, Default)]
pub struct CdTextData {
    pub album_title: Option<String>,
    pub album_artist: Option<String>,
    pub track_titles: Vec<Option<String>>,
    pub track_artists: Vec<Option<String>>,
}

impl CdTextData {
    pub fn has_data(&self) -> bool {
        self.album_title.is_some()
            || self.album_artist.is_some()
            || self.track_titles.iter().any(|t| t.is_some())
            || self.track_artists.iter().any(|t| t.is_some())
    }
}
// Entry point
/// Try to read CD-TEXT from the drive at `device_path`.
/// Returns `Ok(Some(data))` if CD-TEXT was found and parsed,
/// `Ok(None)` if the drive doesn't support it or the disc has no CD-TEXT,
/// `Err(_)` only on hard I/O errors.
pub fn read_cd_text(device_path: &str, track_count: u8) -> Result<Option<CdTextData>, CdTextError> {
    debug!("Attempting CD-TEXT read from {}", device_path);

    #[cfg(target_os = "linux")]
    {
        match linux::read_cd_text_raw(device_path) {
            Ok(raw) if !raw.is_empty() => {
                return Ok(parse_cd_text_pack(&raw, track_count));
            }
            Ok(_) => {
                debug!("CD-TEXT raw read returned empty — disc likely has no CD-TEXT");
                return Ok(None);
            }
            Err(e) => {
                warn!("CD-TEXT SCSI command failed (drive may not support it): {}", e);
                return Ok(None);
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        match windows::read_cd_text_raw(device_path) {
            Ok(raw) if !raw.is_empty() => {
                return Ok(parse_cd_text_pack(&raw, track_count));
            }
            Ok(_) => {
                debug!("CD-TEXT raw read returned empty");
                return Ok(None);
            }
            Err(e) => {
                warn!("CD-TEXT IOCTL failed: {}", e);
                return Ok(None);
            }
        }
    }

    // macOS / unknown platform - not supported...
    #[allow(unreachable_code)]
    {
        debug!("CD-TEXT reading not supported on this platform");
        Ok(None)
    }
}

// CD-TEXT pack parser
/// Parse raw CD-TEXT pack data (18-byte packets from the SCSI response).
///
/// Pack structure (MMC-3 §6.26):
/// ```
///  0       Pack type  (0x80=TITLE, 0x81=PERFORMER, 0x82=SONGWRITER, ...)
///  1       Track number (0 = disc level)
///  2       Sequence number
///  3       Block/character set info
///  4..15   Text data (12 bytes, UTF-8 or ISO-8859-1)
///  16..17  CRC (we don't verify — drives often return bad CRCs)
/// ```
fn parse_cd_text_pack(raw: &[u8], track_count: u8) -> Option<CdTextData> {
    // Raw response starts with a 4-byte header (length field) on some platforms
    // Skip the first 4 bytes if the length field is present
    let packs = if raw.len() >= 4 && (raw[0] as usize * 256 + raw[1] as usize + 2) == raw.len() {
        &raw[4..]
    } else {
        raw
    };

    if packs.len() < 18 {
        return None;
    }

    let mut data = CdTextData {
        track_titles: vec![None; track_count as usize + 1],
        track_artists: vec![None; track_count as usize + 1],
        ..Default::default()
    };

    let mut found_any = false;

    let mut title_buf: Vec<(u8, String)> = Vec::new();
    let mut artist_buf: Vec<(u8, String)> = Vec::new();

    for pack in packs.chunks_exact(18) {
        let pack_type = pack[0];
        let track_num = pack[1];
        let text_bytes = &pack[4..16];

        // Only handle TITLE (0x80) and PERFORMER (0x81) for now
        if pack_type != 0x80 && pack_type != 0x81 {
            continue;
        }

        let text = decode_cdtext_string(text_bytes);
        if text.is_empty() {
            continue;
        }

        found_any = true;

        match pack_type {
            0x80 => title_buf.push((track_num, text)),
            0x81 => artist_buf.push((track_num, text)),
            _ => {}
        }
    }

    if !found_any {
        return None;
    }

    // Commit accumulated text into the data struct
    for (track_num, text) in title_buf {
        if track_num == 0 {
            data.album_title = Some(text);
        } else if (track_num as usize) <= data.track_titles.len() {
            data.track_titles[(track_num - 1) as usize] = Some(text);
        }
    }
    for (track_num, text) in artist_buf {
        if track_num == 0 {
            data.album_artist = Some(text);
        } else if (track_num as usize) <= data.track_artists.len() {
            data.track_artists[(track_num - 1) as usize] = Some(text);
        }
    }

    if data.has_data() { Some(data) } else { None }
}

/// Decode a CD-TEXT text field from raw bytes.
/// Strips null terminators and non-printable control chars.
fn decode_cdtext_string(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take_while(|&&b| b != 0)
        .filter(|&&b| b >= 0x20 || b == 0x09)
        .map(|&b| b as char)
        .collect::<String>()
        .trim()
        .to_string()
}

// Linux SCSI implementation
#[cfg(target_os = "linux")]
mod linux {
    use std::fs::OpenOptions;
    use std::os::unix::io::AsRawFd;

    pub fn read_cd_text_raw(device: &str) -> Result<Vec<u8>, String> {
        use std::io;

        let file = OpenOptions::new()
            .read(true)
            .open(device)
            .map_err(|e| format!("open {}: {}", device, e))?;

        let fd = file.as_raw_fd();

        // SCSI CDB: READ TOC/PMA/ATIP, Format 5 (CD-Text), 2-second lead-in
        // Allocation length = 0x8000 (32 KiB) — max CD-TEXT size
        let mut cdb = [0u8; 10];
        cdb[0] = 0x43;  // READ TOC/PMA/ATIP
        cdb[1] = 0x02;  // MSF bit
        cdb[2] = 0x05;  // Format: CD-TEXT
        cdb[7] = 0x80;  // Allocation length high byte (32768)
        cdb[8] = 0x00;

        let mut buf = vec![0u8; 32768];
        let buf_ptr = buf.as_mut_ptr();
        let buf_len = buf.len() as u32;

        // sg_io_hdr_t — Linux SCSI generic v3 interface
        // We'll use the simplified repr to avoid a full sg_io dependency
        #[repr(C)]
        struct SgIoHdr {
            interface_id:    i32,  // 'S'
            dxfer_direction: i32,  // SG_DXFER_FROM_DEV = -3
            cmd_len:         u8,
            mx_sb_len:       u8,
            iovec_count:     u16,
            dxfer_len:       u32,
            dxferp:          *mut u8,
            cmdp:            *const u8,
            sbp:             *mut u8,
            timeout:         u32,
            flags:           u32,
            pack_id:         i32,
            usr_ptr:         *mut std::ffi::c_void,
            status:          u8,
            masked_status:   u8,
            msg_status:      u8,
            sb_len_wr:       u8,
            host_status:     u16,
            driver_status:   u16,
            resid:           i32,
            duration:        u32,
            info:            u32,
        }

        let mut sense = [0u8; 32];
        let hdr = SgIoHdr {
            interface_id:    b'S' as i32,
            dxfer_direction: -3,
            cmd_len:         10,
            mx_sb_len:       32,
            iovec_count:     0,
            dxfer_len:       buf_len,
            dxferp:          buf_ptr,
            cmdp:            cdb.as_ptr(),
            sbp:             sense.as_mut_ptr(),
            timeout:         5000,
            flags:           0,
            pack_id:         0,
            usr_ptr:         std::ptr::null_mut(),
            status:          0,
            masked_status:   0,
            msg_status:      0,
            sb_len_wr:       0,
            host_status:     0,
            driver_status:   0,
            resid:           0,
            duration:        0,
            info:            0,
        };

        // 0x2285
        let ret = unsafe {
            libc_ioctl(fd, 0x2285, &hdr as *const SgIoHdr)
        };

        if ret != 0 {
            return Err(format!("SG_IO ioctl failed: {}", io::Error::last_os_error()));
        }
        if hdr.status != 0 {
            return Err(format!("SCSI status: 0x{:02x}", hdr.status));
        }

        // First 2 bytes = data length
        if buf.len() < 4 {
            return Ok(Vec::new());
        }
        let data_len = (buf[0] as usize * 256 + buf[1] as usize + 2).min(buf.len());
        Ok(buf[..data_len].to_vec())
    }

    extern "C" {
        fn ioctl(fd: i32, request: u64, ...) -> i32;
    }

    unsafe fn libc_ioctl(fd: i32, request: u64, arg: *const impl Sized) -> i32 {
        ioctl(fd, request, arg)
    }
}

// Windows IOCTL implementation
#[cfg(target_os = "windows")]
mod windows {
    pub fn read_cd_text_raw(device: &str) -> Result<Vec<u8>, String> {
        use std::os::windows::io::AsRawHandle;

        let path_wide: Vec<u16> = device.encode_utf16().chain(std::iter::once(0)).collect();

        let handle = unsafe {
            CreateFileW(
                path_wide.as_ptr(),
                0x80000000, // GENERIC_READ
                0x3,         // FILE_SHARE_READ | FILE_SHARE_WRITE
                std::ptr::null_mut(),
                3,            // OPEN_EXISTING
                0,
                std::ptr::null_mut(),
            )
        };

        if handle.is_null() || handle == usize::MAX as *mut std::ffi::c_void {
            return Err(format!("CreateFileW failed for {}", device));
        }

        #[repr(C)]
        struct CdromReadTocEx {
            format: u8,
            reserved1: u8,
            reserved2: u8,
            session_track: u8,
            reserved3: [u8; 3],
            msf: u8,
        }

        let req = CdromReadTocEx {
            format: 0x05,
            reserved1: 0,
            reserved2: 0,
            session_track: 0,
            reserved3: [0; 3],
            msf: 0,
        };

        let mut buf = vec![0u8; 32768];
        let mut bytes_returned: u32 = 0;

        // IOCTL_CDROM_READ_TOC_EX = 0x00024054
        let ok = unsafe {
            DeviceIoControl(
                handle,
                0x00024054,
                &req as *const _ as *const std::ffi::c_void,
                std::mem::size_of::<CdromReadTocEx>() as u32,
                buf.as_mut_ptr() as *mut std::ffi::c_void,
                buf.len() as u32,
                &mut bytes_returned,
                std::ptr::null_mut(),
            )
        };

        unsafe { CloseHandle(handle) };

        if ok == 0 || bytes_returned < 4 {
            return Ok(Vec::new()); // No CD-TEXT
        }

        Ok(buf[..bytes_returned as usize].to_vec())
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        unsafe fn CreateFileW(
            name: *const u16, access: u32, share: u32,
            sec: *mut std::ffi::c_void, disp: u32, flags: u32,
            tmpl: *mut std::ffi::c_void,
        ) -> *mut std::ffi::c_void;

        unsafe fn DeviceIoControl(
            handle: *mut std::ffi::c_void,
            code: u32,
            in_buf: *const std::ffi::c_void,
            in_size: u32,
            out_buf: *mut std::ffi::c_void,
            out_size: u32,
            returned: *mut u32,
            overlapped: *mut std::ffi::c_void,
        ) -> i32;

        unsafe fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    }
}

// ERR TYPES
#[derive(Debug)]
pub enum CdTextError {
    Io(std::io::Error),
    NotSupported,
}

impl std::fmt::Display for CdTextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CdTextError::Io(e) => write!(f, "I/O error: {}", e),
            CdTextError::NotSupported => write!(f, "CD-TEXT not supported on this platform"),
        }
    }
}

// TESTS
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_cdtext_string_basic() {
        let bytes = b"Nevermind\x00\x00\x00";
        assert_eq!(decode_cdtext_string(bytes), "Nevermind");
    }

    #[test]
    fn decode_cdtext_string_all_null() {
        let bytes = b"\x00\x00\x00\x00";
        assert_eq!(decode_cdtext_string(bytes), "");
    }

    #[test]
    fn decode_cdtext_string_no_null() {
        let bytes = b"ABCDEFGHIJKL"; // 12 bytes, no null
        assert_eq!(decode_cdtext_string(bytes), "ABCDEFGHIJKL");
    }

    #[test]
    fn cdtext_data_has_data_empty() {
        let d = CdTextData::default();
        assert!(!d.has_data());
    }

    #[test]
    fn cdtext_data_has_data_with_title() {
        let d = CdTextData {
            album_title: Some("Nevermind".into()),
            ..Default::default()
        };
        assert!(d.has_data());
    }
}
