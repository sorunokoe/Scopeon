use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Frame,
};

use crate::app::{App, Tab};
use crate::logo::{logo_badge, logo_lines};
use crate::text::{truncate_to_chars, truncate_with_ellipsis};
use crate::theme::Theme;
use crate::views::components::{micro_sparkline, spinner_char};
use crate::views::{agents, budget, dashboard, insights, providers, sessions};

/// Terminal size tier — drives layout and density choices.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SizeClass {
    /// ≥ 80w × 22h — full tab-based layout with all panels.
    Standard,
    /// 55–79w or 14–21h — single-column layouts, abbreviated tab bar.
    Compact,
    /// 24–54w or 5–13h — no chrome, just the KPI widget panel.
    Widget,
    /// < 24w or < 5h — too small for anything useful.
    TooSmall,
}

fn size_class(area: Rect) -> SizeClass {
    let (w, h) = (area.width, area.height);
    if w < 24 || h < 5 {
        SizeClass::TooSmall
    } else if w < 55 || h < 14 {
        SizeClass::Widget
    } else if w < 80 || h < 22 {
        SizeClass::Compact
    } else {
        SizeClass::Standard
    }
}

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();

    // Always wipe first — prevents ghosting on resize / alt-screen transitions.
    f.render_widget(ratatui::widgets::Clear, area);

    match size_class(area) {
        SizeClass::TooSmall => {
            draw_too_small(f, area);
            return;
        },
        SizeClass::Widget => {
            draw_widget_panel(f, app, area);
            return;
        },
        _ => {},
    }

    // IS-4: Zen Mode — render a single ambient status line instead of the full TUI.
    if app.zen_mode {
        draw_zen_mode(f, app, area);
        return;
    }

    // Compact and Standard: tab-based layout.
    let sc = size_class(area);
    let banner_height = if app.alert_banner.is_some() {
        1u16
    } else {
        0u16
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),             // tab bar
            Constraint::Length(banner_height), // alert banner (0 when hidden)
            Constraint::Min(0),                // content
            Constraint::Length(1),             // status bar
        ])
        .split(area);

    draw_tab_bar(f, app, chunks[0], sc);

    if let Some((msg, color)) = &app.alert_banner {
        let ctx_pct = app.budget.context_pressure_pct;
        let banner_modifier = if app.theme == Theme::Cockpit && ctx_pct >= 95.0 {
            Modifier::BOLD | Modifier::SLOW_BLINK
        } else {
            Modifier::BOLD
        };
        let banner = Paragraph::new(Line::from(vec![Span::styled(
            format!("  ⚠  {} ", msg),
            Style::default().fg(*color).add_modifier(banner_modifier),
        )]))
        .style(Style::default().bg(Color::Reset));
        f.render_widget(banner, chunks[1]);
    }

    match app.tab {
        Tab::Dashboard => dashboard::draw(f, app, chunks[2]),
        Tab::Sessions => sessions::draw(f, app, chunks[2]),
        Tab::Insights => insights::draw(f, app, chunks[2]),
        Tab::Budget => budget::draw(f, app, chunks[2]),
        Tab::Providers => providers::draw(f, app, chunks[2]),
        Tab::Agents => agents::draw(f, app, chunks[2]),
    }

    draw_status_bar(f, app, chunks[3], sc);

    if app.show_help {
        draw_help_overlay(f, app, area);
    }
}

// ── IS-4: Zen Mode ────────────────────────────────────────────────────────────

