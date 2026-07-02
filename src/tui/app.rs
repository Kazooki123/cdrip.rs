//! Application state for the TUI.

use crate::{
    drive::DriveInfo,
    encoder::OutputFormat,
    toc::DiscToc,
    tui::{
        job::RipEvent,
        theme::Theme,
    },
};
use std::{collections::VecDeque, time::Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    DriveSelect,
    TrackList,
    Ripping,
    Done,
}

// PER-TRACK UNITS
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackStatus {
    Pending,
    Active,
    Done,
    Failed,
}

#[derive(Debug, Clone)]
pub struct TrackUiState {
    pub status: TrackStatus,
    pub sectors_done: u32,
    pub sectors_total: u32,
    pub error_count: u32,
    pub output_file: Option<String>,
    pub fail_reason: Option<String>,
}

impl TrackUiState {
    pub fn new(sectors_total: u32) -> Self {
        Self {
            status: TrackStatus::Pending,
            sectors_done: 0,
            sectors_total,
            error_count: 0,
            output_file: None,
            fail_reason: None,
        }
    }

    pub fn percent(&self) -> f64 {
        if self.sectors_total == 0 {
            return 0.0;
        }
        (self.sectors_done as f64 / self.sectors_total as f64 * 100.0).min(100.0)
    }
}

pub struct App {
    pub theme: Theme,
    pub screen: Screen,
    pub should_quit: bool,
    pub drives: Vec<DriveInfo>,
    pub drive_cursor: usize,
    pub selected_device: Option<String>,
    pub toc: Option<DiscToc>,
    pub track_cursor: usize,

    /// Which tracks are checked for ripping (index = track_number - 1)
    pub track_selected: Vec<bool>,
    pub format: OutputFormat,

    // Ripping state
    pub track_ui: Vec<TrackUiState>,
    pub overall_done: u8,
    pub overall_total: u8,
    pub rip_started_at: Option<Instant>,
    pub rip_finished: bool,
    pub ripped_count: u8,
    pub failed_count: u8,

    pub log: VecDeque<String>,
    pub log_capacity: usize,
    pub output_dir: std::path::PathBuf,
    pub status_message: Option<String>,
}

impl App {
    pub fn new(drives: Vec<DriveInfo>, output_dir: std::path::PathBuf) -> Self {
        Self {
            theme: Theme::classic(),
            screen: Screen::DriveSelect,
            should_quit: false,

            drives,
            drive_cursor: 0,
            selected_device: None,

            toc: None,
            track_cursor: 0,
            track_selected: Vec::new(),
            format: OutputFormat::Flac,

            track_ui: Vec::new(),
            overall_done: 0,
            overall_total: 0,
            rip_started_at: None,
            rip_finished: false,
            ripped_count: 0,
            failed_count: 0,

            log: VecDeque::new(),
            log_capacity: 200,

            output_dir,
            status_message: None,
        }
    }

    pub fn push_log(&mut self, line: impl Into<String>) {
        self.log.push_back(line.into());
        while self.log.len() > self.log_capacity {
            self.log.pop_front();
        }
    }

    pub fn move_drive_cursor(&mut self, delta: i32) {
        if self.drives.is_empty() {
            return;
        }
        let len = self.drives.len() as i32;
        let new = (self.drive_cursor as i32 + delta).rem_euclid(len);
        self.drive_cursor = new as usize;
    }

    pub fn confirm_drive(&mut self) {
        if let Some(d) = self.drives.get(self.drive_cursor) {
            self.selected_device = Some(d.path.clone());
        }
    }

    pub fn set_toc(&mut self, toc: DiscToc) {
        let count = toc.track_count() as usize;
        self.track_selected = vec![true; count]; // all selected by default
        self.track_ui = toc
            .tracks
            .iter()
            .map(|t| TrackUiState::new(t.sector_count))
            .collect();
        self.toc = Some(toc);
        self.track_cursor = 0;
        self.screen = Screen::TrackList;
    }

