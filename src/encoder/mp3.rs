//! MP3 Encoder
//!
//! ## Why MP3?
//!
//! The archiving community rightly prefers FLAC/WAV for preservation, but MP3
//! still has real use cases: car stereos, older DAPs, smaller storage,
//! sharing with people who don't have a FLAC-capable player. We'll expose it as
//! an opt-in encoder so users can choose.
//!
//! ## Dependency note
//!
//! `mp3lame-encoder` requires `libmp3lame` on the system:
//! - Linux:   `apt install libmp3lame-dev` / `pacman -S lame`
//! - Windows: `scoop install lame` (recommended)
//! - macOS:   `brew install lame`
//!
//! ## Quality presets
//!
//! | Preset   | Bitrate  | VBR mode | Use case                  |
//! |----------|----------|----------|---------------------------|
//! | Mobile   | ~128kbps | VBR V6   | Space-constrained devices |
//! | Standard | ~192kbps | VBR V2   | General listening         |
//! | Extreme  | ~256kbps | VBR V0   | Transparent quality       |
//! | Cbr320   | 320kbps  | CBR      | Maximum compatibility     |
//! 
//! Reference:
//! https://wiki.hydrogenaud.io/index.php?title=MP3
//! 

use super::Encoder;
use crate::error::{CdripError, Result};
use mp3lame_encoder::{Builder, Bitrate, DualPcm, FlushNoGap, Id3Tag, Quality, VbrMode};
use std::{fs, io::Write, path::PathBuf};
use tracing::debug;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Mp3Preset {
    Mobile,
    Standard,
    Extreme,
    Cbr320,
}

impl Default for Mp3Preset {
    fn default() -> Self {
        Self::Standard
    }
}

impl std::fmt::Display for Mp3Preset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mp3Preset::Mobile   => write!(f, "VBR ~128kbps"),
            Mp3Preset::Standard => write!(f, "VBR ~192kbps"),
            Mp3Preset::Extreme  => write!(f, "VBR ~256kbps"),
            Mp3Preset::Cbr320   => write!(f, "CBR 320kbps"),
        }
    }
}

/// MP3 encoder — wraps `mp3lame-encoder` (LAME bindings).
///
/// LAME is an industry-standard encoder; we'll just use its Rust high-level API so
/// we never touch raw pointers in cdrip code! Yay! (*^▽^*)
pub struct Mp3Encoder {
    pub preset: Mp3Preset,
    pub id3: Option<Mp3Id3>,
}

#[derive(Debug, Clone, Default)]
pub struct Mp3Id3 {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub year: String,
    pub comment: String,
}

impl Default for Mp3Encoder {
    fn default() -> Self {
        Self {
            preset: Mp3Preset::Standard,
            id3: None,
        }
    }
}

