/// Tab 5: Projects & Branch Attribution
use ratatui::{
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Frame,
};

use crate::app::App;

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    if app.project_stats.is_empty() {
        let msg = Paragraph::new("No project data yet. Run Claude Code to start collecting data.")
            .block(Block::default().borders(Borders::ALL).title(" Projects "));
        f.render_widget(msg, area);
        return;
    }

    let header = Row::new(vec![
        Cell::from("Project").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Branch").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Sessions").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Cost").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Cache%").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Turns").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Compactions").style(Style::default().add_modifier(Modifier::BOLD)),
    ])
    .style(Style::default().fg(Color::Yellow));

    let rows: Vec<Row> = app
        .project_stats
        .iter()
        .skip(app.projects_scroll)
        .map(|p| {
            let cache_color = if p.avg_cache_hit_rate >= 70.0 {
                Color::Green
            } else if p.avg_cache_hit_rate >= 40.0 {
                Color::Yellow
            } else {
                Color::Red
            };
            Row::new(vec![
                Cell::from(p.project_name.clone()),
                Cell::from(p.git_branch.clone()).style(Style::default().fg(Color::Yellow)),
                Cell::from(p.session_count.to_string()),
                Cell::from(format!("${:.2}", p.total_cost_usd))
                    .style(Style::default().fg(Color::Magenta)),
                Cell::from(format!("{:.1}%", p.avg_cache_hit_rate))
                    .style(Style::default().fg(cache_color)),
                Cell::from(p.total_turns.to_string()),
                Cell::from(p.compaction_count.to_string()).style(if p.compaction_count > 0 {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                }),
            ])
        })
        .collect();

    let widths = [
        Constraint::Min(16),
        Constraint::Min(14),
        Constraint::Length(9),
        Constraint::Length(10),
        Constraint::Length(8),
        Constraint::Length(7),
        Constraint::Length(12),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Projects & Branch Attribution  [sorted by Cost ↓]  [↑↓ scroll] "),
        )
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    f.render_widget(table, area);
}
