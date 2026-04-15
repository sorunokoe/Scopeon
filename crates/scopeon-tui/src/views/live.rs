/// Tab 1: Live session view
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Gauge, Paragraph, Row, Table},
    Frame,
};
use scopeon_core::{SessionStats, Turn};

use crate::app::App;

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    let Some(stats) = &app.live_stats else {
        let msg =
            Paragraph::new("No session data found. Run Claude Code to start collecting data.")
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Live Session "),
                );
        f.render_widget(msg, area);
        return;
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),  // session info
            Constraint::Length(3),  // context bar
            Constraint::Length(10), // token breakdown
            Constraint::Min(8),     // turn table
        ])
        .split(area);

    draw_session_info(f, stats, chunks[0]);
    draw_context_bar(f, stats, chunks[1]);
    draw_token_breakdown(f, stats, chunks[2]);
    draw_turn_table(f, app, stats, chunks[3]);
}

fn draw_session_info(f: &mut Frame, stats: &SessionStats, area: Rect) {
    let session = stats.session.as_ref();
    let model = session.map(|s| s.model.as_str()).unwrap_or("unknown");
    let slug = session.map(|s| s.slug.as_str()).unwrap_or("—");
    let project = session.map(|s| s.project_name.as_str()).unwrap_or("—");
    let branch = session.map(|s| s.git_branch.as_str()).unwrap_or("—");

    let last_turn = stats.turns.last();
    let context_limit = scopeon_core::context_window_for_model(model);
    let last_used = last_turn
        .map(|t| t.input_tokens + t.cache_read_tokens)
        .unwrap_or(0);
    let pressure_pct = (last_used as f64 / context_limit as f64 * 100.0).min(100.0);
    let pressure_color = if pressure_pct >= 95.0 {
        Color::Red
    } else if pressure_pct >= 80.0 {
        Color::Yellow
    } else {
        Color::Green
    };

    let lines = vec![
        Line::from(vec![
            Span::styled("  Session: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                slug,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("   Model: ", Style::default().fg(Color::DarkGray)),
            Span::styled(model, Style::default().fg(Color::Green)),
        ]),
        Line::from(vec![
            Span::styled("  Project: ", Style::default().fg(Color::DarkGray)),
            Span::styled(project, Style::default().fg(Color::White)),
            Span::styled("   Branch: ", Style::default().fg(Color::DarkGray)),
            Span::styled(branch, Style::default().fg(Color::Yellow)),
        ]),
        Line::from(vec![
            Span::styled("  Turns: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                stats.total_turns.to_string(),
                Style::default().fg(Color::White),
            ),
            Span::styled("   Est. Cost: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("${:.4}", stats.estimated_cost_usd),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("   Cache Savings: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("${:.4}", stats.cache_savings_usd),
                Style::default().fg(Color::Green),
            ),
        ]),
        Line::from(vec![
            Span::styled("  ⬡ Context: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{:.0}%", pressure_pct),
                Style::default().fg(pressure_color),
            ),
            Span::styled("   Cache Hit: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{:.1}%", stats.cache_hit_rate * 100.0),
                Style::default().fg(if stats.cache_hit_rate > 0.5 {
                    Color::Green
                } else {
                    Color::Yellow
                }),
            ),
            Span::styled("   MCP: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                stats.total_mcp_calls.to_string(),
                Style::default().fg(Color::Yellow),
            ),
        ]),
    ];

    let para = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Live Session "),
    );
    f.render_widget(para, area);
}

fn draw_context_bar(f: &mut Frame, stats: &SessionStats, area: Rect) {
    let model = stats
        .session
        .as_ref()
        .map(|s| s.model.as_str())
        .unwrap_or("unknown");
    let context_limit = scopeon_core::context_window_for_model(model);
    let used = stats.total_input_tokens + stats.total_cache_read_tokens;
    let pct = (used as f64 / context_limit as f64 * 100.0).min(100.0) as u16;

    let color = match pct {
        0..=79 => Color::Green,
        80..=94 => Color::Yellow,
        _ => Color::Red,
    };

    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Context Window (cumulative) "),
        )
        .gauge_style(Style::default().fg(color))
        .percent(pct)
        .label(format!(
            "{} / {} tokens  ({}%)",
            fmt_tokens(used),
            fmt_tokens(context_limit),
            pct
        ));

    f.render_widget(gauge, area);
}

