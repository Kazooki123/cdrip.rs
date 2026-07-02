//! Rendering for the TUI - laid out to evoke classic CD rippers like
//! Exact Audio Copy: a track grid up top, a status/gauge strip, and a
//! scrolling log pane at the bottom.

use crate::tui::app::{App, Screen, TrackStatus};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Gauge, List, ListItem, Paragraph, Row, Table},
    Frame,
};

pub fn draw(frame: &mut Frame, app: &App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // TITLE
            Constraint::Min(8),    // MAIN CONTENT
            Constraint::Length(3), // FOOTER / KEYS
        ])
        .split(frame.area());

    draw_title_bar(frame, app, root[0]);

    match app.screen {
        Screen::DriveSelect => draw_drive_select(frame, app, root[1]),
        Screen::TrackList => draw_track_list(frame, app, root[1]),
        Screen::Ripping => draw_ripping(frame, app, root[1]),
        Screen::Done => draw_done(frame, app, root[1]),
    }

    draw_footer(frame, app, root[2]);
}

fn draw_title_bar(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;

    let screen_label = match app.screen {
        Screen::DriveSelect => "Select Drive",
        Screen::TrackList   => "Track Selection",
        Screen::Ripping     => "Ripping...",
        Screen::Done        => "Rip Complete!",
    };

    let title = Line::from(vec![
        Span::styled(" YACR ", theme.highlight_style().add_modifier(Modifier::REVERSED)),
        Span::raw("  "),
        Span::styled(screen_label, theme.title_style()),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_style(true))
        .title(title)
        .title_alignment(Alignment::Left);

    frame.render_widget(block, area);
}

fn draw_drive_select(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_style(true))
        .title(" Detected Drives ");

    if app.drives.is_empty() {
        let para = Paragraph::new("No optical drives detected.\nConnect a drive and restart cdrip.")
            .style(theme.dim_style())
            .alignment(Alignment::Center)
            .block(block);
        frame.render_widget(para, area);
        return;
    }

    let items: Vec<ListItem> = app
        .drives
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let disc_marker = if d.has_disc { "●" } else { "○" };
            let disc_color = if d.has_disc { theme.status_ok } else { theme.text_dim };

            let line = Line::from(vec![
                Span::styled(format!("{} ", disc_marker), Style::default().fg(disc_color)),
                Span::styled(d.path.clone(), theme.text_style()),
                Span::styled(
                    if d.has_disc { "  (disc present)" } else { "  (no disc)" },
                    theme.dim_style(),
                ),
            ]);

            let style = if i == app.drive_cursor {
                theme.selection_style()
            } else {
                Style::default()
            };

            ListItem::new(line).style(style)
        })
        .collect();

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

fn draw_track_list(frame: &mut Frame, app: &App, area: Rect) {
    let _theme = &app.theme;

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(area);

    draw_track_table(frame, app, cols[0]);
    draw_track_sidebar(frame, app, cols[1]);
}

fn draw_track_table(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;

    let Some(toc) = &app.toc else { return };

    let header = Row::new(vec![
        Cell::from(""),
        Cell::from("Track"),
        Cell::from("Duration"),
        Cell::from("Size"),
    ])
    .style(theme.dim_style().add_modifier(Modifier::BOLD));

    let rows: Vec<Row> = toc
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let checked = app.track_selected.get(i).copied().unwrap_or(false);
            let checkbox = if checked { "[x]" } else { "[ ]" };
            let mib = t.byte_count() as f64 / (1024.0 * 1024.0);

            let is_cursor = i == app.track_cursor;
            let row_style = if is_cursor {
                theme.selection_style()
            } else if checked {
                theme.text_style()
            } else {
                theme.dim_style()
            };

            Row::new(vec![
                Cell::from(checkbox),
                Cell::from(format!("{:02}", t.number)),
                Cell::from(t.duration_display()),
                Cell::from(format!("{:.1} MiB", mib)),
            ])
            .style(row_style)
        })
        .collect();

    let widths = [
        Constraint::Length(4),
        Constraint::Length(8),
        Constraint::Length(10),
        Constraint::Length(12),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_style(true))
        .title(format!(" Tracks ({} total) ", toc.track_count()));

    let table = Table::new(rows, widths).header(header).block(block);
    frame.render_widget(table, area);
}

fn draw_track_sidebar(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Min(4)])
        .split(area);

    // Format + summary panel
    let selected_count = app.selected_track_numbers().len();
    let total_count = app.toc.as_ref().map(|t| t.track_count()).unwrap_or(0);

    let lines = vec![
        Line::from(vec![
            Span::styled("Format:  ", theme.dim_style()),
            Span::styled(app.format.to_string(), theme.highlight_style()),
        ]),
        Line::from(vec![
            Span::styled("Selected: ", theme.dim_style()),
            Span::styled(format!("{}/{}", selected_count, total_count), theme.text_style()),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled("Output:", theme.dim_style())]),
        Line::from(Span::styled(
            app.output_dir.display().to_string(),
            theme.text_style(),
        )),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_style(false))
        .title(" Session ");

    let para = Paragraph::new(lines).block(block);
    frame.render_widget(para, rows[0]);

    draw_log_panel(frame, app, rows[1], " Log ");
}

