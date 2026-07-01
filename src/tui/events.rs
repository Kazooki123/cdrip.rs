use crate::tui::app::{App, Screen};
pub use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Handle a single key event, mutating `app` in place.
///
/// `on_drive_confirmed` / `on_rip_requested` are reported back to the caller
/// via the return value so `mod.rs` can kick off I/O (reading TOC, spawning
/// the rip thread) without `event.rs` needing direct access to those systems.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputAction {
    None,
    Quit,
    DriveConfirmed(String),
    StartRip,
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> InputAction {
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            return InputAction::Quit;
        }
        KeyCode::Char('q') if app.screen != Screen::Ripping => {
            return InputAction::Quit;
        }
        KeyCode::Char('t') => {
            app.toggle_theme();
            return InputAction::None;
        }
        _ => {}
    }

    match app.screen {
        Screen::DriveSelect => handle_drive_select(app, key),
        Screen::TrackList => handle_track_list(app, key),
        Screen::Ripping => handle_ripping(app, key),
        Screen::Done => handle_done(app, key),
    }
}

fn handle_drive_select(app: &mut App, key: KeyEvent) -> InputAction {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => app.move_drive_cursor(-1),
        KeyCode::Down | KeyCode::Char('j') => app.move_drive_cursor(1),
        KeyCode::Enter => {
            app.confirm_drive();
            if let Some(device) = app.selected_device.clone() {
                return InputAction::DriveConfirmed(device);
            }
        }
        _ => {}
    }
    InputAction::None
}

fn handle_track_list(app: &mut App, key: KeyEvent) -> InputAction {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => app.move_track_cursor(-1),
        KeyCode::Down | KeyCode::Char('j') => app.move_track_cursor(1),
        KeyCode::Char(' ') => app.toggle_track_selected(),
        KeyCode::Char('a') => app.select_all_tracks(true),
        KeyCode::Char('n') => app.select_all_tracks(false),
        KeyCode::Char('f') => app.toggle_format(),
        KeyCode::Enter => {
            if !app.selected_track_numbers().is_empty() {
                return InputAction::StartRip;
            }
        }
        KeyCode::Esc => {
            app.screen = Screen::DriveSelect;
        }
        _ => {}
    }
    InputAction::None
}

fn handle_ripping(_app: &mut App, key: KeyEvent) -> InputAction {
    // While ripping, only allow Ctrl+C to quit (handled globally above).
    // We'll intentionally don't let 'q' interrupt a rip mid flight...
    let _ = key;
    InputAction::None
}

fn handle_done(app: &mut App, key: KeyEvent) -> InputAction {
    match key.code {
        KeyCode::Enter | KeyCode::Esc => {
            app.screen = Screen::TrackList;
            app.push_log("Returned to track list".to_string());
        }
        _ => {}
    }
    InputAction::None
}
