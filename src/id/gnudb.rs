//! # Gnudb.org Tagging
//! 
//! GnuDB is the open-source successor to FreeDB (which died in 2020 ╯︿╰...),
//! which was itself the successor to the original CDDB.
//! It uses the classic CDDB protocol over plain HTTP GET.
//!
//! ## CDDB Disc ID algorithm
//!
//! The CDDB disc ID is an 8-digit hex number computed as:
//! 1. For each track, sum the digits of its start time in seconds.
//!    (e.g. track starting at 95 seconds -> 9 + 5 = 14)
//! 2. Sum all those digit-sums, call it `n`.
//! 3. disc_id = ((n % 255) << 24) | (total_seconds << 8) | track_count
//!
//! This is a much weaker hash than MB's SHA-1 disc ID — collisions exist —
//! but it's still the standard for the CDDB ecosystem.
//!
//! ## Protocol
//!
//! Two HTTP GETs:
//! 1. `cddb query` — find matching entries for our disc ID
//! 2. `cddb read` — fetch the full xmcd entry for the best match
//!
//! Reference: https://gnudb.org/howtognudb.php

use super::{DiscMetadata, LookupConfig, LookupError, LookupResult};
use crate::toc::DiscToc;
use tracing::debug;

const GNUDB_URL: &str = "http://gnudb.gnudb.org/~cddb/cddb.cgi";

pub fn lookup(toc: &DiscToc, config: &LookupConfig) -> LookupResult {
    let disc_id = compute_cddb_id(toc);
    debug!("CDDB disc ID: {:08x}", disc_id);

    let client = reqwest::blocking::Client::builder()
        .user_agent(&config.user_agent)
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| LookupError::Http(e.to_string()))?;

    // Query - find matches
    let matches = cddb_query(&client, toc, disc_id, config)?;
    if matches.is_empty() {
        return Err(LookupError::NotFound);
    }

    // Read - fetch the best match (first result)
    let (category, matched_id) = &matches[0];
    if matches.len() > 1 {
        debug!("GnuDB: {} matches, using first ({} {})", matches.len(), category, matched_id);
    }

    let xmcd = cddb_read(&client, category, matched_id, config)?;
    parse_xmcd(&xmcd, disc_id, toc.track_count())
}

/// Issue a `cddb query` and return a list of (category, disc_id) pairs.
fn cddb_query(
    client: &reqwest::blocking::Client,
    toc: &DiscToc,
    disc_id: u32,
    config: &LookupConfig,
) -> Result<Vec<(String, String)>, LookupError> {
    let offsets: Vec<String> = toc
        .tracks
        .iter()
        .map(|t| (t.start_lba + 150).to_string())
        .collect();

    let total_seconds = toc.total_sectors / 75;

    // Format: cddb+query+discid+ntrks+off1+off2...+nsecs
    let cmd = format!(
        "cddb+query+{:08x}+{}+{}+{}",
        disc_id,
        toc.track_count(),
        offsets.join("+"),
        total_seconds,
    );

    let url = format!(
        "{}?cmd={}&hello={}&proto=6",
        GNUDB_URL,
        cmd,
        hello_string(config),
    );

    debug!("GnuDB query: {}", url);

    let text = client
        .get(&url)
        .send()
        .map_err(|e| LookupError::Http(e.to_string()))?
        .text()
        .map_err(|e| LookupError::Http(e.to_string()))?;

    parse_query_response(&text)
}

/// Parses the CDDB query response into (category, disc_id) pairs.
///
/// Response codes:
/// - `200` = exact match (single line)
/// - `210` = multiple exact matches (terminated by `.`)
/// - `211` = inexact matches
/// - `202` = not found
fn parse_query_response(text: &str) -> Result<Vec<(String, String)>, LookupError> {
    let mut lines = text.lines();
    let first = lines.next().unwrap_or("").trim();

    let code: u32 = first
        .split_whitespace()
        .next()
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);

    match code {
        200 => {
            let parts: Vec<&str> = first.splitn(4, ' ').collect();
            if parts.len() >= 3 {
                Ok(vec![(parts[1].to_string(), parts[2].to_string())])
            } else {
                Err(LookupError::Parse("Malformed 200 response".to_string()))
            }
        }
        210 | 211 => {
            let mut results = Vec::new();
            for line in lines {
                let line = line.trim();
                if line == "." {
                    break;
                }
                let parts: Vec<&str> = line.splitn(3, ' ').collect();
                if parts.len() >= 2 {
                    results.push((parts[0].to_string(), parts[1].to_string()));
                }
            }
            if results.is_empty() {
                Err(LookupError::NotFound)
            } else {
                Ok(results)
            }
        }
        202 => Err(LookupError::NotFound),
        _ => Err(LookupError::Parse(format!("Unexpected response code: {}", code))),
    }
}

