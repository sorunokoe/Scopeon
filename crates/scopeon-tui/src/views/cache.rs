/// Tab 4: Cache Analysis
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph},
    Frame,
};

use crate::app::App;

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),  // live session cache
            Constraint::Length(4),  // hit rate gauge
            Constraint::Min(6),     // global cache analysis
        ])
        .split(area);

    draw_live_cache(f, app, chunks[0]);
    draw_hit_rate_gauge(f, app, chunks[1]);
    draw_global_cache(f, app, chunks[2]);
}

fn draw_live_cache(f: &mut Frame, app: &App, area: Rect) {
    let lines = if let Some(stats) = &app.live_stats {
        let _total_in = stats.total_input_tokens + stats.total_cache_read_tokens;
        let saved_usd = stats.cache_savings_usd;
        let effective_cost = stats.estimated_cost_usd;
        let hypothetical_no_cache = effective_cost + saved_usd;

        vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("  Cache Hits:  ", Style::default().fg(Color::DarkGray)),
                Span::styled(fmt(stats.total_cache_read_tokens), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::raw(" tokens  (served from cache at a discounted rate)"),
            ]),
            Line::from(vec![
                Span::styled("  Cache Writes: ", Style::default().fg(Color::DarkGray)),
                Span::styled(fmt(stats.total_cache_write_tokens), Style::default().fg(Color::LightGreen)),
                Span::raw(" tokens  (cost to create cache entries)"),
            ]),
            Line::from(vec![
                Span::styled("  Actual Cost:  ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("${:.5}", effective_cost), Style::default().fg(Color::Magenta)),
                Span::styled("   Without Cache: ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("${:.5}", hypothetical_no_cache), Style::default().fg(Color::Red)),
                Span::styled("   You Saved: ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("${:.5}", saved_usd), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            ]),
        ]
    } else {
        vec![Line::from("  No live session data.")]
    };

    let para = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Current Session — Cache Efficiency "));
    f.render_widget(para, area);
}

fn draw_hit_rate_gauge(f: &mut Frame, app: &App, area: Rect) {
    let hit_rate = app
        .live_stats
        .as_ref()
        .map(|s| s.cache_hit_rate)
        .unwrap_or(0.0);
    let pct = (hit_rate * 100.0) as u16;

    let color = match pct {
        0..=30 => Color::Red,
        31..=60 => Color::Yellow,
        _ => Color::Green,
    };

    let label = format!("Prompt Cache Hit Rate: {:.1}%  (higher = more tokens served from cache)", hit_rate * 100.0);

    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title(" Hit Rate "))
        .gauge_style(Style::default().fg(color))
        .percent(pct)
        .label(label);

    f.render_widget(gauge, area);
}

fn draw_global_cache(f: &mut Frame, app: &App, area: Rect) {
    let Some(global) = &app.global_stats else {
        let msg = Paragraph::new("No global data.")
            .block(Block::default().borders(Borders::ALL).title(" All-time Cache Analysis "));
        f.render_widget(msg, area);
        return;
    };

    // Use precomputed values from GlobalStats — computed with canonical formula
    // cache_hit_rate = cache_read / (input + cache_read + cache_write).
    let global_hit_rate = global.cache_hit_rate;
    // Use exact per-model savings already computed in get_global_stats().
    let savings_est = global.cache_savings_usd;

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  All-time Cache Hits:   ", Style::default().fg(Color::DarkGray)),
            Span::styled(fmt(global.total_cache_read_tokens), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::raw(" tokens"),
        ]),
        Line::from(vec![
            Span::styled("  All-time Cache Writes: ", Style::default().fg(Color::DarkGray)),
            Span::styled(fmt(global.total_cache_write_tokens), Style::default().fg(Color::LightGreen)),
            Span::raw(" tokens"),
        ]),
        Line::from(vec![
            Span::styled("  Overall Hit Rate:      ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{:.1}%", global_hit_rate * 100.0),
                Style::default().fg(if global_hit_rate > 0.5 { Color::Green } else { Color::Yellow })
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Est. Savings (per-model): ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("${:.4}", savings_est),
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  (read savings minus write overhead)", Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Tip: Use --context-cache or longer sessions to increase hit rate.",
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
        )),
    ];

    let para = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" All-time Cache Analysis "));
    f.render_widget(para, area);
}

fn fmt(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{:.2}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}
