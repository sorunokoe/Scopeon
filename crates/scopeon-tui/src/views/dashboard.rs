//! Tab 1: Dashboard — "Mission Control"
//!
//! One-screen overview: hero (live session + context/cache) + 3 columns:
//!   Left   — Today's activity (stats + session mini-list)
//!   Center — Cost by source (provider → model breakdown)
//!   Right  — Recommendations (health score + top suggestions)
//! Bottom strip: turn cost timeline (when tall enough).

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Borders, Paragraph},
    Frame,
};

use crate::app::App;
use crate::text::{truncate_to_chars, truncate_with_ellipsis};
use crate::theme::Theme;
use crate::views::components::{themed_block, themed_block_borders};

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    let compact = area.width < 80;
    let hero_h = 2u16;
    let timeline_h = 4u16;

    let v_constraints = if area.height >= hero_h + 10 + timeline_h {
        vec![
            Constraint::Length(hero_h),
            Constraint::Min(0),
            Constraint::Length(timeline_h),
        ]
    } else if area.height >= hero_h + 6 {
        vec![Constraint::Length(hero_h), Constraint::Min(0)]
    } else {
        vec![Constraint::Min(0)]
    };

    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints(v_constraints.clone())
        .split(area);

    let idx_main = if v_constraints.len() > 1 { 1 } else { 0 };
    let idx_timeline = if v_constraints.len() == 3 {
        Some(2)
    } else {
        None
    };

    if v_constraints.len() > 1 {
        draw_mission_hero(f, app, v[0]);
    }

    if compact {
        // Narrow: left half = Today stacked over Cost, right half = Recommendations
        let h = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(v[idx_main]);
        let lv = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(h[0]);
        draw_today_activity_col(f, app, lv[0]);
        draw_spend_by_source_col(f, app, lv[1]);
        draw_recommendations_col(f, app, h[1]);
    } else {
        // Wide: 3-column grid with clean single-char separators
        let h = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(38),
                Constraint::Percentage(32),
                Constraint::Percentage(30),
            ])
            .split(v[idx_main]);
        draw_today_activity_col(f, app, h[0]);
        draw_spend_by_source_col(f, app, h[1]);
        draw_recommendations_col(f, app, h[2]);
    }

    if let Some(tl_i) = idx_timeline {
        draw_turn_timeline(f, app, v[tl_i]);
    }
}

// ── Hero: 2-line full-width live session summary ───────────────────────────────

