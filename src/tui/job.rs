//! Background ripping job for the TUI.
//!
//! The CLI's `Ripper` (in `ripper.rs`) drives `indicatif` progress bars
//! directly, which assumes exclusive control of the terminal. The TUI instead
//! needs progress events delivered asynchronously so that the render loop can keep
//! redrawing the whole screen (track table, gauges, log) every tick.
//!
//! This module re-implements the read + encode loop using the same lower-level
//! primitives (`cd_da_reader` streaming, `Encoder` trait) **BUT** reports progress
//! via an `mpsc::Sender<RipEvent>` instead of touching the terminal directly.

use crate::{
    encoder::{make_encoder, OutputFormat},
    toc::{DiscToc, TrackInfo},
};
use cd_da_reader::{CdReader, RetryConfig, TrackStreamConfig};
use std::{
    sync::mpsc::Sender,
    thread,
};

#[derive(Debug, Clone)]
pub enum RipEvent {
    TrackStarted { track: u8, total_sectors: u32 },
    SectorProgress { track: u8, sectors_done: u32 },
    TrackError { track: u8, message: String },
    TrackFinished { track: u8, output_file: String },
    TrackFailed { track: u8, reason: String },
    AllDone { ripped: u8, failed: u8 },
    Log(String),
}

#[derive(Debug, Clone)]
pub struct TuiRipConfig {
    pub device_path: String,
    pub output_dir: std::path::PathBuf,
    pub format: OutputFormat,
    pub tracks: Vec<u8>,
}

/// Spawn the rip job on a background OS thread.
///
/// The thread owns its own `CdReader` (opened fresh inside the thread) so the
/// TUI's main thread never blocks on drive I/O. All progress is reported via
/// `tx`; the TUI's event loop drains it every tick.
pub fn spawn_rip_job(
    config: TuiRipConfig,
    toc: DiscToc,
    tx: Sender<RipEvent>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        run_rip_job(config, toc, tx);
    })
}

fn run_rip_job(config: TuiRipConfig, toc: DiscToc, tx: Sender<RipEvent>) {
    let _ = tx.send(RipEvent::Log(format!(
        "Opening drive {}...",
        config.device_path
    )));

    let reader = match CdReader::open(&config.device_path) {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(RipEvent::Log(format!("Failed to open drive: {}", e)));
            let _ = tx.send(RipEvent::AllDone { ripped: 0, failed: config.tracks.len() as u8 });
            return;
        }
    };

    let raw_toc = match reader.read_toc() {
        Ok(t) => t,
        Err(e) => {
            let _ = tx.send(RipEvent::Log(format!("Failed to read TOC: {}", e)));
            let _ = tx.send(RipEvent::AllDone { ripped: 0, failed: config.tracks.len() as u8 });
            return;
        }
    };

    let _ = std::fs::create_dir_all(&config.output_dir);

    let encoder = make_encoder(config.format);
    let mut ripped = 0u8;
    let mut failed = 0u8;

    for &track_num in &config.tracks {
        let Ok(track) = toc.get_track(track_num) else {
            let _ = tx.send(RipEvent::TrackFailed {
                track: track_num,
                reason: "Track not found in TOC".to_string(),
            });
            failed += 1;
            continue;
        };

        let _ = tx.send(RipEvent::TrackStarted {
            track: track_num,
            total_sectors: track.sector_count,
        });

        match rip_one_track(&reader, &raw_toc, track, &*encoder, &config, &tx) {
            Ok(filename) => {
                let _ = tx.send(RipEvent::TrackFinished {
                    track: track_num,
                    output_file: filename,
                });
                ripped += 1;
            }
            Err(reason) => {
                let _ = tx.send(RipEvent::TrackFailed { track: track_num, reason });
                failed += 1;
            }
        }
    }

    let _ = tx.send(RipEvent::AllDone { ripped, failed });
}

fn rip_one_track(
    reader: &CdReader,
    raw_toc: &cd_da_reader::Toc,
    track: &TrackInfo,
    encoder: &dyn crate::encoder::Encoder,
    config: &TuiRipConfig,
    tx: &Sender<RipEvent>,
) -> Result<String, String> {
    let cfg = TrackStreamConfig {
        sectors_per_chunk: 27,
        retry: RetryConfig::default(),
    };

    let mut stream = reader
        .open_track_stream(raw_toc, track.number, cfg)
        .map_err(|e| format!("stream open failed: {}", e))?;

    let mut pcm: Vec<u8> = Vec::with_capacity(track.sector_count as usize * 2352);
    let mut sectors_done: u32 = 0;

    loop {
        match stream.next_chunk() {
            Ok(Some(chunk)) => {
                let chunk_sectors = (chunk.len() / 2352) as u32;
                pcm.extend_from_slice(&chunk);
                sectors_done += chunk_sectors;

                let _ = tx.send(RipEvent::SectorProgress {
                    track: track.number,
                    sectors_done,
                });
            }
            Ok(None) => break,
            Err(e) => {
                let _ = tx.send(RipEvent::TrackError {
                    track: track.number,
                    message: format!("chunk read error at ~sector {}: {}", sectors_done, e),
                });
                return Err(format!("read error: {}", e));
            }
        }
    }

    let filename = format!("track{:02}.{}", track.number, config.format.extension());
    let output_path = config.output_dir.join(&filename);

    encoder
        .encode(track.number, &pcm, &output_path)
        .map_err(|e| format!("encode failed: {}", e))?;

    Ok(filename)
}