/// Renders a single full-width ambient status line when zen mode is active.
/// Shows only the most essential metrics. Displays across all terminal rows
/// so the terminal background doesn't show through.
fn draw_zen_mode(f: &mut Frame, app: &App, area: Rect) {
    let ctx_pct = app.budget.context_pressure_pct;
    let ctx_color = app.theme.context_color(ctx_pct);
    let health_color = app.theme.health_color(app.health_score);
    let muted = app.theme.muted_color();

    let cache_pct = app
        .live_stats
        .as_ref()
        .map(|s| s.cache_hit_rate * 100.0)
        .unwrap_or(0.0);

    let turns_left = app
        .budget
        .predicted_turns_remaining
        .map(|t| format!(" ~{}t", t))
        .unwrap_or_default();

    let top_hint = app
        .suggestions
        .first()
        .map(|s| format!("  ·  {}", truncate_str(&s.body, 50)))
        .unwrap_or_default();

    let spans = vec![
        Span::styled(
            " ◈ SCOPEON ZEN ".to_string(),
            Style::default()
                .fg(app.theme.accent_color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("│", Style::default().fg(muted)),
        Span::styled(
            format!(" ⬡{:.0} ", app.health_score),
            Style::default()
                .fg(health_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("│", Style::default().fg(muted)),
        Span::styled(
            format!(" Ctx {:.0}%{} ", ctx_pct, turns_left),
            Style::default().fg(ctx_color),
        ),
        Span::styled("│", Style::default().fg(muted)),
        Span::styled(
            format!(" ${:.2}/day ", app.budget.daily_spent),
            Style::default().fg(app.theme.cost_color()),
        ),
        Span::styled("│", Style::default().fg(muted)),
        Span::styled(
            format!(" Cache {:.0}% ", cache_pct),
            Style::default().fg(app.theme.cache_color(cache_pct)),
        ),
        Span::styled(top_hint, Style::default().fg(muted)),
        Span::styled("  [z=expand  q=quit]", Style::default().fg(muted)),
    ];

    // Paint all rows blank first so nothing bleeds through.
    f.render_widget(
        Block::default().style(Style::default().bg(Color::Reset)),
        area,
    );
    // Render the single status line at vertical center.
    let y = area.height / 2;
    let line_area = Rect {
        x: area.x,
        y: area.y + y,
        width: area.width,
        height: 1,
    };
    f.render_widget(Paragraph::new(Line::from(spans)), line_area);
}

// ── Splash screen ─────────────────────────────────────────────────────────────

/// Rendered once at startup (~300ms) while the initial DB refresh runs.
pub fn draw_splash(f: &mut Frame, theme: Theme) {
    let area = f.area();

    // Paint the full frame with an explicit background block first.
    // This forces ratatui to write every cell, so no residual alt-screen
    // content from a previous run can bleed through.
    f.render_widget(
        Block::default().style(Style::default().bg(Color::Black)),
        area,
    );

    let logo_h = 5u16;
    let content_h = logo_h + 4; // logo + tagline + loading line
    let y = area.height.saturating_sub(content_h) / 2;

    let logo = logo_lines(theme);
    let mut lines: Vec<Line> = logo;
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("     v", Style::default().fg(theme.muted_color())),
        Span::styled(
            env!("CARGO_PKG_VERSION"),
            Style::default().fg(theme.muted_color()),
        ),
        Span::styled("  ·  Loading…", Style::default().fg(theme.muted_color())),
    ]));

    let content_area = Rect {
        x: (area.width.saturating_sub(36)) / 2,
        y,
        width: 36u16.min(area.width),
        height: content_h,
    };

    f.render_widget(Paragraph::new(lines), content_area);
}

// ── Terminal too small ────────────────────────────────────────────────────────

fn draw_too_small(f: &mut Frame, area: Rect) {
    f.render_widget(ratatui::widgets::Clear, area);
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  ◈  Terminal too small",
            Style::default()
                .fg(Color::Rgb(255, 196, 0))
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  Scopeon needs at least 24 × 5 characters.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  Resize or zoom out your terminal.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Current: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}×{}", area.width, area.height),
                Style::default()
                    .fg(Color::Rgb(255, 59, 48))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("   Required: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "24×5",
                Style::default()
                    .fg(Color::Rgb(0, 230, 118))
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
    ];
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(255, 196, 0))),
        ),
        area,
    );
}

// ── Widget panel — compact KPI view for small / "side widget" terminals ────────

