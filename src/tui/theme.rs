//! Color themes for the TUI.
//!
//! `Classic` mimics the look of Exact Audio Copy / old Windows rippers:
//! grey chrome, blue borders, yellow highlights, green for success, red for
//! errors. `Midnight` is a higher-contrast dark variant for terminals with
//! true-color support. Toggle with `t` in the TUI.

use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeKind {
    Classic,
    Midnight,
}

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub kind: ThemeKind,
    pub border: Color,
    pub border_focused: Color,
    pub title: Color,
    pub background_hint: Color,
    pub text: Color,
    pub text_dim: Color,
    pub text_highlight: Color,
    pub status_ok: Color,
    pub status_warn: Color,
    pub status_error: Color,
    pub status_pending: Color,
    pub status_active: Color,
    pub gauge_fg: Color,
    pub gauge_bg: Color,
    pub selection_bg: Color,
    pub selection_fg: Color,
}

impl Theme {
    // TODO: Add more themes such as Cattpuccin themes, orange themes, coffee-shade themes, etc.
    pub fn classic() -> Self {
        Self {
            kind: ThemeKind::Classic,
            border: Color::Rgb(90, 110, 140),
            border_focused: Color::Rgb(120, 170, 230),
            title: Color::Rgb(220, 220, 230),
            background_hint: Color::Rgb(40, 44, 52),
            text: Color::Rgb(210, 210, 215),
            text_dim: Color::Rgb(130, 135, 145),
            text_highlight: Color::Rgb(255, 215, 100),
            status_ok: Color::Rgb(110, 200, 120),
            status_warn: Color::Rgb(230, 190, 90),
            status_error: Color::Rgb(220, 90, 90),
            status_pending: Color::Rgb(130, 135, 145),
            status_active: Color::Rgb(120, 170, 230),
            gauge_fg: Color::Rgb(120, 170, 230),
            gauge_bg: Color::Rgb(50, 55, 65),
            selection_bg: Color::Rgb(60, 90, 130),
            selection_fg: Color::White,
        }
    }

    pub fn midnight() -> Self {
        Self {
            kind: ThemeKind::Midnight,
            border: Color::Rgb(70, 80, 110),
            border_focused: Color::Rgb(180, 100, 240),
            title: Color::Rgb(230, 230, 240),
            background_hint: Color::Rgb(18, 18, 26),
            text: Color::Rgb(225, 225, 235),
            text_dim: Color::Rgb(110, 110, 130),
            text_highlight: Color::Rgb(255, 140, 220),
            status_ok: Color::Rgb(90, 230, 160),
            status_warn: Color::Rgb(255, 200, 80),
            status_error: Color::Rgb(255, 100, 110),
            status_pending: Color::Rgb(110, 110, 130),
            status_active: Color::Rgb(180, 100, 240),
            gauge_fg: Color::Rgb(180, 100, 240),
            gauge_bg: Color::Rgb(35, 35, 48),
            selection_bg: Color::Rgb(90, 50, 130),
            selection_fg: Color::White,
        }
    }

    pub fn toggled(&self) -> Self {
        match self.kind {
            ThemeKind::Classic => Self::midnight(),
            ThemeKind::Midnight => Self::classic(),
        }
    }

    pub fn border_style(&self, focused: bool) -> Style {
        Style::default().fg(if focused { self.border_focused } else { self.border })
    }

    pub fn title_style(&self) -> Style {
        Style::default().fg(self.title).add_modifier(Modifier::BOLD)
    }

    pub fn text_style(&self) -> Style {
        Style::default().fg(self.text)
    }

    pub fn dim_style(&self) -> Style {
        Style::default().fg(self.text_dim)
    }

    pub fn highlight_style(&self) -> Style {
        Style::default().fg(self.text_highlight).add_modifier(Modifier::BOLD)
    }

    pub fn selection_style(&self) -> Style {
        Style::default()
            .bg(self.selection_bg)
            .fg(self.selection_fg)
            .add_modifier(Modifier::BOLD)
    }

    pub fn status_style(&self, status: StatusKind) -> Style {
        let color = match status {
            StatusKind::Ok => self.status_ok,
            StatusKind::Warn => self.status_warn,
            StatusKind::Error => self.status_error,
            StatusKind::Pending => self.status_pending,
            StatusKind::Active => self.status_active,
        };
        Style::default().fg(color)
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::classic()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    Ok,
    Warn,
    Error,
    Pending,
    Active,
}
