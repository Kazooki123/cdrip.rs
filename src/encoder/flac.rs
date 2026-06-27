use super::Encoder;
use crate::error::{CdripError, Result};
use flac_codec::{
    byteorder::LittleEndian,
    encode::{FlacByteWriter, Options},
};
use std::{fs::File, io::{BufWriter, Write}, path::PathBuf};
use tracing::debug;

/// FLAC encoder — converts raw CD PCM into a lossless FLAC file.
/// Uses `flac-codec` v1.3.2 (pure Rust, RFC9639, no C deps).
/// Uses `FlacByteWriter::new_cdda()` which hardcodes 44100 Hz / 16-bit / stereo -
/// exactly the CD-DA spec, and accepts raw LE bytes directly via `std::io::Write`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Fast,
    Best,
    Default,
}

pub struct FlacEncoder {
    pub preset: Preset,
}

impl Default for FlacEncoder {
    fn default() -> Self {
        Self { preset: Preset::Default }
    }
}

impl Encoder for FlacEncoder {
    fn encode(&self, track_num: u8, pcm_data: &[u8], output_path: &PathBuf) -> Result<PathBuf> {
        let file = File::create(output_path).map_err(|e| {
            CdripError::FileWriteFailed(output_path.display().to_string(), e)
        })?;
        let writer = BufWriter::new(file);

        let options = match self.preset {
            Preset::Fast    => Options::fast(),
            Preset::Best    => Options::best(),
            Preset::Default => Options::default(),
        };

        // new_cdda() sets 44100 Hz / 16-bit / 2ch automatically.
        // total_bytes = None so the encoder doesn't enforce an exact byte count -
        // avoids the "too many samples" error if sector count drifts slightly from TOC estimate.
        let mut flac_writer = FlacByteWriter::<_, LittleEndian>::new_cdda(
            writer,
            options,
            None,
        )
        .map_err(|e| CdripError::FlacEncodeFailed(track_num, format!("init: {}", e)))?;

        // FlacByteWriter implements std::io::Write — feed raw LE PCM bytes directly.
        // No i32 conversion needed; the crate handles 16-bit LE natively.
        flac_writer.write_all(pcm_data).map_err(|e| {
            CdripError::FlacEncodeFailed(track_num, format!("write: {}", e))
        })?;

        flac_writer.finalize().map_err(|e| {
            CdripError::FlacEncodeFailed(track_num, format!("finalize: {}", e))
        })?;

        debug!(
            "FLAC track {:02}: encoded {} PCM bytes → {}",
            track_num,
            pcm_data.len(),
            output_path.display()
        );

        Ok(output_path.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use flac_codec::{byteorder::LittleEndian, encode::{FlacByteWriter, Options}};

    #[test]
    fn cdda_writer_roundtrip_silent() {
        // Encode 100 frames of silence (200 samples * 2 bytes = 400 bytes)
        let pcm = vec![0u8; 400];
        let mut out = Cursor::new(vec![]);

        let mut w = FlacByteWriter::<_, LittleEndian>::new_cdda(
            &mut out,
            Options::fast(),
            None,
        )
        .unwrap();

        w.write_all(&pcm).unwrap();
        w.finalize().unwrap();

        assert!(out.into_inner().starts_with(b"fLaC"));
    }
}