fn draw_token_breakdown(f: &mut Frame, stats: &SessionStats, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let max = [
        stats.total_input_tokens,
        stats.total_cache_read_tokens,
        stats.total_cache_write_tokens,
        stats.total_output_tokens,
        stats.total_thinking_tokens,
        stats.total_mcp_calls,
    ]
    .iter()
    .copied()
    .max()
    .unwrap_or(1)
    .max(1);

    let left_lines = vec![
        token_bar_line(
            "Prompt  ",
            stats.total_input_tokens,
            max,
            Color::Blue,
            stats.estimated_cost_usd,
        ),
        token_bar_line(
            "Cache↓  ",
            stats.total_cache_read_tokens,
            max,
            Color::Green,
            0.0,
        ),
        token_bar_line(
            "Cache↑  ",
            stats.total_cache_write_tokens,
            max,
            Color::LightGreen,
            0.0,
        ),
        token_bar_line("Output  ", stats.total_output_tokens, max, Color::Cyan, 0.0),
        token_bar_line(
            "Thinking",
            stats.total_thinking_tokens,
            max,
            Color::Magenta,
            0.0,
        ),
    ];

    let right_lines = vec![
        Line::from(vec![
            Span::styled("  MCP Calls: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                stats.total_mcp_calls.to_string(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "  Prompt Cache Hit Rate: ",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                format!("{:.1}%", stats.cache_hit_rate * 100.0),
                Style::default().fg(if stats.cache_hit_rate > 0.5 {
                    Color::Green
                } else {
                    Color::Yellow
                }),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  ↓ = cache hit (read, cheap)",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  ↑ = cache write (new entry)",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  Refresh: auto every 2s",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let left = Paragraph::new(left_lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Token Breakdown (session total) "),
    );
    let right =
        Paragraph::new(right_lines).block(Block::default().borders(Borders::ALL).title(" Stats "));

    f.render_widget(left, chunks[0]);
    f.render_widget(right, chunks[1]);
}

fn token_bar_line(label: &str, value: i64, max: i64, color: Color, _cost: f64) -> Line<'static> {
    let bar_width = 20usize;
    let filled = ((value as f64 / max as f64) * bar_width as f64) as usize;
    let bar: String = "█".repeat(filled) + &"░".repeat(bar_width - filled);
    let label = label.to_string();
    let value_str = fmt_tokens(value);
    Line::from(vec![
        Span::raw(format!("  {} ", label)),
        Span::styled(bar, Style::default().fg(color)),
        Span::raw(format!(" {}", value_str)),
    ])
}

fn draw_turn_table(f: &mut Frame, app: &App, stats: &SessionStats, area: Rect) {
    let scroll = app.turn_scroll;
    let turns = &stats.turns;

    let header = Row::new(vec![
        Cell::from("#").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Input").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Cache↓").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Cache↑").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Think").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Output").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("MCP").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("ms").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("Cost").style(Style::default().add_modifier(Modifier::BOLD)),
    ])
    .style(Style::default().fg(Color::Yellow));

    let rows: Vec<Row> = turns
        .iter()
        .rev() // most recent first
        .skip(scroll)
        .map(|t| turn_to_row(t))
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(7),
            Constraint::Length(8),
            Constraint::Length(4),
            Constraint::Length(7),
            Constraint::Length(9),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Recent Turns (newest first) [↑↓ to scroll] "),
    )
    .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    f.render_widget(table, area);
}

fn turn_to_row(t: &Turn) -> Row<'static> {
    let ms = t
        .duration_ms
        .map(|d| format!("{}ms", d))
        .unwrap_or("—".into());
    Row::new(vec![
        Cell::from(t.turn_index.to_string()),
        Cell::from(fmt_tokens(t.input_tokens)).style(Style::default().fg(Color::Blue)),
        Cell::from(fmt_tokens(t.cache_read_tokens)).style(Style::default().fg(Color::Green)),
        Cell::from(fmt_tokens(t.cache_write_tokens)).style(Style::default().fg(Color::LightGreen)),
        Cell::from(fmt_tokens(t.thinking_tokens)).style(Style::default().fg(Color::Magenta)),
        Cell::from(fmt_tokens(t.output_tokens)).style(Style::default().fg(Color::Cyan)),
        Cell::from(t.mcp_call_count.to_string()).style(Style::default().fg(Color::Yellow)),
        Cell::from(ms),
        Cell::from(format!("${:.4}", t.estimated_cost_usd))
            .style(Style::default().fg(Color::White)),
    ])
}

fn fmt_tokens(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}
