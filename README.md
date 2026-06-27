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

> [!WARNING]
> Doesn't exist yet, i recommend building it for now but don't worry it'll be in **crates.io** soon :3

```sh
cargo install cdrip-rs
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
└── encoder/
    ├── mod.rs      Encoder trait and OutputFormat `enum`
    ├── wav.rs      WAV encoder (hand-rolled RIFF header)
    └── flac.rs     FLAC encoder (flac-codec)
```

## Roadmap & TODOs

- [ ] TUI frontend (ratatui) with live waveform preview and sector error heatmap
- [ ] MusicBrainz disc ID lookup (disc ID derivable from TOC — no extra deps needed)
- [ ] CD-TEXT reading (where supported by drive)
- [ ] Parallel track ripping
- [ ] CUE sheet generation

## License

Under the **MIT** License.
