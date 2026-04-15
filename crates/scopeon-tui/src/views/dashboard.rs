//! Tab 1: Dashboard — "Right now" + "Today at a glance"
//!
//! Split layout: top KPI strip + left = live session, right = today summary.
//! Bottom strip: turn cost timeline.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph},
    Frame,
};

use crate::app::App;
use crate::theme::Theme;
use crate::views::components::{empty_state_lines, kpi_row, themed_block};
use chrono::Datelike;

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    // Layout: KPI strip (1 line) + main split + turn timeline strip (4 lines)
    let timeline_height = 4u16;
    let kpi_height = 1u16;

    // Compact mode: width < 80 — single column, no side-by-side split.
    let compact = area.width < 80;

    let v_constraints = if area.height > kpi_height + timeline_height + 6 {
        vec![
            Constraint::Length(kpi_height),
            Constraint::Min(0),
            Constraint::Length(timeline_height),
        ]
    } else if area.height > kpi_height + 6 {
        vec![Constraint::Length(kpi_height), Constraint::Min(0)]
    } else {
        vec![Constraint::Min(0)]
    };

    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints(v_constraints.clone())
        .split(area);

    let (kpi_area, main_area, timeline_area) = match v_constraints.len() {
        3 => (Some(v[0]), v[1], Some(v[2])),
        2 => (Some(v[0]), v[1], None),
        _ => (None, v[0], None),
    };

    if let Some(ka) = kpi_area {
        draw_kpi_strip(f, app, ka);
    }

    if compact {
        // Single-column: stack live pane + today pane vertically.
        // Give live session ~40% of height, today the rest.
        let live_h = (main_area.height * 2 / 5).max(8).min(main_area.height);
        let today_h = main_area.height.saturating_sub(live_h);
        let vc = if today_h > 4 {
            vec![Constraint::Length(live_h), Constraint::Min(0)]
        } else {
            vec![Constraint::Min(0)]
        };
        let cv = Layout::default()
            .direction(Direction::Vertical)
            .constraints(vc.clone())
            .split(main_area);
        if vc.len() == 2 {
            draw_live_pane(f, app, cv[0]);
            draw_today_pane(f, app, cv[1]);
        } else {
            draw_live_pane(f, app, cv[0]);
        }
    } else {
        // Standard: 40% left (live), 60% right (today)
        let h = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(main_area);
        draw_live_pane(f, app, h[0]);
        draw_today_pane(f, app, h[1]);
    }

    if let Some(ta) = timeline_area {
        draw_turn_timeline(f, app, ta);
    }
}

// ── KPI strip ─────────────────────────────────────────────────────────────────

fn draw_kpi_strip(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;

    // IS-1: Build a rotating natural-language narrative from waste signals.
    // Falls back to KPI chips when no actionable suggestions exist.
    let narratives = build_narrative_messages(app);
    if !narratives.is_empty() {
        let idx = app.narrative_idx % narratives.len();
        let msg = &narratives[idx];
        let color = if msg.starts_with('⚠') || msg.starts_with('🔴') {
            t.error_color()
        } else if msg.starts_with('⬡') {
            t.success_color()
        } else {
            t.text_secondary()
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    " ◈  ".to_string(),
                    Style::default()
                        .fg(t.accent_color())
                        .add_modifier(ratatui::style::Modifier::BOLD),
                ),
                Span::styled(msg.as_str(), Style::default().fg(color)),
            ])),
            area,
        );
        return;
    }

    // Fallback: classic KPI chip row.
    let turns_today = app
        .global_stats
        .as_ref()
        .and_then(|g| g.daily.last())
        .map(|d| d.turn_count)
        .unwrap_or(0);

    let cache_pct = app
        .live_stats
        .as_ref()
        .map(|s| s.cache_hit_rate * 100.0)
        .unwrap_or(0.0);

    let avg_cost = app
        .global_stats
        .as_ref()
        .and_then(|g| g.daily.last())
        .and_then(|d| {
            if d.turn_count > 0 {
                Some(d.estimated_cost_usd / d.turn_count as f64)
            } else {
                None
            }
        })
        .unwrap_or(0.0);

    let ctx_pct = app.budget.context_pressure_pct;
    let health = app.health_score;

    let chips = [
        ("Turns", format!("{}", turns_today), t.text_primary()),
        (
            "Cache",
            format!("{:.0}%", cache_pct),
            t.cache_color(cache_pct),
        ),
        ("Avg/Turn", format!("${:.3}", avg_cost), t.cost_color()),
        ("Ctx", format!("{:.0}%", ctx_pct), t.context_color(ctx_pct)),
        ("Health", format!("{:.0}", health), t.health_color(health)),
    ];

    let chip_refs: Vec<(&str, &str, Color)> =
        chips.iter().map(|(l, v, c)| (*l, v.as_str(), *c)).collect();

    let line = kpi_row(&chip_refs, *t);
    f.render_widget(Paragraph::new(line), area);
}