fn draw_mission_hero(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;

    // Multi-agent: multiple concurrent active sessions
    if app.active_sessions.len() > 1 {
        let count = app.active_sessions.len();
        let names: String = app
            .active_sessions
            .iter()
            .take(3)
            .map(|s| {
                format!(
                    "{}/{}",
                    truncate_with_ellipsis(&s.project_name, 12),
                    shorten_model(&s.model)
                )
            })
            .collect::<Vec<_>>()
            .join("  ·  ");

        let line1 = Line::from(vec![
            Span::styled(
                format!(" ◉ {} CONCURRENT  ", count),
                Style::default()
                    .fg(t.success_color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(names, Style::default().fg(t.text_secondary())),
        ]);
        let line2 = build_ctx_cache_line(app);
        f.render_widget(Paragraph::new(vec![line1, line2]), area);
        return;
    }

    // Single session / IDLE / Copilot
    let live = app.live_stats.as_ref();
    let session = live
        .and_then(|s| s.session.as_ref())
        .or_else(|| app.sessions_list.first());

    let model_str = session.map(|s| shorten_model(&s.model)).unwrap_or_default();
    let provider_str = session.map(|s| s.provider.clone()).unwrap_or_default();
    let project_str = session
        .map(|s| truncate_with_ellipsis(&s.project_name, 20))
        .unwrap_or_default();
    let branch_str = session
        .map(|s| s.git_branch.clone())
        .filter(|b| !b.is_empty() && b != "—")
        .unwrap_or_default();

    let session_cost = live.map(|s| s.estimated_cost_usd).unwrap_or(0.0);
    let session_turns = live.map(|s| s.total_turns).unwrap_or(0i64);

    let (live_icon, live_label, live_color) = if app.is_live {
        ("◉", "LIVE", t.success_color())
    } else if app.copilot_active {
        ("◉", "COPILOT", t.warning_color())
    } else {
        ("◎", "IDLE", t.muted_color())
    };

    let line1 = if app.is_live || app.copilot_active {
        let mut spans = vec![
            Span::styled(
                format!(" {} ", live_icon),
                Style::default().fg(live_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                live_label.to_string(),
                Style::default().fg(live_color).add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
        ];
        if !model_str.is_empty() {
            spans.push(Span::styled(
                model_str.clone(),
                Style::default()
                    .fg(t.model_color())
                    .add_modifier(Modifier::BOLD),
            ));
        }
        if !provider_str.is_empty() {
            spans.push(Span::styled(
                format!("  {}", provider_str),
                Style::default().fg(t.muted_color()),
            ));
        }
        if !project_str.is_empty() {
            spans.push(Span::styled(
                format!("  {}", project_str),
                Style::default().fg(t.text_secondary()),
            ));
        }
        if !branch_str.is_empty() {
            spans.push(Span::styled(
                format!(" ⎇ {}", truncate_with_ellipsis(&branch_str, 20)),
                Style::default().fg(t.muted_color()),
            ));
        }
        if session_turns > 0 {
            spans.push(Span::styled(
                format!("   Turn #{}", session_turns),
                Style::default().fg(t.muted_color()),
            ));
        }
        if session_cost > 0.0 {
            spans.push(Span::styled(
                format!("  ${:.3}", session_cost),
                Style::default().fg(t.cost_color()),
            ));
        }
        if app.copilot_active && !app.is_live {
            spans.push(Span::styled(
                "  (no token data)",
                Style::default().fg(t.muted_color()),
            ));
        }
        Line::from(spans)
    } else {
        // IDLE: show today summary
        let today_str = chrono::Local::now().format("%Y-%m-%d").to_string();
        let today_row = app
            .global_stats
            .as_ref()
            .and_then(|g| g.daily.iter().find(|r| r.date == today_str));
        let today_cost = today_row.map(|r| r.estimated_cost_usd).unwrap_or(0.0);
        let today_sessions = today_row.map(|r| r.session_count).unwrap_or(0);
        let today_turns_count = today_row.map(|r| r.turn_count).unwrap_or(0);

        let mut spans = vec![
            Span::styled(format!(" {} ", live_icon), Style::default().fg(live_color)),
            Span::styled(live_label.to_string(), Style::default().fg(live_color)),
            Span::raw("  "),
        ];
        if !model_str.is_empty() {
            spans.push(Span::styled(
                model_str.clone(),
                Style::default().fg(t.text_secondary()),
            ));
            if !project_str.is_empty() {
                spans.push(Span::styled(
                    format!("  {}  ·  ", project_str),
                    Style::default().fg(t.muted_color()),
                ));
            } else {
                spans.push(Span::raw("  ·  "));
            }
        }
        spans.push(Span::styled(
            format!(
                "Today: {} sessions  {}t  ${:.2}",
                today_sessions, today_turns_count, today_cost
            ),
            Style::default().fg(t.text_secondary()),
        ));
        Line::from(spans)
    };

    let line2 = build_ctx_cache_line(app);
    f.render_widget(Paragraph::new(vec![line1, line2]), area);
}

/// Builds line 2 of the hero: context pressure bar + cache bar.
fn build_ctx_cache_line(app: &App) -> Line<'static> {
    let t = &app.theme;
    let ctx_pct = app.budget.context_pressure_pct;
    let ctx_color = t.context_color(ctx_pct);

    let cache_pct = app
        .live_stats
        .as_ref()
        .map(|s| s.cache_hit_rate * 100.0)
        .unwrap_or(0.0);
    let cache_savings = app
        .live_stats
        .as_ref()
        .map(|s| s.cache_savings_usd)
        .unwrap_or(0.0);

    let ctx_bar_w = 24usize;
    let ctx_filled = (ctx_pct / 100.0 * ctx_bar_w as f64) as usize;
    let ctx_bar = "█".repeat(ctx_filled) + &"░".repeat(ctx_bar_w - ctx_filled);

    let turns_str = app
        .budget
        .predicted_turns_remaining
        .map(|n| format!("  ~{}t", n))
        .unwrap_or_default();

    let urgency = if ctx_pct >= 95.0 {
        "  ⚠ /compact now"
    } else if ctx_pct >= 80.0 {
        "  ⚠ consider /compact"
    } else {
        ""
    };

    let cache_bar_w = 12usize;
    let cache_filled = (cache_pct / 100.0 * cache_bar_w as f64) as usize;
    let cache_bar = "█".repeat(cache_filled) + &"░".repeat(cache_bar_w - cache_filled);
    let cache_color = t.cache_color(cache_pct);

    let ctx_bold = if ctx_pct >= 80.0 {
        Modifier::BOLD
    } else {
        Modifier::empty()
    };

    let mut spans = vec![
        Span::styled("   Ctx ", Style::default().fg(t.muted_color())),
        Span::styled(ctx_bar, Style::default().fg(ctx_color)),
        Span::styled(
            format!(" {:.0}%{}", ctx_pct, turns_str),
            Style::default().fg(ctx_color).add_modifier(ctx_bold),
        ),
        Span::styled(
            urgency.to_string(),
            Style::default().fg(ctx_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled("    Cache ", Style::default().fg(t.muted_color())),
        Span::styled(cache_bar, Style::default().fg(cache_color)),
        Span::styled(
            format!(" {:.0}%", cache_pct),
            Style::default().fg(cache_color),
        ),
    ];

    if cache_savings > 0.001 {
        spans.push(Span::styled(
            format!("  Saved ${:.3}", cache_savings),
            Style::default().fg(t.success_color()),
        ));
    }

    Line::from(spans)
}

// ── Left column: Today's activity ────────────────────────────────────────────

fn draw_today_activity_col(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let today_str = chrono::Local::now().format("%Y-%m-%d").to_string();

    let today_row = app
        .global_stats
        .as_ref()
        .and_then(|g| g.daily.iter().find(|r| r.date == today_str));

    let today_cost = today_row.map(|r| r.estimated_cost_usd).unwrap_or(0.0);
    let today_turns = today_row.map(|r| r.turn_count).unwrap_or(0);

    let cache_pct = app
        .live_stats
        .as_ref()
        .map(|s| s.cache_hit_rate * 100.0)
        .unwrap_or(0.0);
    let cache_savings = app
        .live_stats
        .as_ref()
        .map(|s| s.cache_savings_usd)
        .unwrap_or(0.0);

    let (cost_arrow, cost_arrow_color) = trend_arrow(-app.trend_cost_pct, app.theme);

    let runway_pct = if app.budget.daily_limit > 0.0 {
        (app.budget.daily_spent / app.budget.daily_limit * 100.0).min(100.0)
    } else {
        0.0
    };
    let remaining = (app.budget.daily_limit - app.budget.daily_spent).max(0.0);

    // Sessions active today: those with a turn in today (use last_turn_at)
    let today_sessions: Vec<_> = app
        .sessions_list
        .iter()
        .filter(|s| {
            chrono::DateTime::from_timestamp_millis(s.last_turn_at)
                .map(|dt| {
                    dt.with_timezone(&chrono::Local)
                        .format("%Y-%m-%d")
                        .to_string()
                })
                .map(|d| d == today_str)
                .unwrap_or(false)
        })
        .take(6)
        .collect();

    let max_rows = area.height.saturating_sub(2) as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Headline: cost + trend + turns + cache
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            format!("${:.2}", today_cost),
            Style::default()
                .fg(t.cost_color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {}  ", cost_arrow),
            Style::default().fg(cost_arrow_color),
        ),
        Span::styled(
            format!("{}t  ", today_turns),
            Style::default().fg(t.text_secondary()),
        ),
        Span::styled(
            format!("Cache {:.0}%", cache_pct),
            Style::default().fg(t.cache_color(cache_pct)),
        ),
    ]));

    // Trend line
    if lines.len() < max_rows {
        let trend_str = if app.trend_cost_pct.abs() > 2.0 {
            format!(
                "{}{:.0}% vs yday",
                if app.trend_cost_pct > 0.0 {
                    "↑"
                } else {
                    "↓"
                },
                app.trend_cost_pct.abs()
            )
        } else {
            "≈ yesterday".to_string()
        };
        let savings_str = if cache_savings > 0.001 {
            format!("  Saved ${:.3}", cache_savings)
        } else {
            String::new()
        };
        lines.push(Line::from(vec![Span::styled(
            format!("  {}{}", trend_str, savings_str),
            Style::default().fg(t.muted_color()),
        )]));
    }

    // Budget bar (only if limit is set)
    if app.budget.daily_limit > 0.0 && lines.len() < max_rows {
        let bar_w = (area.width.saturating_sub(16) as usize).clamp(6, 18);
        let filled = (runway_pct / 100.0 * bar_w as f64) as usize;
        let bar = "█".repeat(filled) + &"░".repeat(bar_w - filled);
        let budget_color = if runway_pct >= 90.0 {
            t.error_color()
        } else if runway_pct >= 70.0 {
            t.warning_color()
        } else {
            t.success_color()
        };
        lines.push(Line::from(vec![
            Span::styled("  Budget ", Style::default().fg(t.muted_color())),
            Span::styled(bar, Style::default().fg(budget_color)),
            Span::styled(
                format!(" ${:.2} left", remaining),
                Style::default().fg(budget_color),
            ),
        ]));
    }

    // Blank spacer before session list
    if lines.len() < max_rows {
        lines.push(Line::raw(""));
    }

    // Session list header
    if lines.len() < max_rows {
        lines.push(Line::from(vec![Span::styled(
            format!("  Sessions today ({}):", today_sessions.len()),
            Style::default().fg(t.muted_color()),
        )]));
    }

    if today_sessions.is_empty() && lines.len() < max_rows {
        lines.push(Line::from(vec![Span::styled(
            "  No sessions today",
            Style::default().fg(t.muted_color()),
        )]));
    } else {
        for s in &today_sessions {
            if lines.len() >= max_rows {
                break;
            }
            let cost = app
                .session_summaries
                .get(&s.id)
                .map(|sum| sum.estimated_cost_usd)
                .unwrap_or(0.0);
            let time_ago = time_ago_ms(s.last_turn_at);
            let project_display = truncate_with_ellipsis(&s.project_name, 14);
            let is_active = app.is_live
                && app
                    .sessions_list
                    .first()
                    .map(|ls| ls.id == s.id)
                    .unwrap_or(false);
            let (icon, icon_color) = if is_active {
                ("◉", t.success_color())
            } else {
                ("◎", t.muted_color())
            };

            lines.push(Line::from(vec![
                Span::styled(format!("  {} ", icon), Style::default().fg(icon_color)),
                Span::styled(
                    project_display,
                    Style::default()
                        .fg(t.text_secondary())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {}", time_ago),
                    Style::default().fg(t.muted_color()),
                ),
                Span::styled(
                    format!("  ${:.2}", cost),
                    Style::default().fg(t.cost_color()),
                ),
                Span::styled(
                    format!("  {}t", s.total_turns),
                    Style::default().fg(t.muted_color()),
                ),
            ]));
        }
    }

    f.render_widget(
        Paragraph::new(lines).block(themed_block(app.theme, "Today", false)),
        area,
    );
}

// ── Center column: Cost by source (all-time by provider → model) ───────────────

fn draw_spend_by_source_col(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let data = &app.budget.cost_by_provider_model;
    let max_rows = area.height.saturating_sub(2) as usize;
    let bar_w = (area.width.saturating_sub(26) as usize).clamp(6, 16);

    if data.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  No spending data yet",
                Style::default().fg(t.muted_color()),
            )))
            .block(themed_block_borders(
                app.theme,
                "Cost by Source",
                false,
                Borders::TOP | Borders::RIGHT | Borders::BOTTOM,
            )),
            area,
        );
        return;
    }

    // Collect unique providers in order (data is already sorted provider-total-DESC)
    let mut seen_providers: Vec<&String> = Vec::new();
    for (prov, _, _) in data {
        if !seen_providers.contains(&prov) {
            seen_providers.push(prov);
        }
    }

    let grand_total: f64 = seen_providers
        .iter()
        .map(|prov| {
            data.iter()
                .filter(|(p, _, _)| p == *prov)
                .map(|(_, _, c)| c)
                .sum::<f64>()
        })
        .sum();

    let mut lines: Vec<Line<'static>> = Vec::new();

    for provider in &seen_providers {
        if lines.len() >= max_rows {
            break;
        }

        let prov_total: f64 = data
            .iter()
            .filter(|(p, _, _)| p == *provider)
            .map(|(_, _, c)| c)
            .sum();

        let prov_pct = if grand_total > 0.0 {
            prov_total / grand_total * 100.0
        } else {
            0.0
        };

        // Provider header
        lines.push(Line::from(vec![
            Span::styled("  ◉ ", Style::default().fg(t.accent_color())),
            Span::styled(
                truncate_with_ellipsis(provider, 14),
                Style::default()
                    .fg(t.text_primary())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  ${:.2}", prov_total),
                Style::default()
                    .fg(t.cost_color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {:.0}%", prov_pct),
                Style::default().fg(t.muted_color()),
            ),
        ]));

        // Model rows for this provider
        for (prov2, model, model_cost) in data {
            if prov2 != *provider {
                continue;
            }
            if lines.len() >= max_rows {
                break;
            }
            let model_pct = if prov_total > 0.0 {
                model_cost / prov_total * 100.0
            } else {
                0.0
            };
            let model_filled = (model_pct / 100.0 * bar_w as f64) as usize;
            let model_bar = "█".repeat(model_filled) + &"░".repeat(bar_w - model_filled);
            let model_short = shorten_model(model);
            let model_display = truncate_with_ellipsis(&model_short, 13);

            lines.push(Line::from(vec![
                Span::styled("    ▸ ", Style::default().fg(t.muted_color())),
                Span::styled(model_display, Style::default().fg(t.text_secondary())),
                Span::styled("  ", Style::default()),
                Span::styled(model_bar, Style::default().fg(t.cost_color())),
                Span::styled(
                    format!(" ${:.2}", model_cost),
                    Style::default().fg(t.muted_color()),
                ),
            ]));
        }

        // Blank spacer between providers
        if lines.len() < max_rows {
            lines.push(Line::raw(""));
        }
    }

    f.render_widget(
        Paragraph::new(lines).block(themed_block_borders(
            app.theme,
            "Cost by Source",
            false,
            Borders::TOP | Borders::RIGHT | Borders::BOTTOM,
        )),
        area,
    );
}

