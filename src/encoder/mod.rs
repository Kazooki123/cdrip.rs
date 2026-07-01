#![allow(unused)]

pub mod flac;
pub mod wav;

#[cfg(feature = "mp3")]
pub mod mp3;

use crate::error::Result;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    Wav,
    Flac,
    #[cfg(feature = "mp3")]
    Mp3,
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputFormat::Wav  => write!(f, "WAV"),
            OutputFormat::Flac => write!(f, "FLAC"),
            #[cfg(feature = "mp3")]
            OutputFormat::Mp3  => write!(f, "MP3"),
        }
    }
}

impl OutputFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            OutputFormat::Wav  => "wav",
            OutputFormat::Flac => "flac",
            #[cfg(feature = "mp3")]
            OutputFormat::Mp3  => "mp3",
        }
    }

    pub fn is_lossy(&self) -> bool {
        match self {
            OutputFormat::Wav | OutputFormat::Flac => false,
            #[cfg(feature = "mp3")]
            OutputFormat::Mp3 => true,
        }
    }
}

pub const CD_SAMPLE_RATE: u32      = 44_100;
pub const CD_CHANNELS: u8          = 2;
pub const CD_BITS_PER_SAMPLE: u8   = 16;
pub const CD_BYTES_PER_SECTOR: u32 = 2_352;

pub trait Encoder {
    fn encode(&self, track_num: u8, pcm_data: &[u8], output_path: &PathBuf) -> Result<PathBuf>;
}

pub fn make_encoder(format: OutputFormat) -> Box<dyn Encoder> {
    match format {
        OutputFormat::Wav  => Box::new(wav::WavEncoder),
        OutputFormat::Flac => Box::new(flac::FlacEncoder::default()),
        #[cfg(feature = "mp3")]
        OutputFormat::Mp3  => Box::new(mp3::Mp3Encoder::default()),
    }
}