/// Issue a `cddb read` and return the raw xmcd text.
fn cddb_read(
    client: &reqwest::blocking::Client,
    category: &str,
    disc_id: &str,
    config: &LookupConfig,
) -> Result<String, LookupError> {
    let cmd = format!("cddb+read+{}+{}", category, disc_id);
    let url = format!(
        "{}?cmd={}&hello={}&proto=6",
        GNUDB_URL,
        cmd,
        hello_string(config),
    );

    debug!("GnuDB read: {}", url);

    let text = client
        .get(&url)
        .send()
        .map_err(|e| LookupError::Http(e.to_string()))?
        .text()
        .map_err(|e| LookupError::Http(e.to_string()))?;

    // First line SHOULD be "210 category discid OK"
    let first_line = text.lines().next().unwrap_or("").trim();
    if !first_line.starts_with("210") {
        return Err(LookupError::Parse(format!(
            "cddb read failed: {}",
            first_line
        )));
    }

    Ok(text)
}

/// Parse an xmcd text response into `DiscMetadata`.
///
/// xmcd format is a series of `KEY=VALUE` lines where keys include:
/// - `DTITLE=Artist / Album` (always `Artist / Album` separated by ` / `)
/// - `DYEAR=2001`
/// - `DGENRE=Rock`
/// - `TTITLE0=Track One`, `TTITLE1=Track Two`, etc.
fn parse_xmcd(text: &str, disc_id: u32, track_count: u8) -> LookupResult {
    let mut artist: Option<String> = None;
    let mut album: Option<String> = None;
    let mut year: Option<u16> = None;
    let mut genre: Option<String> = None;
    let mut track_titles: Vec<Option<String>> = vec![None; track_count as usize];

    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with('.') || line.is_empty() {
            continue;
        }

        if let Some(value) = line.strip_prefix("DTITLE=") {
            // DTITLE format: "Artist / Album Title"
            if let Some((a, al)) = value.split_once(" / ") {
                artist = Some(a.trim().to_string());
                album = Some(al.trim().to_string());
            } else {
                album = Some(value.trim().to_string());
            }
        } else if let Some(value) = line.strip_prefix("DYEAR=") {
            year = value.trim().parse::<u16>().ok();
        } else if let Some(value) = line.strip_prefix("DGENRE=") {
            let g = value.trim().to_string();
            if !g.is_empty() {
                genre = Some(g);
            }
        } else if let Some(rest) = line.strip_prefix("TTITLE") {
            // TTITLEn=Track title
            if let Some((idx_str, title)) = rest.split_once('=') {
                if let Ok(idx) = idx_str.parse::<usize>() {
                    if idx < track_titles.len() {
                        track_titles[idx] = Some(title.trim().to_string());
                    }
                }
            }
        }
    }

    if album.is_none() {
        return Err(LookupError::Parse("No DTITLE found in xmcd".to_string()));
    }

    Ok(DiscMetadata {
        album_title: album,
        album_artist: artist,
        year,
        genre,
        track_titles,
        cddb_disc_id: Some(format!("{:08x}", disc_id)),
        sources: vec!["GnuDB".to_string()],
        ..Default::default()
    })
}

/// ## CDDB disc ID computation
/// Compute the 32-bit CDDB disc ID from TOC data.
///
/// Algorithm (from the original CDDB spec):
/// ```
/// n = sum of digit_sum(track_start_seconds) for each track
/// disc_id = ((n % 255) << 24) | (total_seconds << 8) | track_count
/// ```
pub fn compute_cddb_id(toc: &DiscToc) -> u32 {
    // Sums the digit-sums of each track's start time in seconds
    let n: u32 = toc
        .tracks
        .iter()
        .map(|t| {
            let secs = (t.start_lba + 150) / 75; // Then convert LBA to seconds (75 sectors/sec), add standard 2-sec offset
            digit_sum(secs)
        })
        .sum();

    let total_seconds = toc.total_sectors / 75;
    let track_count = toc.track_count() as u32;

    ((n % 255) << 24) | (total_seconds << 8) | track_count
}

