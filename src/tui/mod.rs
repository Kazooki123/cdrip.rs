//! Terminal UI for cdrip - an EAC-style ripping interface.
//!
//! Screens flow: DriveSelect -> TrackList -> Ripping -> Done -> (back to TrackList).
//!
//! Ripping runs on a background thread (see `job.rs`) so the render loop
//! never blocks on drive I/O; progress is delivered via an `mpsc` channel and
//! drained once per tick.

pub mod app;
pub mod events;
pub mod job;
pub mod theme;
pub mod ui;

use crate::{drive::list_drives, toc::read_toc};
use app::{App, Screen};
use crossterm::event::{self, Event};
use events::InputAction;
use ratatui::DefaultTerminal;
use job::{spawn_rip_job, RipEvent, TuiRipConfig};
use std::{
    path::PathBuf,
    sync::mpsc,
    time::Duration,
};

pub fn run(output_dir: PathBuf) -> anyhow::Result<()> {
    let drives = list_drives();

    let mut terminal = ratatui::init();
    let result = run_app(&mut terminal, drives, output_dir);
    ratatui::restore();
    result
}

fn run_app(
    terminal: &mut DefaultTerminal,
    drives: Vec<crate::drive::DriveInfo>,
    output_dir: PathBuf,
) -> anyhow::Result<()> {
    let mut app = App::new(drives, output_dir);
    app.push_log("cdrip TUI started (YACR). Select a drive to begin.".to_string());

    // Channels for background ripping (job) progress events
    let (tx, rx) = mpsc::channel::<RipEvent>();

    loop {
        terminal.draw(|frame| ui::draw(frame, &app))?;

        if app.should_quit {
            break;
        }

        // Drain ANY pending ripping events w/o blocking the render loop
        while let Ok(evt) = rx.try_recv() {
            app.apply_rip_event(evt);
        }

        if event::poll(Duration::from_millis(80))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != event::KeyEventKind::Press {
                    continue;
                }

                let action = events::handle_key(&mut app, key);
                handle_action(&mut app, action, &tx)?;
            }
        }
    }

    Ok(())
}

fn handle_action(
    app: &mut App,
    action: InputAction,
    tx: &mpsc::Sender<RipEvent>,
) -> anyhow::Result<()> {
    match action {
        InputAction::None => {}
        InputAction::Quit => {
            app.should_quit = true;
        }
        InputAction::DriveConfirmed(device) => {
            app.push_log(format!("Reading TOC from {}...", device));
            match open_and_read_toc(&device) {
                Ok(toc) => {
                    app.push_log(format!("Found {} track(s)", toc.track_count()));
                    app.set_toc(toc);
                }
                Err(e) => {
                    app.push_log(format!("Failed to read TOC: {}", e));
                    app.screen = Screen::DriveSelect;
                }
            }
        }
        InputAction::StartRip => {
            let tracks = app.selected_track_numbers();
            if tracks.is_empty() {
                app.push_log("No tracks selected.".to_string());
                return Ok(());
            }

            let device = app
                .selected_device
                .clone()
                .unwrap_or_else(|| "/dev/sr0".to_string());

            app.start_ripping();

            if let Some(toc) = app.toc.clone() {
                let job_config = TuiRipConfig {
                    device_path: device,
                    output_dir: app.output_dir.clone(),
                    format: app.format,
                    tracks,
                };
                spawn_rip_job(job_config, toc, tx.clone());
            }
        }
    }
    Ok(())
}

fn open_and_read_toc(device: &str) -> anyhow::Result<crate::toc::DiscToc> {
    let reader = crate::drive::open_drive(device)?;
    let (toc, _raw) = read_toc(&reader)?;
    Ok(toc)
}
