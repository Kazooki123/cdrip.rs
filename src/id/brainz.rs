//! # MusicBrainz lookup via Disc ID.
//!
//! ## Disc ID
//!
//! MusicBrainz identifies CDs by a disc ID - a SHA-1 hash of the TOC's track
//! offsets and lead-out LBA encoded in a specific base64 variant. The `cdtoc`
//! crate handles this calculation from our `DiscToc` data.
//!
//! ## API flow
//!
//! 1. Compute MB disc ID from TOC using `cdtoc`
//! 2. GET `https://musicbrainz.org/ws/2/discid/<disc_id>?inc=recordings+artists&fmt=json`
//! 3. If not found, fall back to the fuzzy TOC search with the raw TOC string
//! 4. Parse the first release from the response
//!
//! ## Rate limiting
//!
//! MusicBrainz requires max 1 req/sec and a descriptive User-Agent header.
//! We'll just use `reqwest` blocking and add a 1.1s sleep before each request! :3

use super::{DiscMetadata, LookupConfig, LookupError, LookupResult};
use crate::toc::DiscToc;
use serde::Deserialize;
use tracing::{debug, warn};

pub fn lookup(toc: &DiscToc, config: &LookupConfig) -> LookupResult {
    let disc_id = compute_disc_id(toc)?;
    debug!("MusicBrainz disc ID: {}", disc_id);

    // Be polite pwease.. - MB requires ≤ 1 req/sec
    std::thread::sleep(std::time::Duration::from_millis(1100));

    let client = reqwest::blocking::Client::builder()
        .user_agent(&config.user_agent)
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| LookupError::Http(e.to_string()))?;

    let url = format!(
        "https://musicbrainz.org/ws/2/discid/{}?inc=recordings+artists&fmt=json",
        disc_id
    );

    debug!("MB request: {}", url);

    let resp = client
        .get(&url)
        .send()
        .map_err(|e| LookupError::Http(e.to_string()))?;

    if resp.status().is_success() {
        let body: MbDiscResponse = resp
            .json()
            .map_err(|e| LookupError::Parse(e.to_string()))?;
        return parse_disc_response(body, &disc_id, toc.track_count());
    }

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        warn!("Disc ID not found in MB, trying fuzzy TOC search...");
        return fuzzy_toc_lookup(&client, toc, &disc_id, config);
    }

    Err(LookupError::Http(format!("HTTP {}", resp.status())))
}

fn fuzzy_toc_lookup(
    client: &reqwest::blocking::Client,
    toc: &DiscToc,
    disc_id: &str,
    _config: &LookupConfig,
) -> LookupResult {
    // MB TOC string format: "first_track last_track lead_out off1 off2 ..."
    let offsets: Vec<String> = toc.tracks.iter().map(|t| t.start_lba.to_string()).collect();
    let toc_str = format!(
        "1+{}+{}+{}",
        toc.track_count(),
        toc.total_sectors,
        offsets.join("+")
    );

    let url = format!(
        "https://musicbrainz.org/ws/2/discid/-?toc={}&cdstubs=no&fmt=json",
        toc_str
    );

    debug!("MB fuzzy TOC URL: {}", url);
    std::thread::sleep(std::time::Duration::from_millis(1100));

    let resp = client
        .get(&url)
        .send()
        .map_err(|e| LookupError::Http(e.to_string()))?;

    if !resp.status().is_success() {
        return Err(LookupError::NotFound);
    }

    let body: MbDiscResponse = resp
        .json()
        .map_err(|e| LookupError::Parse(e.to_string()))?;

    parse_disc_response(body, disc_id, toc.track_count())
}

/// ## Disc ID calculation 
/// 
/// Compute the MusicBrainz disc ID from our `DiscToc`.
///
/// Uses the `cdtoc` crate which implements the MB disc ID algorithm:
/// SHA-1 of a binary structure encoding the first track, last track,
/// lead-out offset, and all track offsets, then base64url encoded with
/// MB's own alphabet substitutions (+→., /→_, =→-).
fn compute_disc_id(toc: &DiscToc) -> Result<String, LookupError> {
    // Build the cdtoc::Toc from our track offsets
    let offsets: Vec<u32> = toc.tracks.iter().map(|t| t.start_lba).collect();

    let cdtoc = cdtoc::Toc::from_parts(
        offsets,
        None,
        toc.total_sectors,
    )
    .map_err(|e| LookupError::NoDiscId)?;

    Ok(cdtoc.musicbrainz_id().to_string())
}

