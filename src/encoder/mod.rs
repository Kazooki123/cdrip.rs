#![allow(unused)]

pub mod flac;
pub mod wav;
pub mod mp3;

use crate::error::Result;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    Wav,
    Flac,
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputFormat::Wav => write!(f, "WAV"),
            OutputFormat::Flac => write!(f, "FLAC"),
        }
    }
}

impl OutputFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            OutputFormat::Wav => "wav",
            OutputFormat::Flac => "flac",
        }
    }
}

pub const CD_SAMPLE_RATE: u32 = 44_100;
pub const CD_CHANNELS: u8 = 2;
pub const CD_BITS_PER_SAMPLE: u8 = 16;
pub const CD_BYTES_PER_SECTOR: u32 = 2_352;

/// Common interface for all audio encoders.
/// Receives raw interleaved 16-bit LE PCM data (straight off the CD sector
/// reads) and writes it to the appropriate container on disk.
pub trait Encoder {
    fn encode(&self, track_num: u8, pcm_data: &[u8], output_path: &PathBuf) -> Result<PathBuf>;
}

/// Build the correct encoder for the requested format.
pub fn make_encoder(format: OutputFormat) -> Box<dyn Encoder> {
    match format {
        OutputFormat::Wav => Box::new(wav::WavEncoder),
        OutputFormat::Flac => Box::new(flac::FlacEncoder::default()),
    }
}
