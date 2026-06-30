//! CD metadata ID lookup backends.
//!
//! All backends return a common [`DiscMetadata`] type so callers don't need to
//! know which backend succeeded. Each backend is gated behind a Cargo feature
//! so users can opt out of network deps entirely.
//!
//! ## Backend comparison
//!
//! | Backend     | Lookup method       | No-key (?) | Quality        |
//! |-------------|---------------------|------------|----------------|
//! | MusicBrainz | Disc ID (exact)     | ✓          | Best, open     |
//! | GnuDB       | CDDB ID (checksum)  | ✓          | Good, legacy   |
//! | iTunes      | Keyword search      | ✓          | Supplemental   |
//!
//! MusicBrainz is always tried first. GnuDB is the fallback. iTunes is only
//! used as an enrichment step (cover art URL, Apple Music link) once the album
//! name is known from MB or GnuDB.
//!
//! ## Usage
//!
//! ```rust
//! let meta = lookup_all(&toc, LookupConfig::default()).await;
//! ```

pub mod brainz;
pub mod gnudb;
pub mod itunes;

use crate::toc::DiscToc;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

/// Unified disc metadata returned by any lookup backend.
/// All fields are optional - a backend may only fill in a subset.
/// Callers merge results: MB fills the core fields, iTunes can add artwork.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiscMetadata {
    pub album_title: Option<String>,
    pub album_artist: Option<String>,
    pub year: Option<u16>,
    pub genre: Option<String>,
    pub track_titles: Vec<Option<String>>,
    pub track_artists: Vec<Option<String>>,
    pub mb_release_id: Option<String>,
    pub mb_disc_id: Option<String>,
    pub cddb_disc_id: Option<String>,
    pub cover_art_url: Option<String>,
    pub sources: Vec<String>,
}

impl DiscMetadata {
    pub fn is_useful(&self) -> bool {
        self.album_title.is_some()
            && self.track_titles.iter().any(|t| t.is_some())
    }

    /// Merge `other` into `self`, filling in any missing fields.
    /// Fields already set in `self` are not overwritten.
    pub fn merge_from(&mut self, other: DiscMetadata) {
        macro_rules! fill {
            ($field:ident) => {
                if self.$field.is_none() {
                    self.$field = other.$field;
                }
            };
        }
        fill!(album_title);
        fill!(album_artist);
        fill!(year);
        fill!(genre);
        fill!(mb_release_id);
        fill!(mb_disc_id);
        fill!(cddb_disc_id);
        fill!(cover_art_url);

        if self.track_titles.len() < other.track_titles.len() {
            self.track_titles.resize(other.track_titles.len(), None);
        }
        for (i, title) in other.track_titles.into_iter().enumerate() {
            if self.track_titles.get(i).and_then(|t| t.as_ref()).is_none() {
                if i < self.track_titles.len() {
                    self.track_titles[i] = title;
                }
            }
        }

        self.sources.extend(other.sources);
    }
}

/// Which backends to enable and any per-backend config.
#[derive(Debug, Clone)]
pub struct LookupConfig {
    pub use_musicbrainz: bool,
    pub use_gnudb: bool,
    pub use_itunes: bool,
    pub gnudb_email: String,
    pub user_agent: String,
}

impl Default for LookupConfig {
    fn default() -> Self {
        Self {
            use_musicbrainz: true,
            use_gnudb: true,
            use_itunes: true,
            gnudb_email: "mgamerdinge146@gmail.com".to_string(),
            user_agent: format!("cdrip/{} (https://github.com/Kazooki123/cdrip.rs)", env!("CARGO_PKG_VERSION")),
        }
    }
}

// Orchestrator
/// Run all enabled lookup backends and merge their results.
/// Order: MusicBrainz -> GnuDB -> iTunes (enrichment/cover only).
/// Stops after the first backend that returns useful metadata,
/// then runs iTunes on top for cover art if available.
pub fn lookup_all(toc: &DiscToc, config: &LookupConfig) -> DiscMetadata {
    let mut meta = DiscMetadata::default();
    meta.track_titles = vec![None; toc.track_count() as usize];
    meta.track_artists = vec![None; toc.track_count() as usize];

    // 1. MusicBrainz
    if config.use_musicbrainz {
        info!("Trying MusicBrainz lookup...");
        match brainz::lookup(toc, config) {
            Ok(mb_meta) => {
                debug!("MusicBrainz: {:?}", mb_meta.album_title);
                meta.merge_from(mb_meta);
            }
            Err(e) => {
                debug!("MusicBrainz lookup failed: {}", e);
            }
        }
    }

    // 2. GNUDB
    if config.use_gnudb && !meta.is_useful() {
        info!("Trying GnuDB lookup...");
        match gnudb::lookup(toc, config) {
            Ok(gn_meta) => {
                debug!("GnuDB: {:?}", gn_meta.album_title);
                meta.merge_from(gn_meta);
            }
            Err(e) => {
                debug!("GnuDB lookup failed: {}", e);
            }
        }
    }

    // 3. iTunes (cover art, Apple Music link)
    //    Only runs if we already have an album title to search with.
    if config.use_itunes && meta.album_title.is_some() && meta.cover_art_url.is_none() {
        let artist = meta.album_artist.as_deref().unwrap_or("");
        let album = meta.album_title.as_deref().unwrap_or("");
        info!("Trying iTunes enrichment for \"{}\" by \"{}\"...", album, artist);
        match itunes::lookup(artist, album, config) {
            Ok(it_meta) => {
                debug!("iTunes cover art: {:?}", it_meta.cover_art_url);
                meta.merge_from(it_meta);
            }
            Err(e) => {
                debug!("iTunes lookup failed: {}", e);
            }
        }
    }

    meta
}

#[derive(Debug)]
pub enum LookupError {
    Http(String),
    Parse(String),
    NotFound,
    NoDiscId,
}

impl std::fmt::Display for LookupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LookupError::Http(e)  => write!(f, "HTTP error: {}", e),
            LookupError::Parse(e) => write!(f, "Parse error: {}", e),
            LookupError::NotFound => write!(f, "No match found"),
            LookupError::NoDiscId => write!(f, "Could not compute disc ID from TOC"),
        }
    }
}

pub type LookupResult = std::result::Result<DiscMetadata, LookupError>;
