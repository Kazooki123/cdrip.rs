use crate::{
    encoder::{make_encoder, OutputFormat},
    error::{CdripError, Result},
    progress::RipProgress,
    toc::{DiscToc, TrackInfo},
};
use cd_da_reader::{CdReader, RetryConfig, TrackStreamConfig};
use chrono::Utc;
use console::style;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::{info, warn};

#[derive(Debug, Clone)]
pub struct RipConfig {
    pub output_dir: PathBuf,
    pub format: OutputFormat,
    pub max_retries: u8,
    /// If true, keep going after a track fails..
    pub skip_errors: bool,
    /// If Some, rip only this track number. If None, rip all.
    pub track_filter: Option<u8>,
}

impl Default for RipConfig {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("."),
            format: OutputFormat::Flac,
            max_retries: 5,
            skip_errors: false,
            track_filter: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TrackResult {
    pub track: u8,
    pub output_file: String,
    pub sectors: u32,
    pub bytes: u64,
    pub retries: u32,
    pub errors: u32,
    pub status: String, // "ok" | "failed"
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RipManifest {
    pub ripped_at: String,
    pub format: String,
    pub total_tracks: u8,
    pub tracks: Vec<TrackResult>,
    pub total_retries: u32,
    pub total_errors: u32,
}

// Ripper
pub struct Ripper<'a> {
    reader: &'a CdReader,
    toc: &'a DiscToc,
    raw_toc: &'a cd_da_reader::Toc,
    config: &'a RipConfig,
}

impl<'a> Ripper<'a> {
    pub fn new(
        reader: &'a CdReader,
        toc: &'a DiscToc,
        raw_toc: &'a cd_da_reader::Toc,
        config: &'a RipConfig,
    ) -> Self {
        Self { reader, toc, raw_toc, config }
    }

    pub fn run(&self) -> Result<RipManifest> {
        std::fs::create_dir_all(&self.config.output_dir).map_err(|e| {
            CdripError::OutputDirFailed(self.config.output_dir.display().to_string(), e)
        })?;

        let tracks_to_rip: Vec<&TrackInfo> = match self.config.track_filter {
            Some(n) => vec![self.toc.get_track(n)?],
            None => self.toc.tracks.iter().collect(),
        };

        let total = tracks_to_rip.len() as u8;
        let mut progress = RipProgress::new(total);
        let encoder = make_encoder(self.config.format);

        let mut results: Vec<TrackResult> = Vec::new();
        let mut ripped_ok: u8 = 0;
        let mut failed: u8 = 0;

        println!(
            "\n  {} Ripping {} track(s) as {} -> {}\n",
            style("▶").cyan().bold(),
            total,
            style(self.config.format.to_string()).green(),
            style(self.config.output_dir.display().to_string()).dim()
        );

        for track in &tracks_to_rip {
            progress.begin_track(track);

            match self.rip_track(track, &encoder, &mut progress) {
                Ok(result) => {
                    let out = result.output_file.clone();
                    progress.finish_track(track.number, &out);
                    ripped_ok += 1;
                    results.push(result);
                }
                Err(e) => {
                    warn!("Track {:02} failed: {}", track.number, e);
                    progress.fail_track(track.number);
                    failed += 1;
                    results.push(TrackResult {
                        track: track.number,
                        output_file: String::new(),
                        sectors: track.sector_count,
                        bytes: 0,
                        retries: 0,
                        errors: 1,
                        status: "failed".to_string(),
                    });
                    if !self.config.skip_errors {
                        progress.finish(ripped_ok, failed);
                        return Err(e);
                    }
                }
            }
        }

        let manifest = RipManifest {
            ripped_at: Utc::now().to_rfc3339(),
            format: self.config.format.to_string(),
            total_tracks: total,
            tracks: results,
            total_retries: progress.retry_count,
            total_errors: progress.error_count,
        };

        progress.finish(ripped_ok, failed);
        self.write_manifest(&manifest)?;

        Ok(manifest)
    }

    // Single-track rip
    fn rip_track(
        &self,
        track: &TrackInfo,
        encoder: &Box<dyn crate::encoder::Encoder>,
        progress: &mut RipProgress,
    ) -> Result<TrackResult> {
        info!(
            "Ripping track {:02}: LBA {}-{} ({} sectors)",
            track.number,
            track.start_lba,
            track.start_lba + track.sector_count,
            track.sector_count
        );

        // Configure the stream: 27 sectors/chunk ≈ 64 KB per read.
        // RetryConfig::default() gives sensible built-in retry behaviour.
        let cfg = TrackStreamConfig {
            sectors_per_chunk: 27,
            retry: RetryConfig::default(),
        };

        let mut stream = self
            .reader
            .open_track_stream(self.raw_toc, track.number, cfg)
            .map_err(|_e| CdripError::TrackRipFailed(track.number, self.config.max_retries))?;

        let mut pcm: Vec<u8> = Vec::with_capacity(track.sector_count as usize * 2352);
        let mut sectors_read: u32 = 0;
        let mut errors: u32 = 0;

        while let Some(chunk) = stream.next_chunk().map_err(|e| {
            errors += 1;
            progress.record_error(track.start_lba + sectors_read);
            warn!("Chunk read error on track {} at ~sector {}: {}", track.number, sectors_read, e);
            CdripError::SectorReadError {
                lba: track.start_lba + sectors_read,
                track: track.number,
                attempt: 1,
                max_attempts: self.config.max_retries,
            }
        })? {
            let chunk_sectors = (chunk.len() / 2352) as u32;
            pcm.extend_from_slice(&chunk);
            sectors_read += chunk_sectors;
            progress.advance_sectors(chunk_sectors as u64);
        }

        // Build output path: track01.flac, track02.wav, etc.
        let filename = format!(
            "track{:02}.{}",
            track.number,
            self.config.format.extension()
        );
        let output_path = self.config.output_dir.join(&filename);

        encoder.encode(track.number, &pcm, &output_path)?;

        let bytes = output_path.metadata().map(|m| m.len()).unwrap_or(pcm.len() as u64);

        Ok(TrackResult {
            track: track.number,
            output_file: filename,
            sectors: sectors_read,
            bytes,
            retries: 0, // retries handled internally by RetryConfig
            errors,
            status: if errors == 0 { "ok".to_string() } else { "ok_with_errors".to_string() },
        })
    }

    // Manifest
    fn write_manifest(&self, manifest: &RipManifest) -> Result<()> {
        let path = self.config.output_dir.join("cd-manifest.json");
        let json = serde_json::to_string_pretty(manifest)
            .map_err(|e| CdripError::ManifestFailed(e.to_string()))?;
        std::fs::write(&path, json)
            .map_err(|e| CdripError::ManifestFailed(format!("{}: {}", path.display(), e)))?;
        info!("Wrote rip manifest -> {}", path.display());
        Ok(())
    }
}