/// IS-1: Generate rotating natural-language insight messages from waste signals.
/// Returns a prioritized list: critical anomalies first, then tips, then summary.
fn build_narrative_messages(app: &App) -> Vec<String> {
    // Each message has a severity level: 2=critical, 1=warning, 0=info.
    // We sort highest severity first, then rotate only among equal-severity messages.
    let mut messages: Vec<(u8, String)> = Vec::new();

    // Cache bust anomaly — critical.
    if let Some(drop_pct) = app.budget.cache_bust_drop {
        messages.push((2, format!(
            "⚠  Cache efficiency dropped {:.0}% — possible MCP reorder or --resume bug. Consider /compact",
            drop_pct
        )));
    }

    // Context pressure — critical above 80%.
    let ctx_pct = app.budget.context_pressure_pct;
    if ctx_pct >= 95.0 {
        let turns_str = app
            .budget
            .predicted_turns_remaining
            .map(|t| format!(" (~{} turns left)", t))
            .unwrap_or_default();
        messages.push((
            2,
            format!(
                "⚠  Context {:.0}% full{} — run /compact immediately",
                ctx_pct, turns_str
            ),
        ));
    } else if ctx_pct >= 80.0 {
        let turns_str = app
            .budget
            .predicted_turns_remaining
            .map(|t| format!(" (~{} turns left)", t))
            .unwrap_or_default();
        messages.push((
            2,
            format!(
                "⚠  Context {:.0}% full{} — run /compact before next long task",
                ctx_pct, turns_str
            ),
        ));
    }

    // EOD projection warning — warning.
    if app.budget.daily_limit > 0.0 && app.budget.daily_projected_eod > 0.0 {
        let ratio = app.budget.daily_projected_eod / app.budget.daily_limit;
        if ratio > 0.85 {
            messages.push((
                1,
                format!(
                    "⚠  On pace for ${:.2} today (limit ${:.2}) — consider pausing high-cost tasks",
                    app.budget.daily_projected_eod, app.budget.daily_limit
                ),
            ));
        }
    }

    // Top waste suggestion — warning or info depending on severity.
    if let Some(s) = app.suggestions.first() {
        use scopeon_metrics::suggestions::Severity;
        let sev = match s.severity {
            Severity::Critical | Severity::Warning => 1,
            Severity::Info => 0,
        };
        messages.push((
            sev,
            format!(
                "💡  {}: {}",
                s.title,
                s.body
                    .split_whitespace()
                    .take(12)
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        ));
    }

    // Daily cost summary with trend — info.
    let daily = app.budget.daily_spent;
    if daily > 0.0 {
        let trend = if app.trend_cost_pct > 10.0 {
            format!(" ↑{:.0}% vs yesterday", app.trend_cost_pct)
        } else if app.trend_cost_pct < -10.0 {
            format!(" ↓{:.0}% vs yesterday", app.trend_cost_pct.abs())
        } else {
            String::new()
        };
        let cache_pct = app
            .live_stats
            .as_ref()
            .map(|s| s.cache_hit_rate * 100.0)
            .unwrap_or(0.0);
        messages.push((
            0,
            format!(
                "⬡  ${:.2} today{}  ·  Cache {:.0}%  ·  Health {:.0}/100",
                daily, trend, cache_pct, app.health_score
            ),
        ));
    }

    if messages.is_empty() {
        return Vec::new();
    }

    // Sort by severity descending (highest severity first, stable).
    messages.sort_by(|a, b| b.0.cmp(&a.0));

    // Pin the highest-severity message. If there are multiple messages at the same
    // top severity, rotate among them. Lower-severity messages only show when no
    // higher-severity message exists.
    let top_severity = messages[0].0;
    let top_group: Vec<String> = messages
        .iter()
        .filter(|(s, _)| *s == top_severity)
        .map(|(_, m)| m.clone())
        .collect();

    // Rotate within top group only (lower-severity messages are suppressed when critical/warning present).
    top_group
}

// ── Left pane: live session ────────────────────────────────────────────────────

fn draw_live_pane(f: &mut Frame, app: &App, area: Rect) {
    // Multi-agent banner: if multiple sessions are active, show them all first
    if app.active_sessions.len() > 1 {
        draw_multi_active_banner(f, app, area);
        return;
    }

    let Some(stats) = &app.live_stats else {
        let lines = empty_state_lines(
            app.theme,
            "◎",
            "Waiting for Claude Code…",
            "Run claude in any project to see live session data.",
            "r",
            "refresh",
        );
        let msg = Paragraph::new(lines).block(themed_block(app.theme, "◆ Session", false));
        f.render_widget(msg, area);
        return;
    };

    let session = stats.session.as_ref();
    let model = session.map(|s| s.model.as_str()).unwrap_or("—");
    let project = session.map(|s| s.project_name.as_str()).unwrap_or("—");
    let branch = session.map(|s| s.git_branch.as_str()).unwrap_or("—");

    // Staleness: how long since last turn?
    let now_ms = chrono::Utc::now().timestamp_millis();
    let idle_ms = session.map(|s| now_ms - s.last_turn_at).unwrap_or(i64::MAX);
    let idle_str = if idle_ms < 60_000 {
        "just now".to_string()
    } else if idle_ms < 3_600_000 {
        format!("{}m ago", idle_ms / 60_000)
    } else {
        format!("{}h ago", idle_ms / 3_600_000)
    };

    let (live_label, live_color, title) = if app.is_live {
        ("◉ LIVE", Color::Green, " ◆ Live Session ")
    } else if app.copilot_active {
        (
            "◉ Copilot active",
            Color::Yellow,
            " ◆ Session (Copilot — no token data) ",
        )
    } else {
        ("◎ IDLE", Color::DarkGray, " ◆ Last Session ")
    };

    // Context pressure
    let context_limit = 200_000i64;
    let last_turn = stats.turns.last();
    let last_used = last_turn
        .map(|t| t.input_tokens + t.cache_read_tokens)
        .unwrap_or(0);
    let pressure_pct = (last_used as f64 / context_limit as f64 * 100.0).min(100.0);
    let pressure_color = app.theme.context_color(pressure_pct);

    // Split into: session info block + context gauge + this-turn breakdown
    // At small heights, drop the turn breakdown to keep what matters visible.
    let v_constraints = if area.height >= 14 {
        vec![
            Constraint::Length(5), // session info
            Constraint::Length(3), // context gauge
            Constraint::Min(0),    // this-turn breakdown
        ]
    } else if area.height >= 9 {
        vec![
            Constraint::Length(5), // session info
            Constraint::Min(0),    // context gauge (no breakdown)
        ]
    } else {
        vec![Constraint::Min(0)] // just session info
    };

    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints(v_constraints.clone())
        .split(area);

    // Session info
    let model_short = shorten_model(model);
    let branch_str = if branch.is_empty() || branch == "—" {
        "—".to_string()
    } else {
        branch.to_string()
    };
    let info_lines = vec![
        Line::from(vec![
            Span::styled("  Model:   ", Style::default().fg(app.theme.muted_color())),
            Span::styled(
                model_short,
                Style::default()
                    .fg(app.theme.model_color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ", Style::default()),
            Span::styled(
                live_label,
                Style::default().fg(live_color).add_modifier(Modifier::BOLD),
            ),
            if !app.is_live {
                Span::styled(
                    format!("  {}", idle_str),
                    Style::default().fg(app.theme.muted_color()),
                )
            } else {
                Span::raw("")
            },
        ]),
        Line::from(vec![
            Span::styled("  Project: ", Style::default().fg(app.theme.muted_color())),
            Span::styled(project, Style::default().fg(app.theme.text_primary())),
        ]),
        Line::from(vec![
            Span::styled("  Branch:  ", Style::default().fg(app.theme.muted_color())),
            Span::styled(branch_str, Style::default().fg(app.theme.heading_color())),
        ]),
        Line::from(vec![
            Span::styled("  Turns:   ", Style::default().fg(app.theme.muted_color())),
            Span::styled(
                stats.total_turns.to_string(),
                Style::default().fg(app.theme.text_primary()),
            ),
            Span::styled("   MCP: ", Style::default().fg(app.theme.muted_color())),
            Span::styled(
                stats.total_mcp_calls.to_string(),
                Style::default().fg(app.theme.heading_color()),
            ),
        ]),
    ];
    let session_block = Block::default()
        .borders(Borders::ALL)
        .border_type(app.theme.border_type())
        .border_style(
            app.theme
                .crisis_border_style(app.budget.context_pressure_pct),
        )
        .title(title);
    f.render_widget(Paragraph::new(info_lines).block(session_block), v[0]);

    // Context gauge — only if there's a slot for it
    if v_constraints.len() >= 2 {
        // IS-10: Heartbeat — pulse_phase drives a breathing fill character on the leading edge.
        let heartbeat_char = if app.pulse_phase < 0.5 { "█" } else { "▉" };
        let gauge_label = format!(
            "{} / {}  {:.0}%  {}{}",
            fmt_k(last_used),
            fmt_k(context_limit),
            pressure_pct,
            if pressure_pct >= 80.0 {
                "⚠ high "
            } else if pressure_pct >= 60.0 {
                "↑ growing "
            } else {
                "✓ ok "
            },
            heartbeat_char,
        );
        let gauge_style = if app.is_live {
            Style::default().fg(pressure_color)
        } else {
            Style::default().fg(app.theme.muted_color())
        };
        let gauge = Gauge::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(app.theme.border_type())
                    .border_style(app.theme.inactive_border_style())
                    .title(" Context Window "),
            )
            .gauge_style(gauge_style)
            .percent(pressure_pct as u16)
            .label(gauge_label);
        f.render_widget(gauge, v[1]);
    }

    // Turn breakdown — only if there's space for a 3rd slot
    if v_constraints.len() >= 3 {
        draw_turn_breakdown(f, app, stats, v[2]);
    }
}

fn draw_multi_active_banner(f: &mut Frame, app: &App, area: Rect) {
    let mut lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!(
                "  ◉ {} concurrent active sessions",
                app.active_sessions.len()
            ),
            Style::default()
                .fg(app.theme.success_color())
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    for s in &app.active_sessions {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let idle_ms = now_ms - s.last_turn_at;
        let idle_str = if idle_ms < 60_000 {
            "just now".to_string()
        } else {
            format!("{}m ago", idle_ms / 60_000)
        };

        lines.push(Line::from(vec![
            Span::styled("  ◦ ", Style::default().fg(app.theme.accent_color())),
            Span::styled(
                s.project_name.clone(),
                Style::default()
                    .fg(app.theme.text_primary())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {} ", shorten_model(&s.model)),
                Style::default().fg(app.theme.model_color()),
            ),
            Span::styled(idle_str, Style::default().fg(app.theme.muted_color())),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  → Sessions tab to inspect each",
        Style::default().fg(app.theme.muted_color()),
    )));

    if app.copilot_active {
        lines.push(Line::from(vec![
            Span::styled("  ◉ ", Style::default().fg(app.theme.warning_color())),
            Span::styled(
                "Copilot also active (token data unavailable)",
                Style::default().fg(app.theme.muted_color()),
            ),
        ]));
    }

    f.render_widget(
        Paragraph::new(lines).block(themed_block(app.theme, "◆ Active Sessions", false)),
        area,
    );
}

fn draw_turn_breakdown(f: &mut Frame, app: &App, stats: &scopeon_core::SessionStats, area: Rect) {
    let turn = stats.turns.last();
    let scroll = app.turn_scroll;

    let mut lines: Vec<Line> = Vec::new();

    if let Some(t) = turn {
        let max_val = [
            t.input_tokens,
            t.cache_read_tokens,
            t.cache_write_tokens,
            t.thinking_tokens,
            t.output_tokens,
        ]
        .iter()
        .copied()
        .max()
        .unwrap_or(1)
        .max(1);

        lines.push(Line::from(Span::styled(
            format!("  Turn #{}", t.turn_index),
            Style::default().fg(app.theme.muted_color()),
        )));
        lines.push(token_bar(
            "⬢ Prompt ",
            t.input_tokens,
            max_val,
            Color::Rgb(100, 150, 255),
            app.theme,
        ));
        lines.push(token_bar(
            "↓ Cache  ",
            t.cache_read_tokens,
            max_val,
            app.theme.success_color(),
            app.theme,
        ));
        lines.push(token_bar(
            "↑ Write  ",
            t.cache_write_tokens,
            max_val,
            Color::Rgb(0, 200, 100),
            app.theme,
        ));
        lines.push(token_bar(
            "✦ Think  ",
            t.thinking_tokens,
            max_val,
            app.theme.cost_color(),
            app.theme,
        ));
        lines.push(token_bar(
            "▷ Output ",
            t.output_tokens,
            max_val,
            app.theme.accent_color(),
            app.theme,
        ));
        lines.push(Line::from(vec![
            Span::styled("  Cost: ", Style::default().fg(app.theme.muted_color())),
            Span::styled(
                format!("${:.4}", t.estimated_cost_usd),
                Style::default()
                    .fg(app.theme.cost_color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("   Saved: ", Style::default().fg(app.theme.muted_color())),
            Span::styled(
                format!("${:.4}", stats.cache_savings_usd),
                Style::default().fg(app.theme.success_color()),
            ),
        ]));
    } else {
        lines.push(Line::from(Span::styled(
            "  No turns yet.",
            Style::default().fg(app.theme.muted_color()),
        )));
    }

    // Session totals summary
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            "  Session total: ",
            Style::default().fg(app.theme.muted_color()),
        ),
        Span::styled(
            format!("${:.4}", stats.estimated_cost_usd),
            Style::default().fg(app.theme.cost_color()),
        ),
        Span::styled("  Cache: ", Style::default().fg(app.theme.muted_color())),
        Span::styled(
            format!("{:.1}%", stats.cache_hit_rate * 100.0),
            Style::default().fg(app.theme.cache_color(stats.cache_hit_rate * 100.0)),
        ),
    ]));

    // Past turns table (scrollable)
    lines.push(Line::from(Span::styled(
        "  ─ Past turns (newest first) ↑↓ scroll ─────",
        Style::default().fg(app.theme.muted_color()),
    )));
    for t in stats.turns.iter().rev().skip(scroll).take(6) {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  #{:<3}", t.turn_index),
                Style::default().fg(app.theme.muted_color()),
            ),
            Span::styled(
                fmt_k(t.input_tokens),
                Style::default().fg(Color::Rgb(100, 150, 255)),
            ),
            Span::styled(" ↓", Style::default().fg(app.theme.success_color())),
            Span::styled(
                fmt_k(t.cache_read_tokens),
                Style::default().fg(app.theme.success_color()),
            ),
            Span::styled(" ✦", Style::default().fg(app.theme.cost_color())),
            Span::styled(
                fmt_k(t.thinking_tokens),
                Style::default().fg(app.theme.cost_color()),
            ),
            Span::styled(" ▷", Style::default().fg(app.theme.accent_color())),
            Span::styled(
                fmt_k(t.output_tokens),
                Style::default().fg(app.theme.accent_color()),
            ),
            Span::styled(
                format!("  ${:.4}", t.estimated_cost_usd),
                Style::default().fg(Color::White),
            ),
        ]));
    }

    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" This Turn ")),
        area,
    );
}