/// Renders when the terminal is 24–54 columns wide or 5–13 rows tall.
/// Shows the most critical real-time metrics with no tab/status bar chrome.
/// Every line is added only if there is vertical space for it.
fn draw_widget_panel(f: &mut Frame, app: &App, area: Rect) {
    // Pre-extract all data to avoid borrow conflicts across blocks.
    let (badge, badge_color) = if app.is_live {
        ("◉ LIVE", app.theme.success_color())
    } else if app.copilot_active {
        ("◉ Copilot", app.theme.warning_color())
    } else {
        ("◎ IDLE", app.theme.muted_color())
    };

    let model = app
        .live_stats
        .as_ref()
        .and_then(|s| s.session.as_ref())
        .map(|s| shorten_model(&s.model))
        .unwrap_or_else(|| "—".to_string());

    let cost = app.budget.daily_spent;
    let ctx_pct = app.budget.context_pressure_pct;
    let daily_limit = app.budget.daily_limit;

    let cache_pct = app
        .live_stats
        .as_ref()
        .map(|s| s.cache_hit_rate * 100.0)
        .unwrap_or(0.0);

    let tokens_k = app
        .global_stats
        .as_ref()
        .and_then(|g| g.daily.last())
        .map(|d| {
            (d.total_input_tokens + d.total_cache_read_tokens + d.total_output_tokens) as f64
                / 1000.0
        })
        .unwrap_or(0.0);

    let turns_today = app
        .global_stats
        .as_ref()
        .and_then(|g| g.daily.last())
        .map(|d| d.turn_count)
        .unwrap_or(0);

    let health = app.health_score;

    // Border urgency: red when context > 80%, green when live, else muted.
    let border_color = if ctx_pct > 80.0 {
        app.theme.error_color()
    } else if ctx_pct > 60.0 {
        app.theme.warning_color()
    } else if app.is_live {
        app.theme.success_color()
    } else {
        app.theme.muted_color()
    };

    let block = Block::default()
        .title(Span::styled(
            " ◆ scopeon ",
            Style::default()
                .fg(app.theme.accent_color())
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(app.theme.border_type())
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let iw = inner.width as usize;
    let ih = inner.height;
    // Progress bar fills: total width - label (6) - " XX%" (5) = iw - 11
    let bar_w = (iw as u16).saturating_sub(11).max(4) as usize;

    let muted = app.theme.muted_color();
    let mut lines: Vec<Line> = Vec::new();

    // ── Line 1: status badge + model ──────────────────────────────────────────
    let model_max = iw.saturating_sub(badge.len() + 3);
    let model_disp = if model.len() > model_max {
        format!("{}…", &model[..model_max.saturating_sub(1)])
    } else {
        model.clone()
    };
    lines.push(Line::from(vec![
        Span::styled(
            format!(" {}", badge),
            Style::default()
                .fg(badge_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {}", model_disp),
            Style::default().fg(app.theme.text_primary()),
        ),
    ]));

    if ih <= 1 {
        f.render_widget(Paragraph::new(lines), inner);
        return;
    }

    // ── Line 2: cost · tokens · cache ────────────────────────────────────────
    let cache_color = app.theme.cache_color(cache_pct);
    if iw >= 32 {
        lines.push(Line::from(vec![
            Span::styled(
                format!(" ${:.2}", cost),
                Style::default()
                    .fg(app.theme.cost_color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ·  ", Style::default().fg(muted)),
            Span::styled(
                format!("{:.0}K tok", tokens_k),
                Style::default().fg(app.theme.text_primary()),
            ),
            Span::styled("  ·  ", Style::default().fg(muted)),
            Span::styled(
                format!("Cache {:.0}%", cache_pct),
                Style::default().fg(cache_color),
            ),
        ]));
    } else {
        lines.push(Line::from(Span::styled(
            format!(" ${:.2}  {:.0}K  {:.0}%", cost, tokens_k, cache_pct),
            Style::default().fg(app.theme.cost_color()),
        )));
    }

    if ih <= 2 {
        f.render_widget(Paragraph::new(lines), inner);
        return;
    }

    // ── Line 3: context pressure bar ─────────────────────────────────────────
    let ctx_color = app.theme.context_color(ctx_pct);
    let ctx_bar = app.theme.progress_bar(ctx_pct / 100.0, bar_w);
    lines.push(Line::from(vec![
        Span::styled(" Ctx  ", Style::default().fg(muted)),
        Span::styled(ctx_bar, Style::default().fg(ctx_color)),
        Span::styled(
            format!("  {:.0}%", ctx_pct),
            Style::default().fg(ctx_color).add_modifier(Modifier::BOLD),
        ),
    ]));

    if ih <= 3 {
        f.render_widget(Paragraph::new(lines), inner);
        return;
    }

    // ── Line 4: daily budget bar ──────────────────────────────────────────────
    let (bud_pct, bud_color) = if daily_limit > 0.0 {
        let p = (cost / daily_limit * 100.0).min(100.0);
        let c = if p >= 90.0 {
            app.theme.error_color()
        } else if p >= 70.0 {
            app.theme.warning_color()
        } else {
            app.theme.success_color()
        };
        (p, c)
    } else {
        (0.0, muted)
    };
    let bud_bar = app.theme.progress_bar(bud_pct / 100.0, bar_w);
    let bud_suffix = if daily_limit > 0.0 {
        format!("  {:.0}%", bud_pct)
    } else {
        "  no limit".to_string()
    };
    lines.push(Line::from(vec![
        Span::styled(" Bud  ", Style::default().fg(muted)),
        Span::styled(bud_bar, Style::default().fg(bud_color)),
        Span::styled(bud_suffix, Style::default().fg(bud_color)),
    ]));

    if ih <= 4 {
        f.render_widget(Paragraph::new(lines), inner);
        return;
    }

    // ── Line 5: health + turns ────────────────────────────────────────────────
    let health_color = app.theme.health_color(health);
    lines.push(Line::from(vec![
        Span::styled(
            format!(" Health {:.0}", health),
            Style::default()
                .fg(health_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ·  ", Style::default().fg(muted)),
        Span::styled(
            format!("{} turns today", turns_today),
            Style::default().fg(app.theme.text_primary()),
        ),
    ]));

    if ih <= 5 {
        f.render_widget(Paragraph::new(lines), inner);
        return;
    }

    // ── Line 6+: mini 7d sparkline (if enough room) ──────────────────────────
    if ih >= 7 && iw >= 20 {
        lines.push(Line::from("")); // spacer

        let data: Vec<f64> = app
            .global_stats
            .as_ref()
            .map(|g| {
                g.daily
                    .iter()
                    .rev()
                    .take(7)
                    .rev()
                    .map(|d| d.estimated_cost_usd)
                    .collect()
            })
            .unwrap_or_default();

        if !data.is_empty() {
            let max_v = data.iter().cloned().fold(0.0_f64, f64::max).max(0.001);
            let bar_chars = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
            let spark: String = data
                .iter()
                .map(|&v| {
                    let idx = ((v / max_v) * 7.0).min(7.0) as usize;
                    bar_chars[idx]
                })
                .collect();
            lines.push(Line::from(vec![
                Span::styled(" 7d  ", Style::default().fg(muted)),
                Span::styled(spark, Style::default().fg(app.theme.cost_color())),
                Span::styled(format!("  max ${:.2}", max_v), Style::default().fg(muted)),
            ]));
        }
    }

    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_tab_bar(f: &mut Frame, app: &App, area: Rect, sc: SizeClass) {
    let accent = app.theme.accent_color();
    let muted = app.theme.muted_color();
    let sep = Span::styled(" ┃ ", Style::default().fg(muted));

    // Compact mode: very short labels to fit 55-79 col terminals.
    let tab_labels: &[(&str, Tab, &str)] = if sc == SizeClass::Compact {
        &[
            ("1", Tab::Dashboard, "Dash"),
            ("2", Tab::Sessions, "Sess"),
            ("3", Tab::Insights, "Ins"),
            ("4", Tab::Budget, "Bud"),
            ("5", Tab::Providers, "Prov"),
            ("6", Tab::Agents, "Ag"),
        ]
    } else {
        &[
            ("1", Tab::Dashboard, "Dashboard"),
            ("2", Tab::Sessions, "Sessions"),
            ("3", Tab::Insights, "Insights"),
            ("4", Tab::Budget, "Budget"),
            ("5", Tab::Providers, "Providers"),
            ("6", Tab::Agents, "Agents"),
        ]
    };

    let mut spans: Vec<Span> = vec![logo_badge(app.theme)];

    for (key, tab, label) in tab_labels {
        spans.push(sep.clone());
        let is_active = app.tab == *tab;
        if is_active {
            spans.push(Span::styled(
                format!(" {}◆{} ", key, label),
                Style::default()
                    .fg(Color::Black)
                    .bg(accent)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                format!("{}:{}", key, label),
                Style::default().fg(muted),
            ));
        }
    }

    // Right-aligned breadcrumb — only in Standard mode (has room).
    if sc == SizeClass::Standard {
        let breadcrumb = app
            .live_stats
            .as_ref()
            .and_then(|s| s.session.as_ref())
            .map(|s| {
                if s.git_branch.is_empty() || s.git_branch == "—" {
                    s.project_name.clone()
                } else {
                    format!("{} ⎇ {}", s.project_name, s.git_branch)
                }
            })
            .unwrap_or_default();

        let hints_and_crumb = if breadcrumb.is_empty() {
            "  ?:help  q:quit  ".to_string()
        } else {
            let crumb_short = truncate_with_ellipsis(&breadcrumb, 50);
            format!("  {}  │  ?:help  q:quit  ", crumb_short)
        };

        spans.push(Span::styled(hints_and_crumb, Style::default().fg(muted)));
    } else {
        spans.push(Span::styled("  ?", Style::default().fg(muted)));
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ── Status bar ────────────────────────────────────────────────────────────────

fn draw_status_bar(f: &mut Frame, app: &App, area: Rect, sc: SizeClass) {
    let model = app
        .live_stats
        .as_ref()
        .and_then(|s| s.session.as_ref())
        .map(|s| shorten_model(&s.model))
        .unwrap_or("—".to_string());

    // Live / Idle / Copilot badge
    let (live_badge, live_color) = if app.is_live {
        ("◉ LIVE", app.theme.success_color())
    } else if app.copilot_active {
        ("◉ Copilot", app.theme.warning_color())
    } else if app.live_stats.is_some() {
        ("◎ IDLE", app.theme.muted_color())
    } else {
        ("◎ —", app.theme.muted_color())
    };

    // Idle time suffix
    let idle_suffix = if !app.is_live && !app.copilot_active && app.live_stats.is_some() {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let idle_ms = app
            .live_stats
            .as_ref()
            .and_then(|s| s.session.as_ref())
            .map(|s| now_ms - s.last_turn_at)
            .unwrap_or(0);
        if idle_ms < 3_600_000 {
            format!(" {}m", idle_ms / 60_000)
        } else {
            format!(" {}h", idle_ms / 3_600_000)
        }
    } else {
        String::new()
    };

    let sep = Span::styled("  │", Style::default().fg(app.theme.muted_color()));
    let muted = app.theme.muted_color();

    // Compact mode: fewer items to fit narrower terminals.
    if sc == SizeClass::Compact {
        let ctx_pct = app.budget.context_pressure_pct;
        let ctx_color = app.theme.context_color(ctx_pct);
        let refresh_indicator = if app.refresh_in_progress {
            format!("{} ", spinner_char(app.spinner_frame))
        } else {
            let secs = app
                .refresh_interval
                .saturating_sub(app.last_refresh.elapsed())
                .as_secs();
            format!("↻{}s ", secs)
        };
        let spans = vec![
            Span::styled(
                format!(" {} ", model),
                Style::default()
                    .fg(app.theme.model_color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("│", Style::default().fg(muted)),
            Span::styled(
                format!(" {}{} ", live_badge, idle_suffix),
                Style::default().fg(live_color),
            ),
            sep.clone(),
            Span::styled(
                format!(" ${:.2} ", app.budget.daily_spent),
                Style::default().fg(app.theme.cost_color()),
            ),
            sep.clone(),
            Span::styled(
                format!(" Ctx {:.0}% ", ctx_pct),
                Style::default().fg(ctx_color),
            ),
            sep.clone(),
            Span::styled(
                format!(" {} ", refresh_indicator),
                Style::default().fg(muted),
            ),
        ];
        f.render_widget(
            Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::Reset)),
            area,
        );
        return;
    }

    // Standard mode: full status bar.
    let cost_trend = if app.trend_cost_pct > 5.0 {
        "▲"
    } else if app.trend_cost_pct < -5.0 {
        "▼"
    } else {
        "─"
    };
    let trend_color = if app.trend_cost_pct > 5.0 {
        app.theme.error_color()
    } else if app.trend_cost_pct < -5.0 {
        app.theme.success_color()
    } else {
        app.theme.muted_color()
    };

    let turn_costs: Vec<f64> = app
        .live_stats
        .as_ref()
        .map(|s| s.turns.iter().map(|t| t.estimated_cost_usd).collect())
        .unwrap_or_default();
    let spark = micro_sparkline(&turn_costs, 5);

    let cache_pct = app
        .live_stats
        .as_ref()
        .map(|s| s.cache_hit_rate * 100.0)
        .unwrap_or(0.0);
    let cache_color = app.theme.cache_color(cache_pct);

    let ctx_pct = app.budget.context_pressure_pct;
    let ctx_color = app.theme.context_color(ctx_pct);
    let ctx_bar = app.theme.progress_bar(ctx_pct / 100.0, 8);

    let health_color = app.theme.health_color(app.health_score);

    let refresh_indicator = if app.refresh_in_progress {
        format!("{} ", spinner_char(app.spinner_frame))
    } else {
        let secs = app
            .refresh_interval
            .saturating_sub(app.last_refresh.elapsed())
            .as_secs();
        format!("↻{}s ", secs)
    };

    let hints: String = if let Some((msg, _)) = &app.toast {
        format!("  ✓ {}", msg)
    } else {
        build_hints(app)
    };

    // IS-15: Overhead transparency — show Scopeon's own memory footprint.
    let overhead_str = get_process_rss_mb()
        .map(|mb| format!("  [◈ {:.0}MB]", mb))
        .unwrap_or_default();

    let spans = vec![
        Span::styled(
            format!(" {} ", model),
            Style::default()
                .fg(app.theme.model_color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("│", Style::default().fg(app.theme.muted_color())),
        Span::styled(
            format!(" {}{} ", live_badge, idle_suffix),
            Style::default().fg(live_color),
        ),
        sep.clone(),
        Span::styled(
            format!(" ● {:.0} ", app.health_score),
            Style::default()
                .fg(health_color)
                .add_modifier(Modifier::BOLD),
        ),
        sep.clone(),
        Span::styled(
            format!(" ${:.2} {} ", app.budget.daily_spent, spark),
            Style::default().fg(app.theme.cost_color()),
        ),
        Span::styled(format!("{} ", cost_trend), Style::default().fg(trend_color)),
        sep.clone(),
        Span::styled(
            format!(" Cache {:.0}% ", cache_pct),
            Style::default().fg(cache_color),
        ),
        sep.clone(),
        Span::styled(
            format!(
                " Ctx {} {:.0}%{} ",
                ctx_bar,
                ctx_pct,
                app.budget
                    .predicted_turns_remaining
                    .map(|t| format!(" ~{}t", t))
                    .unwrap_or_default()
            ),
            Style::default().fg(ctx_color),
        ),
        sep.clone(),
        Span::styled(
            format!(" {} ", refresh_indicator),
            Style::default().fg(app.theme.muted_color()),
        ),
        Span::styled("│", Style::default().fg(app.theme.muted_color())),
        Span::styled(hints, Style::default().fg(app.theme.muted_color())),
        Span::styled(overhead_str, Style::default().fg(app.theme.muted_color())),
    ];

    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::Reset)),
        area,
    );
}

/// Build context-sensitive keyboard hints, rotating through sets every ~8 ticks.
fn build_hints(app: &App) -> String {
    let tick_group = (app.hint_tick / 40) as usize; // rotate every ~8s (40 × 200ms)

    match app.tab {
        Tab::Sessions if app.session_detail_mode => {
            if app.replay_turn_idx.is_some() {
                " ← →:scrub  ↑↓:scroll  Esc:exit replay".to_string()
            } else {
                " → :replay  ↑↓:scroll  g:top  G:bottom  Esc:back".to_string()
            }
        },
        Tab::Sessions if app.sessions_filter_active => {
            " cost>5  cache<40  tag:X  today  anomaly  model:X  ·  Esc:clear  Enter:apply"
                .to_string()
        },
        Tab::Sessions => {
            let sets = [
                " ↑↓:select  Enter:detail  /:filter  Tab:pane",
                " s:sort  g:top  G:bottom  Tab:pane",
            ];
            sets[tick_group % sets.len()].to_string()
        },
        Tab::Budget => {
            let sets = [
                " 1-6:tabs  r:refresh  ?:help",
                " Set limits: ~/.scopeon/config.toml",
            ];
            sets[tick_group % sets.len()].to_string()
        },
        Tab::Dashboard => {
            let sets = [
                " 1-6:tabs  ↑↓:scroll  r:refresh",
                " c:copy stats  ?:help  q:quit",
            ];
            sets[tick_group % sets.len()].to_string()
        },
        _ => " 1-6:tabs  ↑↓:scroll  r:refresh  ?:help".to_string(),
    }
}

// ── Help overlay ──────────────────────────────────────────────────────────────

fn draw_help_overlay(f: &mut Frame, app: &App, area: Rect) {
    let width = 58u16.min(area.width.saturating_sub(4));
    let height = 26u16.min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup = Rect {
        x,
        y,
        width,
        height,
    };

    f.render_widget(ratatui::widgets::Clear, popup);

    let accent = app.theme.accent_color();
    let heading = app.theme.heading_color();
    let muted = app.theme.muted_color();
    let text = app.theme.text_primary();

    let key_style = Style::default().fg(accent).add_modifier(Modifier::BOLD);
    let section_style = Style::default()
        .fg(heading)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);

    let version = env!("CARGO_PKG_VERSION");
    let theme_name = match app.theme {
        Theme::Cockpit => "cockpit",
        Theme::HighContrast => "high-contrast",
        Theme::Standard => "standard",
    };

    let rows = vec![
        Row::new(vec![
            Cell::from(Span::styled("  Navigation", section_style)),
            Cell::from(""),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  1-6", key_style)),
            Cell::from(Span::styled("  Switch tabs", Style::default().fg(text))),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  ↑↓ / j k", key_style)),
            Cell::from(Span::styled("  Scroll / select", Style::default().fg(text))),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  Tab / BackTab", key_style)),
            Cell::from(Span::styled("  Next / prev tab", Style::default().fg(text))),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  g / G", key_style)),
            Cell::from(Span::styled(
                "  Jump to top / bottom",
                Style::default().fg(text),
            )),
        ]),
        Row::new(vec![Cell::from(""), Cell::from("")]),
        Row::new(vec![
            Cell::from(Span::styled("  Sessions Tab", section_style)),
            Cell::from(""),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  Enter", key_style)),
            Cell::from(Span::styled(
                "  Open session detail",
                Style::default().fg(text),
            )),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  Esc", key_style)),
            Cell::from(Span::styled(
                "  Back from detail",
                Style::default().fg(text),
            )),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  /", key_style)),
            Cell::from(Span::styled("  Filter sessions", Style::default().fg(text))),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  s", key_style)),
            Cell::from(Span::styled(
                "  Cycle sort (Newest / Oldest / Cost)",
                Style::default().fg(text),
            )),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  → / ←", key_style)),
            Cell::from(Span::styled(
                "  Scrub turns (replay mode)",
                Style::default().fg(text),
            )),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  Tab", key_style)),
            Cell::from(Span::styled(
                "  Switch list ↔ detail pane",
                Style::default().fg(text),
            )),
        ]),
        Row::new(vec![Cell::from(""), Cell::from("")]),
        Row::new(vec![
            Cell::from(Span::styled("  General", section_style)),
            Cell::from(""),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  z", key_style)),
            Cell::from(Span::styled(
                "  Toggle zen mode (ambient 1-line)",
                Style::default().fg(text),
            )),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  r", key_style)),
            Cell::from(Span::styled("  Force refresh", Style::default().fg(text))),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  c", key_style)),
            Cell::from(Span::styled(
                "  Copy stats to clipboard",
                Style::default().fg(text),
            )),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  ?", key_style)),
            Cell::from(Span::styled(
                "  Toggle this help",
                Style::default().fg(text),
            )),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  q", key_style)),
            Cell::from(Span::styled("  Quit", Style::default().fg(text))),
        ]),
        Row::new(vec![Cell::from(""), Cell::from("")]),
        Row::new(vec![
            Cell::from(Span::styled("  Theme", section_style)),
            Cell::from(""),
        ]),
        Row::new(vec![
            Cell::from(Span::styled(format!("  {}", theme_name), key_style)),
            Cell::from(Span::styled(
                "  Set [general] theme in config.toml",
                Style::default().fg(muted),
            )),
        ]),
        Row::new(vec![Cell::from(""), Cell::from("")]),
        Row::new(vec![
            Cell::from(Span::styled(
                format!("  v{}  •  Press any key to close", version),
                Style::default().fg(muted),
            )),
            Cell::from(""),
        ]),
    ];

    let table = Table::new(rows, [Constraint::Length(18), Constraint::Min(0)]).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(app.theme.border_type())
            .border_style(Style::default().fg(accent))
            .title(Span::styled(
                " ◈ Scopeon Help ",
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            )),
    );

    f.render_widget(table, popup);
}

fn shorten_model(model: &str) -> String {
    if let Some(s) = model.strip_prefix("claude-") {
        let parts: Vec<&str> = s.split('-').collect();
        if parts.len() >= 2 {
            let name = format!("{}-{}", parts[0], parts[1]);
            return truncate_to_chars(&name, 14);
        }
    }
    truncate_to_chars(model, 14)
}

/// IS-15: Read current process RSS memory in MB on macOS and Linux.
/// Returns None when the platform or /proc are unavailable.
fn get_process_rss_mb() -> Option<f64> {
    // Linux: parse /proc/self/status for VmRSS
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("VmRSS:") {
                let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
                return Some(kb as f64 / 1024.0);
            }
        }
        return None;
    }

    // macOS: use task_info via extern C
    #[cfg(target_os = "macos")]
    {
        use std::mem;
        #[repr(C)]
        struct MachTaskBasicInfo {
            virtual_size: u64,
            resident_size: u64,
            resident_size_max: u64,
            user_time: [u32; 2],
            system_time: [u32; 2],
            policy: i32,
            suspend_count: i32,
        }
        extern "C" {
            fn mach_task_self() -> u32;
            fn task_info(
                task: u32,
                flavor: u32,
                task_info_out: *mut u8,
                task_info_count: *mut u32,
            ) -> i32;
        }
        const MACH_TASK_BASIC_INFO: u32 = 20;
        let mut info: MachTaskBasicInfo = unsafe { mem::zeroed() };
        let mut count = (mem::size_of::<MachTaskBasicInfo>() / mem::size_of::<u32>()) as u32;
        let ret = unsafe {
            task_info(
                mach_task_self(),
                MACH_TASK_BASIC_INFO,
                &mut info as *mut _ as *mut u8,
                &mut count,
            )
        };
        if ret == 0 {
            return Some(info.resident_size as f64 / 1_048_576.0);
        }
        return None;
    }

    #[allow(unreachable_code)]
    None
}

/// Truncate a string to `max_chars` characters, appending "…" if truncated.
fn truncate_str(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        s.to_string()
    } else {
        format!(
            "{}…",
            chars[..max_chars.saturating_sub(1)]
                .iter()
                .collect::<String>()
        )
    }
}
