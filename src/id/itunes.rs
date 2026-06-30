//! iTunes Search API - Supplemental metadata enrichment.
//!
//! The iTunes Search API is a free, keyless keyword search. It can't do a disc
//! ID lookup, so it's only useful *after* MusicBrainz or GnuDB has already
//! given us an album title and artist name to search with.
//!
//! ## What we use it for
//!
//! - High-quality cover art URL (iTunes serves 600×600 or larger artwork)
//! - Release year cross-check
//! - Genre (iTunes genre taxonomy is well-maintained)
//!
//! ## What we don't use it for
//!
//! - Primary album/track title lookup (keyword search returns false positives)
//! - Track listing (iTunes results match digital releases, not physical CDs)
//!
//! ## Rate limit
//!
//! Apple limits to ~20 calls/min. We make at most 1 call per rip session.
//!
//! Reference: https://performance-partners.apple.com/search-api
//! Thanks to @cbladeofficial for the suggestion.

use super::{DiscMetadata, LookupConfig, LookupError, LookupResult};
use serde::Deserialize;
use tracing::debug;

const ITUNES_SEARCH_URL: &str = "https://itunes.apple.com/search";

/// Search iTunes for an album by artist + album name.
///
/// Returns a `DiscMetadata` with only the supplemental fields populated:
/// `cover_art_url`, optionally `year` and `genre`.
/// Track titles are intentionally not set — iTunes digital track listing
/// may differ from the physical CD (bonus tracks, remastering, etc.).
pub fn lookup(artist: &str, album: &str, config: &LookupConfig) -> LookupResult {
    let client = reqwest::blocking::Client::builder()
        .user_agent(&config.user_agent)
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| LookupError::Http(e.to_string()))?;

    let query = build_search_term(artist, album);
    debug!("iTunes search term: \"{}\"", query);

    let url = format!(
        "{}?term={}&media=music&entity=album&limit=5",
        ITUNES_SEARCH_URL,
        urlencoding::encode(&query)
    );

    debug!("iTunes URL: {}", url);

    let resp = client
        .get(&url)
        .send()
        .map_err(|e| LookupError::Http(e.to_string()))?;

    if !resp.status().is_success() {
        return Err(LookupError::Http(format!("HTTP {}", resp.status())));
    }

    let body: ItunesResponse = resp
        .json()
        .map_err(|e| LookupError::Parse(e.to_string()))?;

    if body.results.is_empty() {
        return Err(LookupError::NotFound);
    }

    // Picks the best result: prefer exact artist+album title match
    let best = best_match(&body.results, artist, album)
        .ok_or(LookupError::NotFound)?;

    debug!(
        "iTunes match: \"{}\" by \"{}\" ({})",
        best.collection_name, best.artist_name,
        best.release_date.as_deref().unwrap_or("?")
    );

    let cover_art_url = best
        .artwork_url_100
        .as_deref()
        .map(|url| url.replace("100x100bb", "600x600bb"));

    let year = best
        .release_date
        .as_deref()
        .and_then(|d| d.split('-').next())
        .and_then(|y| y.parse::<u16>().ok());

    Ok(DiscMetadata {
        cover_art_url,
        year,
        genre: best.primary_genre_name.clone(),
        sources: vec!["iTunes".to_string()],
        ..Default::default()
    })
}

/// Build an iTunes search term from artist + album.
/// iTunes keyword search works best with a combined "artist album" string.
/// We strip common noise words that confuse the search engine.
fn build_search_term(artist: &str, album: &str) -> String {
    // Sanitize: remove parenthetical suffixes like "(Remastered)" or "[Deluxe]"
    let clean_album = album
        .split('(').next().unwrap_or(album)
        .split('[').next().unwrap_or(album)
        .trim();

    if artist.is_empty() {
        clean_album.to_string()
    } else {
        format!("{} {}", artist, clean_album)
    }
}

/// Pick the best result from iTunes results.
/// Prefers exact case-insensitive matches on both artist and album.
/// Falls back to the first result if no exact match found.
fn best_match<'a>(
    results: &'a [ItunesAlbum],
    artist: &str,
    album: &str,
) -> Option<&'a ItunesAlbum> {
    let artist_lower = artist.to_lowercase();
    let album_lower = album.to_lowercase();

    // Try exact match
    if let Some(exact) = results.iter().find(|r| {
        r.artist_name.to_lowercase() == artist_lower
            && r.collection_name.to_lowercase() == album_lower
    }) {
        return Some(exact);
    }

    // Try partial match (album contains search term already..)
    if let Some(partial) = results.iter().find(|r| {
        r.collection_name
            .to_lowercase()
            .contains(&album_lower)
    }) {
        return Some(partial);
    }

    results.first()
}

// JSON TYPES >.<
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ItunesResponse {
    #[serde(default)]
    results: Vec<ItunesAlbum>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ItunesAlbum {
    artist_name: String,
    collection_name: String,
    artwork_url_100: Option<String>,
    release_date: Option<String>,
    primary_genre_name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_term_strips_parens() {
        let term = build_search_term("Nirvana", "Nevermind (Remastered)");
        assert_eq!(term, "Nirvana Nevermind");
    }

    #[test]
    fn search_term_strips_brackets() {
        let term = build_search_term("Nirvana", "Nevermind [Deluxe Edition]");
        assert_eq!(term, "Nirvana Nevermind");
    }

    #[test]
    fn search_term_no_artist() {
        let term = build_search_term("", "In Utero");
        assert_eq!(term, "In Utero");
    }

    #[test]
    fn best_match_exact_wins() {
        let results = vec![
            ItunesAlbum {
                artist_name: "Nirvana".to_string(),
                collection_name: "Nevermind (Deluxe)".to_string(),
                artwork_url_100: None,
                release_date: None,
                primary_genre_name: None,
            },
            ItunesAlbum {
                artist_name: "Nirvana".to_string(),
                collection_name: "Nevermind".to_string(),
                artwork_url_100: Some("http://example.com/100x100bb.jpg".to_string()),
                release_date: Some("1991-09-24T00:00:00Z".to_string()),
                primary_genre_name: Some("Alternative".to_string()),
            },
        ];

        let best = best_match(&results, "Nirvana", "Nevermind").unwrap();
        assert_eq!(best.collection_name, "Nevermind");
    }

    #[test]
    fn artwork_url_upscaled() {
        let url = "https://is1-ssl.mzstatic.com/image/thumb/abc/100x100bb.jpg";
        let upscaled = url.replace("100x100bb", "600x600bb");
        assert!(upscaled.contains("600x600bb"));
    }
}