// ── Right pane: today summary ─────────────────────────────────────────────────

fn draw_today_pane(f: &mut Frame, app: &App, area: Rect) {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8), // today stats + health
            Constraint::Min(0),    // sparkline + suggestions
        ])
        .split(area);

    draw_today_summary(f, app, v[0]);
    draw_sparkline_and_suggestions(f, app, v[1]);
}

fn draw_today_summary(f: &mut Frame, app: &App, area: Rect) {
    let global = app.global_stats.as_ref();

    let (today_cost, today_sessions, today_turns, today_mcp): (f64, i64, i64, i64) =
        if let Some(g) = global {
            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
            let r = g.daily.iter().find(|r| r.date == today);
            (
                r.map(|r| r.estimated_cost_usd).unwrap_or(0.0),
                r.map(|r| r.session_count).unwrap_or(0),
                r.map(|r| r.turn_count).unwrap_or(0),
                r.map(|r| r.total_mcp_calls).unwrap_or(0),
            )
        } else {
            (0.0, 0, 0, 0)
        };

    let cache_hit = app
        .live_stats
        .as_ref()
        .map(|s| s.cache_hit_rate * 100.0)
        .unwrap_or(0.0);
    let cache_saved = app
        .live_stats
        .as_ref()
        .map(|s| s.cache_savings_usd)
        .unwrap_or(0.0);

    // Trend arrows
    let cost_arrow = trend_arrow(-app.trend_cost_pct); // negative because lower cost = better
    let cache_arrow = trend_arrow(app.trend_cache_pct);

    let health = app.health_score;
    let health_color = app.theme.health_color(health);
    let health_bar = app.theme.progress_bar(health / 100.0, 20);

    // ── Progressive disclosure: lead with system state ───────────────────────
    // When healthy: show a ✓ confirmation so the user can dismiss the dashboard
    // at a glance. When degraded: surface the top issue immediately.
    let state_line = if health >= 80.0 {
        Line::from(vec![
            Span::styled("  ✓ ", Style::default().fg(health_color)),
            Span::styled(
                "All systems healthy",
                Style::default()
                    .fg(health_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ])
    } else if health >= 50.0 {
        Line::from(vec![
            Span::styled("  ↑ ", Style::default().fg(app.theme.warning_color())),
            Span::styled(
                "Optimisation opportunity — see Insights",
                Style::default().fg(app.theme.warning_color()),
            ),
        ])
    } else {
        let top_issue = app
            .suggestions
            .first()
            .map(|s| s.body.as_str())
            .unwrap_or("Check Insights tab for details");
        let short = if top_issue.len() > 34 {
            format!("{}…", &top_issue[..33])
        } else {
            top_issue.to_string()
        };
        Line::from(vec![
            Span::styled(
                "  ⚠ ",
                Style::default()
                    .fg(app.theme.error_color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(short, Style::default().fg(app.theme.error_color())),
        ])
    };

    let lines = vec![
        state_line,
        Line::from(vec![
            Span::styled("  Sessions: ", Style::default().fg(app.theme.muted_color())),
            Span::styled(
                today_sessions.to_string(),
                Style::default()
                    .fg(app.theme.text_primary())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  Turns: ", Style::default().fg(app.theme.muted_color())),
            Span::styled(
                today_turns.to_string(),
                Style::default().fg(app.theme.text_primary()),
            ),
            Span::styled("  MCP: ", Style::default().fg(app.theme.muted_color())),
            Span::styled(
                today_mcp.to_string(),
                Style::default().fg(app.theme.heading_color()),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Cost:  ", Style::default().fg(app.theme.muted_color())),
            Span::styled(
                format!("${:.3}", today_cost),
                Style::default()
                    .fg(app.theme.cost_color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {}  ", cost_arrow.0),
                Style::default().fg(cost_arrow.1),
            ),
            Span::styled(
                "Cache Saved: ",
                Style::default().fg(app.theme.muted_color()),
            ),
            Span::styled(
                format!("${:.3}", cache_saved),
                Style::default().fg(app.theme.success_color()),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Cache: ", Style::default().fg(app.theme.muted_color())),
            Span::styled(
                format!("{:.1}%", cache_hit),
                Style::default().fg(app.theme.cache_color(cache_hit)),
            ),
            Span::styled(
                format!(" {}", cache_arrow.0),
                Style::default().fg(cache_arrow.1),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Health  ", Style::default().fg(app.theme.muted_color())),
            Span::styled(health_bar, Style::default().fg(health_color)),
            Span::styled(
                format!(" {:.0}/100", health),
                Style::default()
                    .fg(health_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
    ];

    f.render_widget(
        Paragraph::new(lines).block(themed_block(app.theme, "Today", false)),
        area,
    );
}

fn draw_sparkline_and_suggestions(f: &mut Frame, app: &App, area: Rect) {
    let h = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    draw_cost_sparkline(f, app, h[0]);
    draw_top_suggestions(f, app, h[1]);
}

fn draw_cost_sparkline(f: &mut Frame, app: &App, area: Rect) {
    let global = app.global_stats.as_ref();
    let Some(g) = global else {
        f.render_widget(
            Paragraph::new("  No data yet.").block(themed_block(
                app.theme,
                "Cost trend (7d)",
                false,
            )),
            area,
        );
        return;
    };

    let data: Vec<(String, f64)> = g
        .daily
        .iter()
        .rev()
        .take(7)
        .rev()
        .map(|r| (r.date.clone(), r.estimated_cost_usd))
        .collect();

    if data.is_empty() {
        f.render_widget(
            Paragraph::new("  No daily data yet.").block(themed_block(
                app.theme,
                "Cost trend (7d)",
                false,
            )),
            area,
        );
        return;
    }

    let max_cost = data
        .iter()
        .map(|(_, c)| *c)
        .fold(0.0_f64, f64::max)
        .max(0.001);
    let avg = data.iter().map(|(_, c)| c).sum::<f64>() / data.len() as f64;
    let today = data.last().map(|(_, c)| *c).unwrap_or(0.0);

    // Dynamic bar width — fills the available area so there's no dead space.
    // inner_w = area width - 2 (border) - 2 (left pad)
    let inner_w = area.width.saturating_sub(4) as usize;
    let bar_w = (inner_w / data.len()).clamp(1, 12);

    let bar_spans: Vec<Span> = data
        .iter()
        .map(|(_, v)| {
            let ratio = v / max_cost;
            let filled = ((ratio * bar_w as f64) as usize).min(bar_w);
            let ch = match (ratio * 8.0) as u8 {
                0 => '▁',
                1 => '▂',
                2 => '▃',
                3 => '▄',
                4 => '▅',
                5 => '▆',
                6 => '▇',
                _ => '█',
            };
            Span::styled(
                ch.to_string().repeat(filled) + &"▁".repeat(bar_w - filled),
                Style::default().fg(app.theme.success_color()),
            )
        })
        .collect();

    let date_spans: Vec<Span> = data
        .iter()
        .map(|(date_str, _)| {
            let label = if let Ok(d) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                if bar_w >= 6 {
                    format!("{}", d.format("%b %d"))
                } else if bar_w >= 4 {
                    format!("{}/{:02}", d.month(), d.day())
                } else if bar_w >= 2 {
                    format!("{:02}", d.day())
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            Span::styled(
                format!("{:^width$}", label, width = bar_w),
                Style::default().fg(app.theme.muted_color()),
            )
        })
        .collect();

    let header = Line::from(vec![
        Span::styled("  max ", Style::default().fg(app.theme.muted_color())),
        Span::styled(
            format!("${:.2}", max_cost),
            Style::default().fg(app.theme.cost_color()),
        ),
        Span::styled("  avg ", Style::default().fg(app.theme.muted_color())),
        Span::styled(
            format!("${:.2}", avg),
            Style::default().fg(app.theme.cost_color()),
        ),
        Span::styled("  today ", Style::default().fg(app.theme.muted_color())),
        Span::styled(
            format!("${:.2}", today),
            Style::default()
                .fg(app.theme.cost_color())
                .add_modifier(Modifier::BOLD),
        ),
    ]);

    let mut bar_line = vec![Span::styled("  ", Style::default())];
    bar_line.extend(bar_spans);
    let mut date_line = vec![Span::styled("  ", Style::default())];
    date_line.extend(date_spans);

    f.render_widget(
        Paragraph::new(vec![header, Line::from(bar_line), Line::from(date_line)])
            .block(themed_block(app.theme, "Cost trend (7d)", false)),
        area,
    );
}

fn draw_top_suggestions(f: &mut Frame, app: &App, area: Rect) {
    // Body width: area width - borders(2) - left-pad(2) - icon+space(4)
    let body_w = area.width.saturating_sub(8) as usize;
    // Available text rows: border top+bottom (2) + blank spacer (1)
    let avail_h = (area.height as usize).saturating_sub(3);
    const CONT_INDENT: &str = "      ";

    let mut lines: Vec<Line> = vec![Line::from("")]; // blank spacer

    // IS-M: Cache bust alert — shown at top of insights if detected.
    if let Some(drop_pct) = app.budget.cache_bust_drop {
        lines.push(Line::from(vec![
            Span::styled("  ⚡ ", Style::default().fg(app.theme.error_color())),
            Span::styled(
                format!(
                    "Cache efficiency dropped {:.0}% — possible MCP tool reorder or --resume bug. \
                     Try /clear and start a fresh session.",
                    drop_pct
                ),
                Style::default().fg(app.theme.text_primary()),
            ),
        ]));
    }

    if app.suggestions.is_empty() && app.budget.cache_bust_drop.is_none() {
        lines.push(Line::from(Span::styled(
            "  ✓ No issues found",
            Style::default().fg(app.theme.success_color()),
        )));
    } else {
        'outer: for s in app.suggestions.iter().take(5) {
            let (icon, col) = match s.severity {
                scopeon_metrics::suggestions::Severity::Critical => ("⚡", app.theme.error_color()),
                scopeon_metrics::suggestions::Severity::Warning => {
                    ("⚠ ", app.theme.warning_color())
                },
                scopeon_metrics::suggestions::Severity::Info => ("ℹ ", app.theme.accent_color()),
            };
            let wrapped = dash_word_wrap(&s.body, body_w);
            for (i, part) in wrapped.iter().enumerate() {
                if lines.len() > avail_h {
                    break 'outer;
                }
                if i == 0 {
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {} ", icon), Style::default().fg(col)),
                        Span::styled(part.clone(), Style::default().fg(app.theme.text_primary())),
                    ]));
                } else {
                    lines.push(Line::from(vec![
                        Span::styled(CONT_INDENT.to_string(), Style::default()),
                        Span::styled(part.clone(), Style::default().fg(app.theme.text_primary())),
                    ]));
                }
            }
        }
    }
    f.render_widget(
        Paragraph::new(lines).block(themed_block(app.theme, "Insights", false)),
        area,
    );
}

/// Word-wrap `text` to at most `max_width` display characters per line.
fn dash_word_wrap(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![text.to_string()];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let word_len = word.chars().count();
        let cur_len = current.chars().count();
        if current.is_empty() {
            current.push_str(word);
        } else if cur_len + 1 + word_len <= max_width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(current);
            current = word.to_string();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

// ── Bottom strip: turn cost timeline ─────────────────────────────────────────

fn draw_turn_timeline(f: &mut Frame, app: &App, area: Rect) {
    let Some(stats) = &app.live_stats else {
        f.render_widget(
            Paragraph::new("  No session data.").block(themed_block(
                app.theme,
                "Turn Timeline",
                false,
            )),
            area,
        );
        return;
    };

    let inner_w = (area.width.saturating_sub(6)) as usize;
    let turns = &stats.turns;
    if turns.is_empty() {
        f.render_widget(
            Paragraph::new("  Waiting for first turn…").block(themed_block(
                app.theme,
                "Turn Timeline",
                false,
            )),
            area,
        );
        return;
    }

    let bar_chars = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
    let max_cost = turns
        .iter()
        .map(|t| t.estimated_cost_usd)
        .fold(0.0_f64, f64::max)
        .max(0.0001);
    let visible: Vec<_> = turns.iter().rev().take(inner_w).rev().collect();

    let mut bar_row = String::new();
    let mut idx_row = String::new();

    for (i, t) in visible.iter().enumerate() {
        let ratio = t.estimated_cost_usd / max_cost;
        let bar_idx = (ratio * 7.0).min(7.0) as usize;
        let bar_char = bar_chars[bar_idx];
        let col_width = 2usize;

        let is_last = i == visible.len() - 1;
        if is_last {
            bar_row.push('●');
        } else {
            bar_row.push_str(bar_char);
        }
        bar_row.push(' ');

        if t.turn_index % 5 == 0 || i == 0 {
            let label = format!("{:<width$}", t.turn_index, width = col_width);
            idx_row.push_str(&label);
        } else {
            idx_row.push_str("  ");
        }
    }

    let lines = vec![
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(bar_row, Style::default().fg(app.theme.success_color())),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(idx_row, Style::default().fg(app.theme.muted_color())),
        ]),
    ];

    let title = format!(" Turn Timeline ({} turns)  ● = current ", turns.len());
    f.render_widget(
        Paragraph::new(lines).block(themed_block(app.theme, &title, false)),
        area,
    );
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn token_bar(label: &str, value: i64, max: i64, color: Color, theme: Theme) -> Line<'static> {
    let bar_w = 14usize;
    let ratio = value as f64 / max as f64;
    let bar = theme.progress_bar(ratio, bar_w);
    let label = label.to_string();
    let val_str = fmt_k(value);
    Line::from(vec![
        Span::raw(format!("  {} ", label)),
        Span::styled(bar, Style::default().fg(color)),
        Span::raw(format!(" {}", val_str)),
    ])
}

fn fmt_k(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

pub fn health_color(score: f64) -> Color {
    if score >= 75.0 {
        Color::Green
    } else if score >= 50.0 {
        Color::Yellow
    } else {
        Color::Red
    }
}

fn shorten_model(model: &str) -> String {
    // claude-opus-4-5-20250514 → claude-opus-4.5
    if let Some(s) = model.strip_prefix("claude-") {
        let parts: Vec<&str> = s.split('-').collect();
        if parts.len() >= 2 {
            return format!("claude-{}-{}", parts[0], parts[1]);
        }
    }
    if model.len() > 22 {
        model[..22].to_string()
    } else {
        model.to_string()
    }
}

fn trend_arrow(pct: f64) -> (&'static str, Color) {
    if pct > 5.0 {
        ("▲", Color::Green)
    } else if pct > 0.5 {
        ("↑", Color::Green)
    } else if pct < -5.0 {
        ("▼", Color::Red)
    } else if pct < -0.5 {
        ("↓", Color::Red)
    } else {
        ("─", Color::DarkGray)
    }
}
