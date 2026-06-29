# CDRip.rs 🎵💽

A fast, accurate CD ripper written in pure Rust. **No GNU dependencies**, no `cdparanoia` system library required.

- **Demo**: Ripping a **Nirvana In Utero 2CD Deluxe Edition**
![demo](https://codeberg.org/Kazooki123/cdrip.rs/raw/branch/main/demo.png)

## Features

- **Cross-platform** — Windows, Linux, macOS (via direct SCSI commands through `cd-da-reader`)
- **WAV & FLAC output** — pure-Rust encoders, no native libs needed
- **Retry logic** — configurable per-sector retry count with silence substitution fallback
- **Progress display** — live sector-level progress bars with error/retry counters
- **Rip manifest** — JSON log of every track's result, file size, errors, retries
- **TUI-ready** — progress layer designed to be promoted to a full TUI (coming soon)

## Build

```sh
cargo build --release
```

## Install

```sh
cargo install cdrip
```

## Usage

```sh
# List detected drives
cdrip list

# Show disc Table of Contents
cdrip toc
cdrip toc --device /dev/sr1        # specific drive

# Rip entire disc to FLAC (default)
cdrip rip

# Rip to WAV
cdrip rip --format wav

# Rip with ID lookups (musicbrainz, gnudb.org, itunes)
cdrip rip --lookup
cdrip rip --lookup --cue     # populates cue sheet
cdrip rip --lookup --cd-text # wins over cd-text for cue fields

# Generate cue sheets
cdrip rip --cue

# Reads a CD-TEXT
cdrip rip --cd-text

# CUE sheet with CD-TEXT populated
cdrip rip --cue --cd-text

# Parallel Encoding
cdrip rip --parallel

# Detect if a CD has index 00 (also known as HTOA)
cdrip rip --hidden

# Rip to a specific directory
cdrip rip --out ~/Music/rips/

# Rip only track 3
cdrip rip --track 3

# Rip with specific device + more retries, keep going on errors
cdrip rip --device /dev/sr0 --retries 10 --skip-errors

# Verbose logging
cdrip rip -v       # info
cdrip rip -vv      # debug
cdrip rip -vvv     # trace
```

## Output

```txt
Music/rips/
├── track01.flac
├── track02.flac
├── ...
└── cd-manifest.json
```

`cd-manifest.json` records the rip timestamp, format, per-track sector counts, byte sizes, retry counts, and error counts.

## Architecture

```txt
src/
├── main.rs         CLI
├── drive.rs        Drive detection & opening (cross-platform)
├── toc.rs          TOC parsing and pretty printer
├── ripper.rs       Sector-level rip loop + retry logic + manifest
├── progress.rs     indicatif progress bars and spinner (weeeeeee)
├── error.rs        thiserror error types
├── parallel.rs     parallel ripping
├── cdtext.rs       CD-TEXT reader
├── cue.rs          Cue Sheets generator
├── htoa.rs         Hidden Track One Audio detection
└── encoder/
    ├── mod.rs      Encoder trait and OutputFormat `enum`
    ├── wav.rs      WAV encoder (hand-rolled RIFF header)
    └── flac.rs     FLAC encoder (flac-codec)
└── id/
    ├── brainz.rs   MusicBrainz id lookup (musicbrainz_rs)
    ├── itunes.rs   iTunes for enrichment (usually cover arts)
    ├── gnudb.rs    GnuDB id lookup
    └── mod.rs      LookupConfig `struct`
```

## Roadmap & TODOs

- [ ] TUI frontend (ratatui) with live waveform preview and sector error heatmap
- [x] MusicBrainz disc ID lookup (disc ID derivable from TOC — no extra deps needed)
- [x] CD-TEXT reading (where supported by drive)
- [x] Parallel track ripping
- [x] CUE sheet generation

## License

Under the **MIT** License.