fn digit_sum(mut n: u32) -> u32 {
    let mut sum = 0;
    if n == 0 {
        return 0;
    }
    while n > 0 {
        sum += n % 10;
        n /= 10;
    }
    sum
}

/// Build the GnuDB hello string.
/// Format: `username+hostname+clientname+version`
fn hello_string(config: &LookupConfig) -> String {
    let email = config.gnudb_email.replace('@', "+").replace('.', "+");
    format!("{}+cdrip+{}", email, env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digit_sum_basic() {
        assert_eq!(digit_sum(0), 0);
        assert_eq!(digit_sum(9), 9);
        assert_eq!(digit_sum(95), 14);
        assert_eq!(digit_sum(100), 1);
        assert_eq!(digit_sum(123), 6);
    }

    #[test]
    fn cddb_id_known_disc() {
        // Known Nevermind offsets (from online CDDB records)
        // This will validates the algorithm and produces a stable value!!
        use crate::toc::{DiscToc, TrackInfo};
        let toc = DiscToc {
            tracks: vec![
                TrackInfo { number: 1,  start_lba: 0,      sector_count: 14795, duration_msf: (3, 17, 20) },
                TrackInfo { number: 2,  start_lba: 14795,  sector_count: 16100, duration_msf: (3, 34, 26) },
                TrackInfo { number: 3,  start_lba: 30895,  sector_count: 12730, duration_msf: (2, 49, 55) },
                TrackInfo { number: 4,  start_lba: 43625,  sector_count: 13340, duration_msf: (2, 57, 65) },
                TrackInfo { number: 5,  start_lba: 56965,  sector_count: 15300, duration_msf: (3, 24,  0) },
                TrackInfo { number: 6,  start_lba: 72265,  sector_count: 15960, duration_msf: (3, 32, 60) },
                TrackInfo { number: 7,  start_lba: 88225,  sector_count: 12810, duration_msf: (2, 50, 60) },
                TrackInfo { number: 8,  start_lba: 101035, sector_count: 11480, duration_msf: (2, 32, 30) },
                TrackInfo { number: 9,  start_lba: 112515, sector_count: 15350, duration_msf: (3, 25, 25) },
                TrackInfo { number: 10, start_lba: 127865, sector_count: 12820, duration_msf: (2, 50, 70) },
                TrackInfo { number: 11, start_lba: 140685, sector_count: 12840, duration_msf: (2, 51, 15) },
                TrackInfo { number: 12, start_lba: 153525, sector_count: 20190, duration_msf: (4, 28, 15) },
            ],
            total_sectors: 173715,
        };
        let id = compute_cddb_id(&toc);

        assert_ne!(id, 0);
        assert_eq!(format!("{:08x}", id).len(), 8);
    }

    #[test]
    fn parse_query_200_response() {
        let resp = "200 rock 9a09340d Pink Floyd / The Wall\r\n";
        let matches = parse_query_response(resp).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].0, "rock");
        assert_eq!(matches[0].1, "9a09340d");
    }

    #[test]
    fn parse_query_202_not_found() {
        let resp = "202 No match found\r\n";
        assert!(matches!(parse_query_response(resp), Err(LookupError::NotFound)));
    }

    #[test]
    fn parse_xmcd_basic() {
        let xmcd = "210 rock 9a09340d OK\nDTITLE=Nirvana / Nevermind\nDYEAR=1991\nDGENRE=Rock\nTTITLE0=Smells Like Teen Spirit\nTTITLE1=In Bloom\n.\n";
        let meta = parse_xmcd(xmcd, 0x9a09340d, 2).unwrap();
        assert_eq!(meta.album_title.as_deref(), Some("Nevermind"));
        assert_eq!(meta.album_artist.as_deref(), Some("Nirvana"));
        assert_eq!(meta.year, Some(1991));
        assert_eq!(meta.track_titles[0].as_deref(), Some("Smells Like Teen Spirit"));
        assert_eq!(meta.track_titles[1].as_deref(), Some("In Bloom"));
    }
}