impl Encoder for Mp3Encoder {
    fn encode(&self, track_num: u8, pcm_data: &[u8], output_path: &PathBuf) -> Result<PathBuf> {
        let mut builder = Builder::new()
            .map_err(|e| CdripError::FlacEncodeFailed(track_num, format!("LAME init: {}", e)))?;

        builder
            .set_num_channels(2)
            .map_err(|e| CdripError::FlacEncodeFailed(track_num, e.to_string()))?;
        builder
            .set_sample_rate(44_100)
            .map_err(|e| CdripError::FlacEncodeFailed(track_num, e.to_string()))?;
        builder
            .set_quality(Quality::Best)
            .map_err(|e| CdripError::FlacEncodeFailed(track_num, e.to_string()))?;

        match self.preset {
            Mp3Preset::Mobile => {
                builder.set_vbr_mode(VbrMode::Vbr)
                    .map_err(|e| CdripError::FlacEncodeFailed(track_num, e.to_string()))?;
                builder.set_vbr_quality(6)
                    .map_err(|e| CdripError::FlacEncodeFailed(track_num, e.to_string()))?;
            }
            Mp3Preset::Standard => {
                builder.set_vbr_mode(VbrMode::Vbr)
                    .map_err(|e| CdripError::FlacEncodeFailed(track_num, e.to_string()))?;
                builder.set_vbr_quality(2)
                    .map_err(|e| CdripError::FlacEncodeFailed(track_num, e.to_string()))?;
            }
            Mp3Preset::Extreme => {
                builder.set_vbr_mode(VbrMode::Vbr)
                    .map_err(|e| CdripError::FlacEncodeFailed(track_num, e.to_string()))?;
                builder.set_vbr_quality(0)
                    .map_err(|e| CdripError::FlacEncodeFailed(track_num, e.to_string()))?;
            }
            Mp3Preset::Cbr320 => {
                builder.set_brate(Bitrate::Kbps320)
                    .map_err(|e| CdripError::FlacEncodeFailed(track_num, e.to_string()))?;
            }
        }

        if let Some(id3) = &self.id3 {
            builder.set_id3_tag(Id3Tag {
                title:   id3.title.as_bytes(),
                artist:  id3.artist.as_bytes(),
                album:   id3.album.as_bytes(),
                year:    id3.year.as_bytes(),
                comment: id3.comment.as_bytes(),
            });
        }

        let mut encoder = builder
            .build()
            .map_err(|e| CdripError::FlacEncodeFailed(track_num, format!("LAME build: {}", e)))?;

        // Split PCM into left/right channels.
        // Raw CD PCM is interleaved: L0 R0 L1 R1 ... (2 bytes each = 4 bytes/frame).
        // LAME's `DualPcm` wants separate i16 slices for L and R.
        let frames = pcm_data.len() / 4;
        let mut left  = Vec::<i16>::with_capacity(frames);
        let mut right = Vec::<i16>::with_capacity(frames);

        for chunk in pcm_data.chunks_exact(4) {
            left.push( i16::from_le_bytes([chunk[0], chunk[1]]));
            right.push(i16::from_le_bytes([chunk[2], chunk[3]]));
        }

        let input = DualPcm {
            left:  &left,
            right: &right,
        };

        let mut mp3_buf: Vec<u8> = Vec::new();
        mp3_buf.reserve(mp3lame_encoder::max_required_buffer_size(frames));

        let encoded = encoder
            .encode(input, mp3_buf.spare_capacity_mut())
            .map_err(|e| CdripError::FlacEncodeFailed(track_num, format!("LAME encode: {}", e)))?;

        unsafe { mp3_buf.set_len(encoded) };

        // Flush the remaining encoder state (gapless info, etc.)
        let flushed = encoder
            .flush::<FlushNoGap>(mp3_buf.spare_capacity_mut())
            .map_err(|e| CdripError::FlacEncodeFailed(track_num, format!("LAME flush: {}", e)))?;

        unsafe { mp3_buf.set_len(mp3_buf.len() + flushed) };

        fs::write(output_path, &mp3_buf)
            .map_err(|e| CdripError::FileWriteFailed(output_path.display().to_string(), e))?;

        debug!(
            "MP3 track {:02}: {} PCM bytes → {} MP3 bytes ({}) → {}",
            track_num,
            pcm_data.len(),
            mp3_buf.len(),
            self.preset,
            output_path.display()
        );

        Ok(output_path.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_deinterleave_frame_count() {
        let pcm = vec![0u8; 8];
        let frames = pcm.len() / 4;
        let mut left  = Vec::<i16>::with_capacity(frames);
        let mut right = Vec::<i16>::with_capacity(frames);
        for chunk in pcm.chunks_exact(4) {
            left.push( i16::from_le_bytes([chunk[0], chunk[1]]));
            right.push(i16::from_le_bytes([chunk[2], chunk[3]]));
        }
        assert_eq!(left.len(),  2);
        assert_eq!(right.len(), 2);
    }

    #[test]
    fn pcm_deinterleave_values() {
        let pcm = vec![0x00u8, 0x01, 0x00, 0x02];
        let l = i16::from_le_bytes([pcm[0], pcm[1]]);
        let r = i16::from_le_bytes([pcm[2], pcm[3]]);
        assert_eq!(l, 256);
        assert_eq!(r, 512);
    }

    #[test]
    fn preset_display() {
        assert!(Mp3Preset::Standard.to_string().contains("192"));
        assert!(Mp3Preset::Cbr320.to_string().contains("320"));
    }
}
