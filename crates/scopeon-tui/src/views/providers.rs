/// Tab 4: Providers — status of configured data providers
use ratatui::{
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Frame,
};

use crate::app::App;
use crate::views::components::themed_block;

const DETECTION_ONLY: &[&str] = &["cursor", "windsurf", "continue"];

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    if app.providers.is_empty() {
        let msg = Paragraph::new("No providers configured.").block(themed_block(
            app.theme,
            "Providers",
            false,
        ));
        f.render_widget(msg, area);
        return;
    }

    let hdr_style = Style::default()
        .add_modifier(Modifier::BOLD)
        .fg(app.theme.warning_color());
    let header = Row::new(vec![
        Cell::from("Status").style(hdr_style),
        Cell::from("Provider").style(hdr_style),
        Cell::from("Type").style(hdr_style),
        Cell::from("Sessions").style(hdr_style),
        Cell::from("Turns").style(hdr_style),
        Cell::from("Data Path / Config Hint").style(hdr_style),
    ]);

    let rows: Vec<Row> = app
        .providers
        .iter()
        .map(|p| {
            let is_detect_only = DETECTION_ONLY.contains(&p.id.as_str());

            let (status_text, status_style) = if p.is_active && !is_detect_only {
                (
                    "● active",
                    Style::default()
                        .fg(app.theme.success_color())
                        .add_modifier(Modifier::BOLD),
                )
            } else if p.is_active && is_detect_only {
                ("◐ detected", Style::default().fg(app.theme.warning_color()))
            } else {
                ("○ inactive", Style::default().fg(app.theme.muted_color()))
            };

            let type_label = if is_detect_only {
                Span::styled(
                    "detect-only",
                    Style::default().fg(app.theme.warning_color()),
                )
            } else {
                Span::styled("full data", Style::default().fg(app.theme.success_color()))
            };

            let sessions = if p.session_count > 0 {
                p.session_count.to_string()
            } else {
                "—".to_string()
            };

            let turns = if p.turn_count > 0 {
                p.turn_count.to_string()
            } else {
                "—".to_string()
            };

            let name_style = if p.is_active && !is_detect_only {
                Style::default()
                    .fg(app.theme.text_primary())
                    .add_modifier(Modifier::BOLD)
            } else if p.is_active {
                Style::default().fg(app.theme.warning_color())
            } else {
                Style::default().fg(app.theme.muted_color())
            };

            Row::new(vec![
                Cell::from(status_text).style(status_style),
                Cell::from(p.name.as_str()).style(name_style),
                Cell::from(type_label),
                Cell::from(sessions).style(Style::default().fg(app.theme.accent_color())),
                Cell::from(turns).style(Style::default().fg(app.theme.accent_color())),
                Cell::from(Span::styled(
                    p.config_hint.as_str(),
                    Style::default().fg(app.theme.muted_color()),
                )),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(12),
        Constraint::Length(18),
        Constraint::Length(13),
        Constraint::Length(10),
        Constraint::Length(8),
        Constraint::Min(30),
    ];

    let active_count = app.providers.iter().filter(|p| p.is_active).count();
    let full_count = app
        .providers
        .iter()
        .filter(|p| p.is_active && !DETECTION_ONLY.contains(&p.id.as_str()))
        .count();
    let title = format!(
        " Providers — {}/{} detected  ({} with full data) ",
        active_count,
        app.providers.len(),
        full_count,
    );

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(app.theme.border_type())
                .title(title),
        )
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    f.render_widget(table, area);
}
