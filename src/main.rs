///
/// ▄█████ ████▄  █████▄  ██ █████▄ 
/// ██     ██  ██ ██▄▄██▄ ██ ██▄▄█▀ 
/// ▀█████ ████▀  ██   ██ ██ ██     
///
/// CDRIP.RS
/// COPYRIGHT 2026
/// KAZOOKI123                             

#[allow(unused)]

mod cdtext;
mod cue;
mod htoa;
mod drive;
mod parallel;
mod encoder;
mod error;
mod progress;
mod ripper;
mod toc;
mod id;

use crate::{
    drive::{list_drives, open_default_drive, open_drive},
    encoder::OutputFormat,
    progress::Spinner,
    ripper::{RipConfig, Ripper},
    toc::{print_toc, read_toc},
};
use clap::{Parser, Subcommand};
use console::style;
use std::path::PathBuf;
use tracing::Level;

#[derive(Parser)]
#[command(
    name = "cdrip",
    about = "A fast, accurate CD ripper in pure Rust 💿",
    version,
    author = "Kazooki123",
    long_about = None
)]

struct Cli {
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    List,
    Toc {
        #[arg(short, long)]
        device: Option<String>,
    },
    Rip {
        /// Device path (auto-detected if omitted)
        #[arg(short, long)]
        device: Option<String>,

        /// Output directory [default: current directory]
        #[arg(short, long, default_value = ".")]
        out: PathBuf,

        /// Output format
        #[arg(short, long, value_enum, default_value_t = OutputFormat::Flac)]
        format: OutputFormat,

        /// Rip only this track number (1-based)
        #[arg(short, long)]
        track: Option<u8>,

        /// Max sector read retries before giving up
        #[arg(long, default_value_t = 5)]
        retries: u8,

        /// Continue ripping even if a track fails
        #[arg(long)]
        skip_errors: bool,

        /// Generate a CUE sheet alongside the ripped tracks
        #[arg(long)]
        cue: bool,

        /// Attempt to read CD-TEXT from the disc (best-effort, many drives unsupported)
        #[arg(long)]
        cd_text: bool,

        /// Encode tracks in parallel (parallelises the CPU-bound encoding step)
        #[arg(long)]
        parallel: bool,

        /// Detect and extract HTOA from Track 1 pregap (IDX 00)
        #[arg(long)]
        hidden: bool,

        /// Look up disc metadata (MusicBrainz -> GnuDB -> iTunes).
        /// Populates the manifest and CUE sheet file with album/track infos.
        #[arg(long)]
        lookup: bool,
    },
}

// Entry point
fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    setup_tracing(cli.verbose);

    match cli.command {
        Commands::List => cmd_list(),
        Commands::Toc { device } => cmd_toc(device.as_deref()),
        Commands::Rip {
            device,
            out,
            format,
            track,
            retries,
            skip_errors,
            cue,
            cd_text,
            parallel,
            hidden,
            lookup,
        } => cmd_rip(device.as_deref(), out, format, track, retries, skip_errors, cue, cd_text, parallel, hidden, lookup),
    }
}

// Subcommand implementations
fn cmd_list() -> anyhow::Result<()> {
    println!("\n  {} Scanning for optical drives…\n", style("●").cyan());

    let drives = list_drives();

    if drives.is_empty() {
        println!("  {} No optical drives detected.", style("✗").red());
        println!(
            "  {} Make sure a CD drive is connected and a disc is inserted.\n",
            style("·").dim()
        );
        return Ok(());
    }

    println!(
        "  {:>4}  {}",
        style("#").bold(),
        style("Device path").bold()
    );
    println!("  {}", style("─".repeat(40)).dim());

    for (i, drive) in drives.iter().enumerate() {
        println!("  {:>4}  {}", style(i + 1).yellow(), drive);
    }

    println!();
    Ok(())
}

fn cmd_toc(device: Option<&str>) -> anyhow::Result<()> {
    let spinner = Spinner::new("Reading Table of Contents...");

    let reader = match device {
        Some(path) => open_drive(path)?,
        None => open_default_drive()?,
    };

    let toc = match read_toc(&reader) {
        Ok((t, _)) => {
            spinner.finish_ok(format!("Found {} track(s)", t.track_count()));
            t
        }
        Err(e) => {
            spinner.finish_err(e.to_string());
            return Err(e.into());
        }
    };

    print_toc(&toc);
    Ok(())
}

