mod drive;
mod encoder;
mod error;
mod progress;
mod ripper;
mod toc;

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
    author,
    long_about = None
)]
struct Cli {
    /// Verbose logging (use -v, -vv for more)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    List,
    Toc {
        /// Device path (auto-detected if omitted)
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
        } => cmd_rip(device.as_deref(), out, format, track, retries, skip_errors),
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
    let spinner = Spinner::new("Reading Table of Contents…");

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
) -> anyhow::Result<()> {
    let spinner = Spinner::new("Opening drive…");
    let reader = match device {
        Some(path) => open_drive(path)?,
        None => open_default_drive()?,
    };
    spinner.finish_ok("Drive opened");

    let spinner = Spinner::new("Reading Table of Contents…");
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

    let config = RipConfig {
        output_dir: out,
        format,
        max_retries,
        skip_errors,
        track_filter,
    };

    let ripper = Ripper::new(&reader, &toc, &raw_toc, &config);
    let manifest = ripper.run()?;

    println!(
        "  {} Manifest: {}\n",
        style("·").cyan(),
        style("cd-manifest.json").dim()
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
