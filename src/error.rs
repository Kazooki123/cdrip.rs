#![allow(unused)]

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CdripError {
    #[error("No CD drive found. Is a drive connected and a disc inserted?")]
    NoDriveFound,
    #[error("Drive '{0}' not found or inaccessible")]
    DriveNotFound(String),
    #[error("No disc inserted in drive '{0}'")]
    NoDiscInserted(String),
    #[error("Failed to open drive: {0}")]
    DriveOpenFailed(String),
    #[error("Failed to read Table of Contents: {0}")]
    TocReadFailed(String),
    #[error("Track {0} not found on disc (disc has {1} tracks)")]
    TrackNotFound(u8, u8),
    #[error("Sector read error at LBA {lba} (track {track}, attempt {attempt}/{max_attempts})")]
    SectorReadError {
        lba: u32,
        track: u8,
        attempt: u8,
        max_attempts: u8,
    },
    #[error("Track {0} rip failed after {1} retries — too many bad sectors")]
    TrackRipFailed(u8, u8),
    #[error("WAV encoding failed for track {0}: {1}")]
    WavEncodeFailed(u8, String),
    #[error("FLAC encoding failed for track {0}: {1}")]
    FlacEncodeFailed(u8, String),
    #[error("Output directory '{0}' could not be created: {1}")]
    OutputDirFailed(String, #[source] std::io::Error),
    #[error("Failed to write '{0}': {1}")]
    FileWriteFailed(String, #[source] std::io::Error),
    #[error("Failed to write rip manifest: {0}")]
    ManifestFailed(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, CdripError>;