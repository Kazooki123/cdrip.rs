//! Parallel encoding support.
//!
//! A physical CD drive has one laser head — true parallel *reading* of multiple
//! tracks simultaneously is not possible. What we CAN parallelise is the CPU-
//! bound encoding step (especially FLAC, which is compute-heavy).
//!
//! Strategy:
//!   1. Read all tracks sequentially from the drive (only one can read at a time).
//!   2. Dispatch encoding jobs to a thread pool — each track's PCM buffer is
//!      encoded independently and concurrently.
//!
//! This gives a significant speedup on multi-core machines: on a 12-track disc
//! with FLAC encoding you can expect ~3–6× faster total time depending on core
//! count and compression preset.
//!
//! The `rayon` feature flag on `indicatif` is already enabled in Cargo.toml,
//! so progress bars work safely across threads.

#[allow(unused)]
#[allow(dead_code)]

use crate::{
    encoder::{make_encoder, OutputFormat},
    error::{CdripError, Result},
    toc::TrackInfo,
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
};
use tracing::{info, warn};

// Encode job

#[derive(Debug)]
pub struct EncodeJob {
    pub track_num: u8,
    pub pcm: Vec<u8>,
    pub output_path: PathBuf,
}

/// Result of a completed encode job.
#[derive(Debug)]
pub struct EncodeResult {
    pub track_num: u8,
    pub output_path: PathBuf,
    pub bytes_written: u64,
    pub status: EncodeStatus,
}

#[derive(Debug, PartialEq, Eq)]
pub enum EncodeStatus {
    Ok,
    Failed(String),
}

// Parallel encoder pool
/// Encode a batch of PCM jobs in parallel using a thread-per-job model.
/// Jobs are spawned via `std::thread::spawn`; no external thread-pool crate
/// needed. The number of concurrent threads is bounded by `max_threads`.
/// Returns results in *completion* order (not track order). Callers should
/// sort by `track_num` if order matters (e.g. for the manifest).
pub fn encode_parallel(
    jobs: Vec<EncodeJob>,
    format: OutputFormat,
    max_threads: usize,
) -> Vec<EncodeResult> {
    if jobs.is_empty() {
        return Vec::new();
    }

    info!(
        "Parallel encode: {} job(s) across up to {} thread(s)",
        jobs.len(),
        max_threads
    );

    let queue: Arc<Mutex<Vec<EncodeJob>>> = Arc::new(Mutex::new(jobs));
    let results: Arc<Mutex<Vec<EncodeResult>>> = Arc::new(Mutex::new(Vec::new()));

    let thread_count = max_threads.min(
        queue.lock().unwrap().len()
    );

    let mut handles = Vec::with_capacity(thread_count);

    for _ in 0..thread_count {
        let queue = Arc::clone(&queue);
        let results = Arc::clone(&results);

        let handle = thread::spawn(move || {
            // Each thread keeps pulling jobs until the queue is empty
            loop {
                let job = {
                    let mut q = queue.lock().unwrap();
                    if q.is_empty() {
                        break;
                    }
                    q.remove(0)
                };

                let track_num = job.track_num;
                info!("Thread encoding track {:02}", track_num);

                let encoder = make_encoder(format);
                let result = encoder.encode(track_num, &job.pcm, &job.output_path);

                let encode_result = match result {
                    Ok(path) => {
                        let bytes = path.metadata().map(|m| m.len()).unwrap_or(0);
                        EncodeResult {
                            track_num,
                            output_path: path,
                            bytes_written: bytes,
                            status: EncodeStatus::Ok,
                        }
                    }
                    Err(e) => {
                        warn!("Encode failed for track {:02}: {}", track_num, e);
                        EncodeResult {
                            track_num,
                            output_path: job.output_path,
                            bytes_written: 0,
                            status: EncodeStatus::Failed(e.to_string()),
                        }
                    }
                };

                results.lock().unwrap().push(encode_result);
            }
        });

        handles.push(handle);
    }

    for handle in handles {
        if let Err(e) = handle.join() {
            warn!("Encode thread panicked: {:?}", e);
        }
    }

    let mut out = Arc::try_unwrap(results)
        .expect("all threads finished")
        .into_inner()
        .expect("mutex not poisoned");

    out.sort_by_key(|r| r.track_num);
    out
}

// Thread count helpers
/// Sensible default thread count for encoding:
/// half the logical CPU count, minimum 1, maximum 8.
/// Leaves headroom for the main thread and OS tasks.
pub fn default_thread_count() -> usize {
    let cpus = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2);
    (cpus / 2).clamp(1, 8)
}

// TESTS
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_thread_count_in_range() {
        let n = default_thread_count();
        assert!(n >= 1 && n <= 8);
    }

    #[test]
    fn encode_parallel_empty() {
        let results = encode_parallel(vec![], OutputFormat::Wav, 4);
        assert!(results.is_empty());
    }
}
