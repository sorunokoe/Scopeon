/// Tab 3: Distribution — daily token bar chart + model/cost trend
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Bar, BarChart, BarGroup, Block, Borders, Paragraph},
    Frame,
};

use crate::app::App;

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    let Some(global) = &app.global_stats else {
        let msg = Paragraph::new("No data yet.")
            .block(Block::default().borders(Borders::ALL).title(" Distribution "));
        f.render_widget(msg, area);
        return;
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    // ── Top: input + output bar chart ────────────────────────────────────────
    let daily = &global.daily;
    let skip = app.history_scroll;
    let visible: Vec<_> = daily.iter().rev().skip(skip).take(30).collect();
    let visible: Vec<_> = visible.into_iter().rev().collect();

    let _groups: Vec<BarGroup> = visible
        .iter()
        .map(|r| {
            let date_label = r.date.trim_start_matches("20").to_string();
            BarGroup::default()
                .label(Line::from(date_label))
                .bars(&[
                    Bar::default()
                        .value(r.total_input_tokens as u64)
                        .style(Style::default().fg(Color::Blue))
                        .label(Line::from("")),
                    Bar::default()
                        .value(r.total_cache_read_tokens as u64)
                        .style(Style::default().fg(Color::Green))
                        .label(Line::from("")),
                    Bar::default()
                        .value(r.total_output_tokens as u64)
                        .style(Style::default().fg(Color::Cyan))
                        .label(Line::from("")),
                ])
        })
        .collect();

    let _bar_chart = BarChart::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Daily Tokens [Blue=Input  Green=Cache↓  Cyan=Output]  [↑↓ scroll] "),
        )
        .data(BarGroup::default())
        .bar_width(3)
        .bar_gap(1)
        .group_gap(2)
        .max(
            daily
                .iter()
                .map(|r| r.total_input_tokens + r.total_cache_read_tokens + r.total_output_tokens)
                .max()
                .unwrap_or(1) as u64,
        );

    // Render each group individually as ratatui BarChart supports single group
    // We'll use a simpler approach: combine into a single BarChart with all days
    let bars: Vec<Bar> = visible
        .iter()
        .flat_map(|r| {
            let _label = r.date.trim_start_matches("20").to_string();
            vec![
                Bar::default()
                    .value(r.total_input_tokens as u64)
                    .style(Style::default().fg(Color::Blue)),
                Bar::default()
                    .value(r.total_cache_read_tokens as u64)
                    .style(Style::default().fg(Color::Green)),
                Bar::default()
                    .value(r.total_output_tokens as u64)
                    .style(Style::default().fg(Color::Cyan)),
            ]
        })
        .collect();

    let barchart = BarChart::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Daily Tokens  Blue=Prompt  Green=Cache↓  Cyan=Output  [↑↓ scroll] "),
        )
        .data(BarGroup::default().bars(&bars))
        .bar_width(2)
        .bar_gap(1)
        .group_gap(1);

    f.render_widget(barchart, chunks[0]);

    // ── Bottom: summary stats ────────────────────────────────────────────────
    let total_context = global.total_input_tokens + global.total_cache_read_tokens;
    let hit_rate_pct = if total_context > 0 {
        global.total_cache_read_tokens as f64 / total_context as f64 * 100.0
    } else {
        0.0
    };

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  All-time Totals", Style::default().add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Sessions: ", Style::default().fg(Color::DarkGray)),
            Span::raw(global.total_sessions.to_string()),
            Span::styled("   Turns: ", Style::default().fg(Color::DarkGray)),
            Span::raw(global.total_turns.to_string()),
        ]),
        Line::from(vec![
            Span::styled("  Total Input: ", Style::default().fg(Color::DarkGray)),
            Span::styled(fmt(global.total_input_tokens), Style::default().fg(Color::Blue)),
            Span::styled("   Cache Hits: ", Style::default().fg(Color::DarkGray)),
            Span::styled(fmt(global.total_cache_read_tokens), Style::default().fg(Color::Green)),
            Span::styled(format!("  ({:.1}%)", hit_rate_pct), Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(vec![
            Span::styled("  Total Output: ", Style::default().fg(Color::DarkGray)),
            Span::styled(fmt(global.total_output_tokens), Style::default().fg(Color::Cyan)),
            Span::styled("   Thinking: ", Style::default().fg(Color::DarkGray)),
            Span::styled(fmt(global.total_thinking_tokens), Style::default().fg(Color::Magenta)),
        ]),
        Line::from(vec![
            Span::styled("  Est. Total Cost: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("${:.4}", global.estimated_cost_usd),
                Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
            ),
            Span::styled("   MCP Calls: ", Style::default().fg(Color::DarkGray)),
            Span::styled(global.total_mcp_calls.to_string(), Style::default().fg(Color::Yellow)),
        ]),
    ];

    let para = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" All-time Summary "));
    f.render_widget(para, chunks[1]);
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
