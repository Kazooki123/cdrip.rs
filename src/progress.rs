use crate::toc::TrackInfo;
use console::style;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::time::Instant;

const BAR_WIDTH: u64 = 40;

fn track_bar_style() -> ProgressStyle {
    ProgressStyle::with_template(
        "  {prefix:.cyan.bold} [{bar:40.green/dim}] {pos}/{len} sectors  {msg}",
    )
    .unwrap()
    .progress_chars("█▉▊▋▌▍▎▏ ")
}

fn overall_bar_style() -> ProgressStyle {
    ProgressStyle::with_template(
        "  {prefix:.bold}  [{bar:40.cyan/dim}] {pos}/{len} tracks",
    )
    .unwrap()
    .progress_chars("█▉▊▋▌▍▎▏ ")
}

fn spinner_style() -> ProgressStyle {
    ProgressStyle::with_template("  {spinner:.cyan} {msg}")
        .unwrap()
        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
}

pub struct RipProgress {
    multi: MultiProgress,
    overall_bar: ProgressBar,
    track_bar: Option<ProgressBar>,
    started_at: Instant,
    pub error_count: u32,
    pub retry_count: u32,
}

impl RipProgress {
    pub fn new(total_tracks: u8) -> Self {
        let multi = MultiProgress::new();

        let overall_bar = multi.add(ProgressBar::new(total_tracks as u64));
        overall_bar.set_style(overall_bar_style());
        overall_bar.set_prefix("Overall  ");

        Self {
            multi,
            overall_bar,
            track_bar: None,
            started_at: Instant::now(),
            error_count: 0,
            retry_count: 0,
        }
    }

    pub fn begin_track(&mut self, track: &TrackInfo) {
        // Remove previous track bar if present
        if let Some(old) = self.track_bar.take() {
            old.finish_and_clear();
        }

        let bar = self.multi.add(ProgressBar::new(track.sector_count as u64));
        bar.set_style(track_bar_style());
        bar.set_prefix(format!("Track {:02}", track.number));
        bar.set_message(format!("({})", track.duration_display()));
        self.track_bar = Some(bar);
    }

    pub fn advance_sectors(&self, n: u64) {
        if let Some(bar) = &self.track_bar {
            bar.inc(n);
        }
    }

    pub fn record_retry(&mut self, lba: u32) {
        self.retry_count += 1;
        if let Some(bar) = &self.track_bar {
            bar.set_message(format!(
                "{} retrying sector {}…",
                style("⚠").yellow(),
                lba
            ));
        }
    }

    /// Record an unrecoverable sector error.
    pub fn record_error(&mut self, lba: u32) {
        self.error_count += 1;
        if let Some(bar) = &self.track_bar {
            bar.set_message(format!(
                "{} bad sector at LBA {} (errors: {})",
                style("✗").red(),
                lba,
                self.error_count
            ));
        }
    }

    pub fn finish_track(&mut self, track_num: u8, output: &str) {
        if let Some(bar) = self.track_bar.take() {
            bar.finish_with_message(format!(
                "{} → {}",
                style("✓").green().bold(),
                style(output).dim()
            ));
        }
        self.overall_bar.inc(1);
        self.overall_bar.set_message(format!(
            "last: track {:02}",
            track_num
        ));
    }

    pub fn fail_track(&mut self, track_num: u8) {
        if let Some(bar) = self.track_bar.take() {
            bar.finish_with_message(format!(
                "{} track {:02} FAILED",
                style("✗").red().bold(),
                track_num
            ));
        }
        self.overall_bar.inc(1);
    }

    /// Finish everything and print a summary.
    pub fn finish(&self, ripped: u8, failed: u8) {
        self.overall_bar.finish_and_clear();

        let elapsed = self.started_at.elapsed();
        let secs = elapsed.as_secs();

        println!();
        println!(
            "  {} Rip complete in {}m {}s",
            style("✓").green().bold(),
            secs / 60,
            secs % 60
        );
        println!(
            "  {} {} track(s) ripped successfully",
            style("·").cyan(),
            ripped
        );

        if failed > 0 {
            println!(
                "  {} {} track(s) failed",
                style("·").red(),
                failed
            );
        }

        if self.retry_count > 0 {
            println!(
                "  {} {} sector retries, {} unrecoverable errors",
                style("·").yellow(),
                self.retry_count,
                self.error_count
            );
        }

        println!();
    }
}

pub struct Spinner(ProgressBar);

impl Spinner {
    pub fn new(msg: impl Into<String>) -> Self {
        let pb = ProgressBar::new_spinner();
        pb.set_style(spinner_style());
        pb.set_message(msg.into());
        pb.enable_steady_tick(std::time::Duration::from_millis(80));
        Self(pb)
    }

    pub fn finish_ok(self, msg: impl Into<String>) {
        self.0.finish_with_message(format!(
            "{} {}",
            style("✓").green(),
            msg.into()
        ));
    }

    pub fn finish_err(self, msg: impl Into<String>) {
        self.0.finish_with_message(format!(
            "{} {}",
            style("✗").red(),
            msg.into()
        ));
    }
}