#[derive(Debug, Deserialize)]
struct MbDiscResponse {
    releases: Option<Vec<MbRelease>>,
    #[serde(rename = "release-list")]
    release_list: Option<Vec<MbRelease>>,
}

#[derive(Debug, Deserialize)]
struct MbRelease {
    id: String,
    title: String,
    #[serde(rename = "artist-credit")]
    artist_credit: Option<Vec<MbArtistCredit>>,
    date: Option<String>,
    media: Option<Vec<MbMedium>>,
}

#[derive(Debug, Deserialize)]
struct MbArtistCredit {
    artist: Option<MbArtist>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MbArtist {
    name: String,
}

#[derive(Debug, Deserialize)]
struct MbMedium {
    tracks: Option<Vec<MbTrack>>,
}

#[derive(Debug, Deserialize)]
struct MbTrack {
    title: String,
    #[serde(rename = "artist-credit")]
    artist_credit: Option<Vec<MbArtistCredit>>,
}

fn parse_disc_response(
    body: MbDiscResponse,
    disc_id: &str,
    track_count: u8,
) -> LookupResult {
    // Unify the two list fields
    let releases = body
        .releases
        .or(body.release_list)
        .filter(|r| !r.is_empty())
        .ok_or(LookupError::NotFound)?;

    // And THEN take the first release (MB sorts by relevance)
    let release = &releases[0];

    let album_artist = release.artist_credit.as_ref().and_then(|credits| {
        credits.first().and_then(|c| {
            c.name
                .clone()
                .or_else(|| c.artist.as_ref().map(|a| a.name.clone()))
        })
    });

    let year = release
        .date
        .as_deref()
        .and_then(|d| d.split('-').next())
        .and_then(|y| y.parse::<u16>().ok());

    let (track_titles, track_artists) = release
        .media
        .as_ref()
        .and_then(|media| media.first())
        .and_then(|m| m.tracks.as_ref())
        .map(|tracks| {
            let titles: Vec<Option<String>> = tracks
                .iter()
                .map(|t| Some(t.title.clone()))
                .collect();
            let artists: Vec<Option<String>> = tracks
                .iter()
                .map(|t| {
                    t.artist_credit.as_ref().and_then(|credits| {
                        credits.first().and_then(|c| {
                            c.name.clone().or_else(|| c.artist.as_ref().map(|a| a.name.clone()))
                        })
                    })
                })
                .collect();
            (titles, artists)
        })
        .unwrap_or_else(|| (vec![None; track_count as usize], vec![None; track_count as usize]));

    // Cover Art Archive URL (always worth trying - may 404 if the art isn't uploaded)
    // Feature exists thanks to @cbladeofficial on Discord suggestions! <3
    let cover_art_url = Some(format!(
        "https://coverartarchive.org/release/{}/front",
        release.id
    ));

    Ok(DiscMetadata {
        album_title: Some(release.title.clone()),
        album_artist,
        year,
        track_titles,
        track_artists,
        mb_release_id: Some(release.id.clone()),
        mb_disc_id: Some(disc_id.to_string()),
        cover_art_url,
        sources: vec!["MusicBrainz".to_string()],
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toc::{DiscToc, TrackInfo};

    fn sample_toc() -> DiscToc {
        DiscToc {
            tracks: vec![
                TrackInfo { number: 1, start_lba: 150,   sector_count: 14792, duration_msf: (3, 17, 17) },
                TrackInfo { number: 2, start_lba: 14942, sector_count: 16363, duration_msf: (3, 38, 13) },
                TrackInfo { number: 3, start_lba: 31305, sector_count: 11450, duration_msf: (2, 32, 50) },
            ],
            total_sectors: 42755,
        }
    }

    #[test]
    fn disc_id_computation_does_not_panic() {
        // Just check it runs without crashing — actual ID depends on cdtoc internals
        let toc = sample_toc();
        let result = compute_disc_id(&toc);
        assert!(result.is_ok() || matches!(result, Err(LookupError::NoDiscId)));
    }
}
