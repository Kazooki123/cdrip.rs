use super::{Encoder, CD_BITS_PER_SAMPLE, CD_CHANNELS, CD_SAMPLE_RATE};
use crate::error::{CdripError, Result};
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::PathBuf,
};
use tracing::debug;

/// WAV encoder — prepends a standard RIFF/WAVE header to raw PCM data.
/// No external dependencies; the WAV format is simple enough that we hand-roll
/// the 44-byte header in little-endian format per the PCM WAV spec.
pub struct WavEncoder;

impl Encoder for WavEncoder {
    fn encode(&self, track_num: u8, pcm_data: &[u8], output_path: &PathBuf) -> Result<PathBuf> {
        let header = build_wav_header(pcm_data.len() as u32);

        let file = File::create(output_path).map_err(|e| {
            CdripError::FileWriteFailed(output_path.display().to_string(), e)
        })?;
        let mut writer = BufWriter::new(file);

        writer.write_all(&header).map_err(|e| {
            CdripError::WavEncodeFailed(track_num, format!("header write: {}", e))
        })?;
        writer.write_all(pcm_data).map_err(|e| {
            CdripError::WavEncodeFailed(track_num, format!("pcm write: {}", e))
        })?;
        writer.flush().map_err(|e| {
            CdripError::WavEncodeFailed(track_num, format!("flush: {}", e))
        })?;

        debug!(
            "WAV track {:02}: wrote {} bytes ({} PCM + 44 header) → {}",
            track_num,
            pcm_data.len() + 44,
            pcm_data.len(),
            output_path.display()
        );

        Ok(output_path.clone())
    }
}

// RIFF/WAV header construction
/// Build the 44-byte canonical PCM WAV header.
///
/// Layout (all values little-endian):
/// ```
/// Offset  Size  Field
///  0       4    "RIFF"
///  4       4    ChunkSize  = 36 + data_len
///  8       4    "WAVE"
/// 12       4    "fmt "
/// 16       4    Subchunk1Size = 16  (PCM)
/// 20       2    AudioFormat   = 1   (PCM linear)
/// 22       2    NumChannels   = 2
/// 24       4    SampleRate    = 44100
/// 28       4    ByteRate      = SampleRate * NumChannels * BitsPerSample/8
/// 32       2    BlockAlign    = NumChannels * BitsPerSample/8
/// 34       2    BitsPerSample = 16
/// 36       4    "data"
/// 40       4    Subchunk2Size = data_len
/// ```
fn build_wav_header(data_len: u32) -> [u8; 44] {
    let channels = CD_CHANNELS as u32;
    let sample_rate = CD_SAMPLE_RATE;
    let bits = CD_BITS_PER_SAMPLE as u32;
    let byte_rate = sample_rate * channels * bits / 8;
    let block_align = (channels * bits / 8) as u16;
    let chunk_size = 36u32.wrapping_add(data_len);

    let mut h = [0u8; 44];

    // RIFF chunk descriptor
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&chunk_size.to_le_bytes());
    h[8..12].copy_from_slice(b"WAVE");

    // fmt sub-chunk
    h[12..16].copy_from_slice(b"fmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());          // PCM subchunk size
    h[20..22].copy_from_slice(&1u16.to_le_bytes());           // AudioFormat = PCM
    h[22..24].copy_from_slice(&(CD_CHANNELS as u16).to_le_bytes());
    h[24..28].copy_from_slice(&sample_rate.to_le_bytes());
    h[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    h[32..34].copy_from_slice(&block_align.to_le_bytes());
    h[34..36].copy_from_slice(&(bits as u16).to_le_bytes());

    // data sub-chunk
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data_len.to_le_bytes());

    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_size() {
        let h = build_wav_header(0);
        assert_eq!(h.len(), 44);
    }

    #[test]
    fn wav_header_riff_magic() {
        let h = build_wav_header(1000);
        assert_eq!(&h[0..4], b"RIFF");
        assert_eq!(&h[8..12], b"WAVE");
        assert_eq!(&h[12..16], b"fmt ");
        assert_eq!(&h[36..40], b"data");
    }

    #[test]
    fn wav_header_chunk_size() {
        let data_len = 500_000u32;
        let h = build_wav_header(data_len);
        let chunk_size = u32::from_le_bytes(h[4..8].try_into().unwrap());
        assert_eq!(chunk_size, 36 + data_len);
    }

    #[test]
    fn wav_header_sample_rate() {
        let h = build_wav_header(0);
        let sr = u32::from_le_bytes(h[24..28].try_into().unwrap());
        assert_eq!(sr, 44_100);
    }
}