fn draw_ripping(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // overall gauge
            Constraint::Min(6),     // track grid with per-track status
            Constraint::Length(8),  // log
        ])
        .split(area);

    let overall_pct = if app.overall_total > 0 {
        (app.overall_done as f64 / app.overall_total as f64 * 100.0) as u16
    } else {
        0
    };

    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.border_style(true))
                .title(format!(
                    " Overall: {}/{} tracks ",
                    app.overall_done, app.overall_total
                )),
        )
        .gauge_style(Style::default().fg(theme.gauge_fg).bg(theme.gauge_bg))
        .percent(overall_pct);

    frame.render_widget(gauge, rows[0]);

    draw_track_status_grid(frame, app, rows[1]);
    draw_log_panel(frame, app, rows[2], " Activity Log ");
}

fn draw_track_status_grid(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;

    let Some(toc) = &app.toc else { return };

    let header = Row::new(vec![
        Cell::from("Track"),
        Cell::from("Status"),
        Cell::from("Progress"),
        Cell::from("Errors"),
    ])
    .style(theme.dim_style().add_modifier(Modifier::BOLD));

    let rows: Vec<Row> = toc
        .tracks
        .iter()
        .enumerate()
        .filter(|(i, _)| app.track_selected.get(*i).copied().unwrap_or(false))
        .map(|(i, t)| {
            let ui = &app.track_ui[i];

            let (status_text, status_style) = match ui.status {
                TrackStatus::Pending => ("pending", theme.status_style(crate::tui::theme::StatusKind::Pending)),
                TrackStatus::Active  => ("ripping...", theme.status_style(crate::tui::theme::StatusKind::Active)),
                TrackStatus::Done    => ("done ✓", theme.status_style(crate::tui::theme::StatusKind::Ok)),
                TrackStatus::Failed  => ("FAILED ✗", theme.status_style(crate::tui::theme::StatusKind::Error)),
            };

            let bar_width = 20usize;
            let filled = ((ui.percent() / 100.0) * bar_width as f64) as usize;
            let bar = format!(
                "[{}{}] {:>5.1}%",
                "█".repeat(filled),
                "░".repeat(bar_width.saturating_sub(filled)),
                ui.percent()
            );

            Row::new(vec![
                Cell::from(format!("{:02}", t.number)),
                Cell::from(status_text).style(status_style),
                Cell::from(bar),
                Cell::from(if ui.error_count > 0 {
                    ui.error_count.to_string()
                } else {
                    "-".to_string()
                }),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(6),
        Constraint::Length(10),
        Constraint::Length(30),
        Constraint::Length(8),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_style(false))
        .title(" Per-Track Progress ");

    let table = Table::new(rows, widths).header(header).block(block);
    frame.render_widget(table, area);
}

fn draw_done(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(7), Constraint::Min(4)])
        .split(area);

    let elapsed = app
        .rip_started_at
        .map(|t| t.elapsed().as_secs())
        .unwrap_or(0);

    let lines = vec![
        Line::from(Span::styled(
            "Rip session complete!",
            theme.highlight_style(),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Ripped:  ", theme.dim_style()),
            Span::styled(
                app.ripped_count.to_string(),
                theme.status_style(crate::tui::theme::StatusKind::Ok),
            ),
        ]),
        Line::from(vec![
            Span::styled("Failed:  ", theme.dim_style()),
            Span::styled(
                app.failed_count.to_string(),
                if app.failed_count > 0 {
                    theme.status_style(crate::tui::theme::StatusKind::Error)
                } else {
                    theme.dim_style()
                },
            ),
        ]),
        Line::from(vec![
            Span::styled("Time:    ", theme.dim_style()),
            Span::styled(format!("{}m {}s", elapsed / 60, elapsed % 60), theme.text_style()),
        ]),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_style(true))
        .title(" Summary ");

    let para = Paragraph::new(lines).block(block);
    frame.render_widget(para, rows[0]);

    draw_log_panel(frame, app, rows[1], " Full Log ");
}

fn draw_log_panel(frame: &mut Frame, app: &App, area: Rect, title: &str) {
    let theme = &app.theme;

    let visible_lines = area.height.saturating_sub(2) as usize;
    let start = app.log.len().saturating_sub(visible_lines);

    let items: Vec<ListItem> = app
        .log
        .iter()
        .skip(start)
        .map(|l| ListItem::new(Span::styled(l.clone(), theme.text_style())))
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_style(false))
        .title(title);

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;

    let keys: &[(&str, &str)] = match app.screen {
        Screen::DriveSelect => &[("↑↓", "navigate"), ("Enter", "select"), ("t", "theme"), ("q", "quit")],
        Screen::TrackList => &[
            ("↑↓", "navigate"),
            ("Space", "toggle"),
            ("a/n", "all/none"),
            ("f", "format"),
            ("Enter", "rip"),
            ("Esc", "back"),
            ("q", "quit"),
        ],
        Screen::Ripping => &[("Ctrl+C", "quit")],
        Screen::Done => &[("Enter", "back to tracks"), ("q", "quit")],
    };

    let mut spans = Vec::new();
    for (i, (key, label)) in keys.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("   "));
        }
        spans.push(Span::styled(format!(" {} ", key), theme.highlight_style().add_modifier(Modifier::REVERSED)));
        spans.push(Span::raw(format!(" {}", label)));
    }

    let line = Line::from(spans);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_style(false));

    let para = Paragraph::new(line).block(block);
    frame.render_widget(para, area);
}
