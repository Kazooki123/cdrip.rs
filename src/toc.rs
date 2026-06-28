use crate::error::{CdripError, Result};
use cd_da_reader::{CdReader, Toc};
use console::style;
use tracing::debug;

#[derive(Debug, Clone)]
pub struct TrackInfo {
    pub number: u8,
    /// Start sector (Logical Block Address)
    pub start_lba: u32,
    /// Total number of sectors in this track
    pub sector_count: u32,
    /// Duration as (minutes, seconds, frames)
    pub duration_msf: (u8, u8, u8),
}

impl TrackInfo {
    pub fn duration_display(&self) -> String {
        format!("{:02}:{:02}", self.duration_msf.0, self.duration_msf.1)
    }
    pub fn byte_count(&self) -> u64 {
        self.sector_count as u64 * 2352
    }
}

// Disc TOC
#[derive(Debug, Clone)]
pub struct DiscToc {
    pub tracks: Vec<TrackInfo>,
    /// Total disc duration in sectors
    pub total_sectors: u32,
}

impl DiscToc {
    pub fn track_count(&self) -> u8 {
        self.tracks.len() as u8
    }

    pub fn total_duration_display(&self) -> String {
        let total_seconds = self.total_sectors / 75;
        format!("{:02}:{:02}", total_seconds / 60, total_seconds % 60)
    }

    pub fn total_bytes(&self) -> u64 {
        self.tracks.iter().map(|t| t.byte_count()).sum()
    }

    pub fn get_track(&self, number: u8) -> Result<&TrackInfo> {
        self.tracks
            .iter()
            .find(|t| t.number == number)
            .ok_or_else(|| CdripError::TrackNotFound(number, self.track_count()))
    }
}

// Reading
/// Read and parse the Table of Contents from an open drive.
/// Returns both our enriched `DiscToc` and the raw `cd_da_reader::Toc`
/// (the raw one is needed by the streaming read API).
pub fn read_toc(reader: &CdReader) -> Result<(DiscToc, cd_da_reader::Toc)> {
    let raw_toc: Toc = reader
        .read_toc()
        .map_err(|e| CdripError::TocReadFailed(e.to_string()))?;

    let raw_tracks = &raw_toc.tracks;
    let track_count = raw_tracks.len();

    let mut tracks = Vec::with_capacity(track_count);

    for (i, track) in raw_tracks.iter().enumerate() {
        // Sector count = next track start - this track start
        // For the last track, use the lead-out LBA.
        let next_lba = if i + 1 < track_count {
            raw_tracks[i + 1].start_lba
        } else {
            raw_toc.leadout_lba
        };

        let start_lba = track.start_lba;
        let sector_count = next_lba.saturating_sub(start_lba);

        // Convert sector count to MSF (75 sectors/sec)
        let total_seconds = sector_count / 75;
        let frames = (sector_count % 75) as u8;
        let minutes = (total_seconds / 60) as u8;
        let seconds = (total_seconds % 60) as u8;

        debug!(
            "Track {:02}: start_lba={} sectors={} duration={:02}:{:02}",
            i + 1,
            start_lba,
            sector_count,
            minutes,
            seconds
        );

        tracks.push(TrackInfo {
            number: (i + 1) as u8,
            start_lba,
            sector_count,
            duration_msf: (minutes, seconds, frames),
        });
    }

    let total_sectors = raw_toc.leadout_lba;

    Ok((DiscToc { tracks, total_sectors }, raw_toc))
}

// Display
pub fn print_toc(toc: &DiscToc) {
    println!();
    println!(
        "  {}",
        style("=============== TABLE OF CONTENTS ===============").cyan()
    );
    println!(
        "  {:>5}  {:>10}  {:>10}  {:>10}  {:>12}",
        style("Track").bold(),
        style("Start LBA").bold(),
        style("Sectors").bold(),
        style("Duration").bold(),
        style("Size (MiB)").bold(),
    );
    println!("  {}", style("─".repeat(56)).dim());

    for track in &toc.tracks {
        let mib = track.byte_count() as f64 / (1024.0 * 1024.0);
        println!(
            "  {:>5}  {:>10}  {:>10}  {:>10}  {:>10.1} MiB",
            style(format!("{:02}", track.number)).yellow(),
            track.start_lba,
            track.sector_count,
            style(track.duration_display()).green(),
            mib,
        );
    }

    println!("  {}", style("─".repeat(56)).dim());

    let total_mib = toc.total_bytes() as f64 / (1024.0 * 1024.0);
    println!(
        "  {:>5}  {:>10}  {:>10}  {:>10}  {:>10.1} MiB",
        style(format!("{} tracks", toc.track_count())).bold(),
        "",
        toc.total_sectors,
        style(toc.total_duration_display()).green().bold(),
        total_mib,
    );
    println!();
}
