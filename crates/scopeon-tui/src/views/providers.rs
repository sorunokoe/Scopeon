/// Tab 5: Sources — status of configured data providers, shown as a card grid.
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::App;
use crate::text::truncate_with_ellipsis;
use crate::views::components::{empty_state_lines, themed_block};

const DETECTION_ONLY: &[&str] = &["cursor", "windsurf", "continue"];

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    if app.providers.is_empty() {
        let lines = empty_state_lines(
            app.theme,
            "◎",
            "No sources detected",
            "Install Claude Code, GitHub Copilot CLI, or another supported provider.",
            "r",
            "refresh",
        );
        let msg = Paragraph::new(lines).block(themed_block(app.theme, "Sources", false));
        f.render_widget(msg, area);
        return;
    }

    let active_count = app.providers.iter().filter(|p| p.is_active).count();
    let full_count = app
        .providers
        .iter()
        .filter(|p| p.is_active && !DETECTION_ONLY.contains(&p.id.as_str()))
        .count();
    let title = format!(
        " Sources — {}/{} detected  ({} with full data) ",
        active_count,
        app.providers.len(),
        full_count,
    );

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(app.theme.border_type())
        .title(title);
    let inner = outer.inner(area);
    f.render_widget(outer, area);

    // Lay out cards in a 2-column grid; each card is 5 lines tall.
    let card_h = 5u16;
    let providers = &app.providers;
    let cols = if inner.width >= 80 { 2usize } else { 1usize };
    let rows_needed = providers.len().div_ceil(cols);

    let row_constraints: Vec<Constraint> = (0..rows_needed)
        .map(|_| Constraint::Length(card_h))
        .collect();

    let row_areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints(row_constraints)
        .split(inner);

    for (row_idx, row_area) in row_areas.iter().enumerate() {
        let start = row_idx * cols;
        let end = (start + cols).min(providers.len());
        let cards_in_row = &providers[start..end];

        let col_constraints: Vec<Constraint> = cards_in_row
            .iter()
            .map(|_| Constraint::Percentage((100 / cols) as u16))
            .collect();

        let col_areas = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(col_constraints)
            .split(*row_area);

        for (col_idx, p) in cards_in_row.iter().enumerate() {
            draw_provider_card(f, app, p, col_areas[col_idx]);
        }
    }
}

fn draw_provider_card(
    f: &mut Frame,
    app: &App,
    p: &crate::app::ProviderStatus,
    area: Rect,
) {
    let is_detect_only = DETECTION_ONLY.contains(&p.id.as_str());

    let (status_icon, status_label, status_color) = if p.is_active && !is_detect_only {
        ("●", "active", app.theme.success_color())
    } else if p.is_active && is_detect_only {
        ("◐", "detect-only", app.theme.warning_color())
    } else {
        ("○", "inactive", app.theme.muted_color())
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

    let sessions_str = if p.session_count > 0 {
        format!("{} sessions", p.session_count)
    } else {
        "no sessions".to_string()
    };

    let turns_str = if p.turn_count > 0 {
        format!(" · {} turns", p.turn_count)
    } else {
        String::new()
    };

    let path_hint = truncate_with_ellipsis(&p.config_hint, (area.width.saturating_sub(4)) as usize);

    let lines = vec![
        // Name + status badge
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(&p.name, name_style),
            Span::styled("  ", Style::default()),
            Span::styled(
                format!("{} {}", status_icon, status_label),
                Style::default().fg(status_color),
            ),
        ]),
        // Session + turn counts
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(&sessions_str, Style::default().fg(app.theme.accent_color())),
            Span::styled(&turns_str, Style::default().fg(app.theme.muted_color())),
        ]),
        // Data path / config hint
        Line::from(Span::styled(
            format!("  {}", path_hint),
            Style::default().fg(app.theme.muted_color()),
        )),
    ];

    let border_color = if p.is_active && !is_detect_only {
        app.theme.success_color()
    } else if p.is_active {
        app.theme.warning_color()
    } else {
        app.theme.muted_color()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(app.theme.border_type())
        .border_style(Style::default().fg(border_color));

    f.render_widget(Paragraph::new(lines).block(block), area);
}
