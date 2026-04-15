/// Tab 2: Session history view
use ratatui::{
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Row, Table},
    Frame,
};

use crate::app::App;

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    let Some(global) = &app.global_stats else {
        let msg = ratatui::widgets::Paragraph::new("No data yet.")
            .block(Block::default().borders(Borders::ALL).title(" History "));
        f.render_widget(msg, area);
        return;
    };

    // Build rows from daily rollups (used as proxy for session list here)
    let header = Row::new(vec![
        Cell::from("Date").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Sessions").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Turns").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Input").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Cache↓").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Cache↑").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Output").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Think").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("MCP").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Cost").style(Style::default().add_modifier(Modifier::BOLD)),
    ])
    .style(Style::default().fg(Color::Yellow));

    let rows: Vec<Row> = global
        .daily
        .iter()
        .rev()
        .skip(app.history_scroll)
        .map(|r| {
            Row::new(vec![
                Cell::from(r.date.clone()),
                Cell::from(r.session_count.to_string()),
                Cell::from(r.turn_count.to_string()),
                Cell::from(fmt(r.total_input_tokens)).style(Style::default().fg(Color::Blue)),
                Cell::from(fmt(r.total_cache_read_tokens)).style(Style::default().fg(Color::Green)),
                Cell::from(fmt(r.total_cache_write_tokens))
                    .style(Style::default().fg(Color::LightGreen)),
                Cell::from(fmt(r.total_output_tokens)).style(Style::default().fg(Color::Cyan)),
                Cell::from(fmt(r.total_thinking_tokens)).style(Style::default().fg(Color::Magenta)),
                Cell::from(r.total_mcp_calls.to_string()).style(Style::default().fg(Color::Yellow)),
                Cell::from(format!("${:.4}", r.estimated_cost_usd)),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(12),
        Constraint::Length(9),
        Constraint::Length(6),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(7),
        Constraint::Length(5),
        Constraint::Length(10),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(format!(
            " History — {} sessions, ${:.4} total est. cost  [↑↓ scroll] ",
            global.total_sessions, global.estimated_cost_usd
        )))
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    f.render_widget(table, area);
}

fn fmt(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}