    pub fn move_track_cursor(&mut self, delta: i32) {
        let Some(toc) = &self.toc else { return };
        let len = toc.track_count() as i32;
        if len == 0 {
            return;
        }
        let new = (self.track_cursor as i32 + delta).rem_euclid(len);
        self.track_cursor = new as usize;
    }

    pub fn toggle_track_selected(&mut self) {
        if let Some(sel) = self.track_selected.get_mut(self.track_cursor) {
            *sel = !*sel;
        }
    }

    pub fn select_all_tracks(&mut self, value: bool) {
        for s in &mut self.track_selected {
            *s = value;
        }
    }

    pub fn toggle_format(&mut self) {
        self.format = match self.format {
            OutputFormat::Flac => OutputFormat::Wav,
            OutputFormat::Wav  => OutputFormat::Flac,
            OutputFormat::Mp3  => OutputFormat::Mp3,
        };
    }

    pub fn selected_track_numbers(&self) -> Vec<u8> {
        self.track_selected
            .iter()
            .enumerate()
            .filter(|(_, sel)| **sel)
            .map(|(i, _)| (i + 1) as u8)
            .collect()
    }

    pub fn start_ripping(&mut self) {
        let selected = self.selected_track_numbers();
        self.overall_total = selected.len() as u8;
        self.overall_done = 0;
        self.rip_started_at = Some(Instant::now());
        self.rip_finished = false;
        self.ripped_count = 0;
        self.failed_count = 0;
        self.screen = Screen::Ripping;
        self.push_log(format!("Starting rip of {} track(s)...", selected.len()));
    }

    pub fn apply_rip_event(&mut self, event: RipEvent) {
        match event {
            RipEvent::TrackStarted { track, total_sectors } => {
                if let Some(ui) = self.track_ui.get_mut((track - 1) as usize) {
                    ui.status = TrackStatus::Active;
                    ui.sectors_total = total_sectors;
                    ui.sectors_done = 0;
                }
                self.push_log(format!("Track {:02}: ripping started", track));
            }
            RipEvent::SectorProgress { track, sectors_done } => {
                if let Some(ui) = self.track_ui.get_mut((track - 1) as usize) {
                    ui.sectors_done = sectors_done;
                }
            }
            RipEvent::TrackError { track, message } => {
                if let Some(ui) = self.track_ui.get_mut((track - 1) as usize) {
                    ui.error_count += 1;
                }
                self.push_log(format!("Track {:02}: {}", track, message));
            }
            RipEvent::TrackFinished { track, output_file } => {
                if let Some(ui) = self.track_ui.get_mut((track - 1) as usize) {
                    ui.status = TrackStatus::Done;
                    ui.output_file = Some(output_file.clone());
                    ui.sectors_done = ui.sectors_total;
                }
                self.overall_done += 1;
                self.ripped_count += 1;
                self.push_log(format!("Track {:02}: done -> {}", track, output_file));
            }
            RipEvent::TrackFailed { track, reason } => {
                if let Some(ui) = self.track_ui.get_mut((track - 1) as usize) {
                    ui.status = TrackStatus::Failed;
                    ui.fail_reason = Some(reason.clone());
                }
                self.overall_done += 1;
                self.failed_count += 1;
                self.push_log(format!("Track {:02}: FAILED - {}", track, reason));
            }
            RipEvent::AllDone { ripped, failed } => {
                self.rip_finished = true;
                self.ripped_count = ripped;
                self.failed_count = failed;
                self.screen = Screen::Done;
                self.push_log(format!(
                    "Rip session complete: {} ripped, {} failed",
                    ripped, failed
                ));
            }
            RipEvent::Log(line) => self.push_log(line),
        }
    }

    pub fn toggle_theme(&mut self) {
        self.theme = self.theme.toggled();
    }
}