fn cmd_rip(
    device: Option<&str>,
    out: PathBuf,
    format: OutputFormat,
    track_filter: Option<u8>,
    max_retries: u8,
    skip_errors: bool,
    gen_cue: bool,
    try_cd_text: bool,
    use_parallel: bool,
    check_hidden: bool,
    do_lookup: bool,
) -> anyhow::Result<()> {
    use crate::{
        cdtext::read_cd_text,
        cue::{write_cue, CueMetadata},
        htoa::{detect_htoa, extract_htoa, HtoaStatus},
        id::{lookup_all, LookupConfig},
        parallel::default_thread_count,
    };

    let spinner = Spinner::new("Opening drive…");
    let reader = match device {
        Some(path) => open_drive(path)?,
        None => open_default_drive()?,
    };
    spinner.finish_ok("Drive opened");

    let spinner = Spinner::new("Reading Table of Contents...");
    let (toc, raw_toc) = match read_toc(&reader) {
        Ok((t, r)) => {
            spinner.finish_ok(format!("Found {} track(s)", t.track_count()));
            (t, r)
        }
        Err(e) => {
            spinner.finish_err(e.to_string());
            return Err(e.into());
        }
    };

    print_toc(&toc);

    let cd_text = if try_cd_text {
        let spinner = Spinner::new("Reading CD-TEXT…");
        let dev_path = device.unwrap_or("/dev/sr0");
        match read_cd_text(dev_path, toc.track_count()) {
            Ok(Some(data)) => {
                spinner.finish_ok(format!(
                    "CD-TEXT found: '{}'",
                    data.album_title.as_deref().unwrap_or("(no title)")
                ));
                Some(data)
            }
            Ok(None) => {
                spinner.finish_ok("No CD-TEXT on this disc");
                None
            }
            Err(e) => {
                spinner.finish_err(format!("CD-TEXT unavailable: {}", e));
                None
            }
        }
    } else {
        None
    };

    let lookup_meta = if do_lookup {
        let spinner = Spinner::new("Looking up disc metadata...");
        let cfg = LookupConfig::default();
        let meta = lookup_all(&toc, &cfg);

        if meta.is_useful() {
            spinner.finish_ok(format!(
                "Found: '{}' '{}'",
                meta.album_title.as_deref().unwrap_or("?"),
                meta.album_artist
                    .as_deref()
                    .map(|a| format!(" by {}" ,a))
                    .unwrap_or_default()
            ));
        } else {
            spinner.finish_ok("No match found - ripping without metadata...");
        }
        Some(meta)
    } else {
        None
    };

    if use_parallel {
        println!(
            "  {} Parallel encoding enabled ({} threads)",
            style("·").cyan(),
            default_thread_count()
        );
    }

    let config = RipConfig {
        output_dir: out.clone(),
        format,
        max_retries,
        skip_errors,
        track_filter,
    };

    let ripper = Ripper::new(&reader, &toc, &raw_toc, &config);
    let manifest = ripper.run()?;

    if check_hidden {
        let dev_path = device.unwrap_or("/dev/sr0");
        let spinner = Spinner::new("Checking for hidden tracks...");
        let status = detect_htoa(dev_path, &toc);

        match &status {
            HtoaStatus::DriveUnsupported => {
                spinner.finish_err("Drive does NOT support pregap reads!");
            }
            HtoaStatus::NoPregap => {
                spinner.finish_ok("No HTOA -  standard 2-second lead-in only!");
            }
            HtoaStatus::SilentPregap { sectors } => {
                spinner.finish_ok(format!(
                    "Pregap present ({} sectors) but silent.. no hidden audio",
                    sectors
                ));
            }
            HtoaStatus::HtoaDetected { sectors: _, duration_secs } => {
                spinner.finish_ok(format!(
                    "HTOA detected! {:.1}s of hidden audio - extracting...",
                    duration_secs
                ));
                match extract_htoa(dev_path, &toc, format, &out) {
                    Ok(Some(path)) => println!(
                        "   {} Hidden Track -> {}",
                        style("✔️").green().bold(),
                        style(path.display().to_string()).dim()
                    ),
                    Ok(None) => println!(
                        "   {} HTOA read returned empty (silent or driver issue)",
                        style(".").yellow()
                    ),
                    Err(e) => println!(
                        "   {} HTOA extraction failed: {}",
                        style("⚠️").yellow(), e
                    ),
                }
            }
        }
    }

    if gen_cue {
        let mut meta = CueMetadata::empty(toc.track_count());
        if let Some(ref lm) = lookup_meta {
            meta.album_title   = lm.album_title.clone();
            meta.album_artist  = lm.album_artist.clone();
            meta.track_titles  = lm.track_titles.clone();
            meta.track_artists = lm.track_artists.clone();
        } else if let Some(ref cd) = cd_text {
            meta.album_title   = cd.album_title.clone();
            meta.album_artist  = cd.album_artist.clone();
            meta.track_titles  = cd.track_titles.iter()
                .map(|t| t.clone())
                .collect();
            meta.track_artists = cd.track_artists.iter()
                .map(|a| a.clone())
                .collect();
        }
        match write_cue(&toc, format, &out, &meta) {
            Ok(path) => println!(
                "  {} CUE: {}",
                style("·").cyan(),
                style(path.display().to_string()).dim()
            ),
            Err(e) => println!(
                "  {} CUE write failed: {}",
                style("⚠").yellow(), e
            ),
        }
    }

    println!(
        "\n  {} Manifest: {}\n",
        style("·").cyan(),
        style("cdrip-manifest.json").dim()
    );

    if manifest.total_errors > 0 {
        println!(
            "  {} {} sector error(s) encountered — see manifest for details\n",
            style("⚠").yellow(),
            manifest.total_errors
        );
    }

    Ok(())
}

// Logging setup
fn setup_tracing(verbosity: u8) {
    let level = match verbosity {
        0 => Level::WARN,
        1 => Level::INFO,
        2 => Level::DEBUG,
        _ => Level::TRACE,
    };

    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_target(false)
        .without_time()
        .init();
}