// ── Right column: Recommendations (health + suggestions) ─────────────────────

fn draw_recommendations_col(f: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let max_rows = area.height.saturating_sub(2) as usize;
    let body_w = (area.width.saturating_sub(8) as usize).max(8);

    let health = app.health_score;
    let health_color = t.health_color(health);
    let health_label = if health >= 90.0 {
        "Excellent"
    } else if health >= 75.0 {
        "Good"
    } else if health >= 50.0 {
        "Degraded"
    } else {
        "Critical"
    };

    let cache_pct = app
        .live_stats
        .as_ref()
        .map(|s| s.cache_hit_rate * 100.0)
        .unwrap_or(0.0);
    let cache_bar_w = (area.width.saturating_sub(16) as usize).clamp(6, 18);
    let cache_filled = (cache_pct / 100.0 * cache_bar_w as f64) as usize;
    let cache_bar = "█".repeat(cache_filled) + &"░".repeat(cache_bar_w - cache_filled);

    let mut lines: Vec<Line<'static>> = Vec::new();

    // Health score headline
    lines.push(Line::from(vec![
        Span::styled(
            format!("  ⬡ {:.0}  ", health),
            Style::default()
                .fg(health_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            health_label.to_string(),
            Style::default()
                .fg(health_color)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    // Cache bar
    if lines.len() < max_rows {
        lines.push(Line::from(vec![
            Span::styled("  Cache ", Style::default().fg(t.muted_color())),
            Span::styled(cache_bar, Style::default().fg(t.cache_color(cache_pct))),
            Span::styled(
                format!(" {:.0}%", cache_pct),
                Style::default().fg(t.cache_color(cache_pct)),
            ),
        ]));
    }

    // Cache bust alert (highest priority)
    if let Some(drop_pct) = app.budget.cache_bust_drop {
        if lines.len() < max_rows {
            lines.push(Line::raw(""));
        }
        if lines.len() < max_rows {
            lines.push(Line::from(vec![
                Span::styled(
                    "  ⚡ ",
                    Style::default()
                        .fg(t.error_color())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("Cache dropped {:.0}%", drop_pct),
                    Style::default()
                        .fg(t.error_color())
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        }
        if lines.len() < max_rows {
            lines.push(Line::from(vec![
                Span::styled("    → ", Style::default().fg(t.muted_color())),
                Span::styled("run /compact", Style::default().fg(t.accent_color())),
            ]));
        }
    }

    // Top suggestions (up to 3)
    let mut shown = 0usize;
    for s in app.suggestions.iter().take(5) {
        if shown >= 3 || lines.len() + 2 > max_rows {
            break;
        }
        let (icon, col) = match s.severity {
            scopeon_metrics::suggestions::Severity::Critical => ("⚡", t.error_color()),
            scopeon_metrics::suggestions::Severity::Warning => ("⚠ ", t.warning_color()),
            scopeon_metrics::suggestions::Severity::Info => ("ℹ ", t.accent_color()),
        };

        if lines.len() < max_rows {
            lines.push(Line::raw(""));
        }
        if lines.len() < max_rows {
            let title_display = truncate_with_ellipsis(s.title, body_w.saturating_sub(4));
            lines.push(Line::from(vec![
                Span::styled(format!("  {} ", icon), Style::default().fg(col)),
                Span::styled(title_display, Style::default().fg(t.text_primary())),
            ]));
        }
        // Short body preview (first 10 words)
        if lines.len() < max_rows {
            let short_body: String = s
                .body
                .split_whitespace()
                .take(10)
                .collect::<Vec<_>>()
                .join(" ");
            let short_body = truncate_with_ellipsis(&short_body, body_w.saturating_sub(4));
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(short_body, Style::default().fg(t.text_secondary())),
            ]));
        }
        // Action hint
        if let Some(cmd) = action_hint_for_id(s.id) {
            if lines.len() < max_rows {
                lines.push(Line::from(vec![
                    Span::styled("    → ", Style::default().fg(t.muted_color())),
                    Span::styled(cmd.to_string(), Style::default().fg(t.accent_color())),
                ]));
            }
        }
        shown += 1;
    }

    if shown == 0 && app.budget.cache_bust_drop.is_none() {
        if lines.len() < max_rows {
            lines.push(Line::raw(""));
        }
        if lines.len() < max_rows {
            lines.push(Line::from(vec![
                Span::styled("  ✓ ", Style::default().fg(t.success_color())),
                Span::styled(
                    "All systems healthy",
                    Style::default().fg(t.success_color()),
                ),
            ]));
        }
    }

    f.render_widget(
        Paragraph::new(lines).block(themed_block_borders(
            app.theme,
            "Recommendations",
            false,
            Borders::TOP | Borders::RIGHT | Borders::BOTTOM,
        )),
        area,
    );
}

// ── Action hint mapping (suggestion id → terminal command) ────────────────────

fn action_hint_for_id(id: &str) -> Option<&'static str> {
    match id {
        "cache-warmup" | "compaction-freq" | "cold-cache" | "below-avg-cache" => Some("/compact"),
        "thinking-ratio" => Some("disable extended thinking"),
        "redundant-tools" | "high-mcp" | "heavy-mcp-server" => Some("reduce MCP tools"),
        "task-fanout" | "task-prompt-bloat" => Some("use smaller focused tasks"),
        "high-cost-per-turn" | "above-avg-input" => Some("check context window size"),
        "skill-opportunity" | "conversation-phase" => Some("review in History tab"),
        "file-heavy-session" => Some("use --files flag selectively"),
        _ => None,
    }
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

/// Returns a time-ago string for a millisecond timestamp.
fn time_ago_ms(ts_ms: i64) -> String {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let delta = (now_ms - ts_ms).max(0);
    if delta < 60_000 {
        "just now".to_string()
    } else if delta < 3_600_000 {
        format!("{}m", delta / 60_000)
    } else if delta < 86_400_000 {
        format!("{}h", delta / 3_600_000)
    } else {
        format!("{}d", delta / 86_400_000)
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
    truncate_to_chars(model, 22)
}

fn trend_arrow(pct: f64, theme: Theme) -> (&'static str, Color) {
    if pct > 5.0 {
        ("▲", theme.success_color())
    } else if pct > 0.5 {
        ("↑", theme.success_color())
    } else if pct < -5.0 {
        ("▼", theme.error_color())
    } else if pct < -0.5 {
        ("↓", theme.error_color())
    } else {
        ("─", theme.muted_color())
    }
}
