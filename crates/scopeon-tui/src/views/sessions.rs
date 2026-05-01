//! Tab 1: Sessions — dashboard overview + full-width interactive session list.
//!
//! Top: 3-card overview strip (Today / Efficiency / Providers) — hides on tiny terminals.
//! Below: scrollable session list (newest first), selectable with ↑↓.
//! Enter: full-screen session detail (turns table + compact header).
//! /: filter sessions. s: cycle sort order. []: provider scope. {}: model scope.
//! t: toggle Trends chart. [ / ] (in detail): cycle Turns / Context / MCP & Skills.

use std::{cmp::Reverse, collections::HashMap};

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Bar, BarChart, BarGroup, Block, Borders, Cell, Paragraph, Row, Table},
    Frame,
};

use scopeon_core::{
    context_window_for_model, shadow_cost, Session, SessionStats, ToolBreakdownItem,
};

use crate::app::{App, DetailSection};
use crate::text::{truncate_to_chars, truncate_with_ellipsis};
use crate::views::components::{
    empty_state_lines, micro_sparkline, themed_block, themed_block_borders,
};

/// Returns the number of rows the scope selector bar will occupy.
/// 0 when fewer than 2 providers (nothing to select), 1 for providers-only,
/// 2 when a provider is scoped and 2+ models are available (or a model scope
/// is active, which guards against any state where scope_model was set
/// without a provider scope).
pub(crate) fn compute_scope_h(app: &App) -> u16 {
    if app.all_providers.len() < 2 {
        return 0;
    }
    let model_row =
        (app.scope_provider.is_some() && app.all_models.len() >= 2) || app.scope_model.is_some();
    1 + model_row as u16
}

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    // Full-screen detail mode (Enter key)
    if app.session_detail_mode {
        draw_session_detail_fullscreen(f, app, area);
        return;
    }

    let sessions = app.filtered_sessions();

    // True empty: no sessions at all (first run)
    if app.sessions_list.is_empty() {
        let lines = empty_state_lines(
            app.theme,
            "⬡",
            "No sessions yet",
            "Run Claude Code or Codex in any project to start collecting data.",
            "r",
            "refresh",
        );
        let msg = Paragraph::new(lines).block(themed_block(app.theme, "Sessions", false));
        f.render_widget(msg, area);
        return;
    }

    // Scope selector bar: 1 row (providers) or 2 rows (providers + models).
    let scope_h = compute_scope_h(app);

    // Overview cards: 7 rows (5 content + 2 borders).
    // Threshold includes scope_h so the list always has ≥7 rows below.
    let has_data = app.budget.daily_spent > 0.0 || app.global_stats.is_some();
    let cards_h = if has_data && area.height >= 14 + scope_h {
        7u16
    } else {
        0u16
    };

    // Build layout constraints dynamically.
    let mut constraints: Vec<Constraint> = Vec::new();
    if cards_h > 0 {
        constraints.push(Constraint::Length(cards_h));
    }
    if scope_h > 0 {
        constraints.push(Constraint::Length(scope_h));
    }
    constraints.push(Constraint::Min(0));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let mut chunk_idx = 0usize;
    if cards_h > 0 {
        draw_overview_cards(f, app, chunks[chunk_idx]);
        chunk_idx += 1;
    }
    if scope_h > 0 {
        draw_scope_bar(f, app, chunks[chunk_idx]);
        chunk_idx += 1;
    }

    // Split list area horizontally on wide terminals: list (38%) | preview (62%).
    let list_area = chunks[chunk_idx];
    const SPLIT_THRESHOLD: u16 = 120;
    if list_area.width >= SPLIT_THRESHOLD {
        let horiz = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
            .split(list_area);
        draw_session_list(f, app, &sessions, horiz[0]);
        draw_session_preview_panel(f, app, horiz[1]);
    } else {
        draw_session_list(f, app, &sessions, list_area);
    }
}

// ── Scope selector bar ────────────────────────────────────────────────────────
//
// A 1-2 row borderless strip between the overview cards and the session list.
// Row 1: provider chips (always shown when 2+ providers exist).
// Row 2: model chips (shown when a provider is scoped AND 2+ models exist,
//         or defensively when a model scope is active without a provider scope).
//
// Active chip: accent_color + BOLD  Inactive: accent_dim  Label: muted
// Key hints are embedded at the end of each row (self-documenting).

fn draw_scope_bar(f: &mut Frame, app: &App, area: Rect) {
    let show_model_row =
        (app.scope_provider.is_some() && app.all_models.len() >= 2) || app.scope_model.is_some();

    let (prov_area, model_area) = if show_model_row && area.height >= 2 {
        let splits = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(area);
        (splits[0], Some(splits[1]))
    } else {
        (area, None)
    };

    draw_provider_chip_row(f, app, prov_area);
    if let Some(ma) = model_area {
        draw_model_chip_row(f, app, ma);
    }
}

/// Builds a chip row into `spans` and returns how many total chars were used.
/// Chips are rendered in this priority order on overflow:
///   1. Label (always)
///   2. "All" chip (always)
///   3. Active chip (always, if any)
///   4. Remaining chips left-to-right
///   5. `+N` overflow indicator
fn scope_chip_spans(
    label: &str,
    options: &[String],              // all available options (without "All")
    active: Option<&str>,            // currently selected option (None = All)
    counts: &HashMap<String, usize>, // session count per option name
    hint: &str,                      // key hint suffix e.g. "[ ] cycle   Esc"
    show_esc: bool,                  // whether to show Esc hint
    total_width: usize,
    t: crate::theme::Theme,
) -> Vec<Span<'static>> {
    let label_style = Style::default().fg(t.muted_color());
    let active_style = Style::default()
        .fg(t.accent_color())
        .add_modifier(Modifier::BOLD);
    let inactive_style = Style::default().fg(t.accent_dim());
    let muted = Style::default().fg(t.muted_color());

    // Compute widths of fixed elements.
    let label_w = label.chars().count();
    let all_label = if active.is_none() {
        "◉ All"
    } else {
        "○ All"
    };
    let all_w = all_label.chars().count();
    // hint suffix (right side)
    let hint_full = if show_esc {
        format!("   {}   Esc", hint)
    } else {
        format!("   {}", hint)
    };
    let hint_w = hint_full.chars().count();

    // Budget for chips after label, All, and hint.
    let mut remaining_w = total_width.saturating_sub(label_w + 2 + all_w + hint_w + 4); // 4 = "  ·  " separators

    // Decide which named chips to render.
    // Priority: active chip first, then others, truncate with +N.
    let mut selected_chips: Vec<&str> = Vec::new();
    let mut skipped = 0usize;

    // Always include active chip if it's not "All"
    if let Some(a) = active {
        if let Some(full) = options.iter().find(|o| o.as_str() == a) {
            let cnt = counts.get(full.as_str()).copied().unwrap_or(0);
            let w = chip_display_width(full, cnt) + 4; // + "  ·  " pad
            if remaining_w >= w {
                selected_chips.push(full.as_str());
                remaining_w = remaining_w.saturating_sub(w);
            }
        }
    }

    // Fill in the rest in order
    for opt in options {
        if active == Some(opt.as_str()) {
            continue; // already added
        }
        let cnt = counts.get(opt.as_str()).copied().unwrap_or(0);
        let w = chip_display_width(opt, cnt) + 4;
        if remaining_w >= w {
            selected_chips.push(opt.as_str());
            remaining_w = remaining_w.saturating_sub(w);
        } else {
            skipped += 1;
        }
    }

    // Build spans
    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(Span::styled(format!("  {}", label), label_style));

    // All chip
    spans.push(Span::styled("  ".to_string(), muted));
    spans.push(Span::styled(
        all_label.to_string(),
        if active.is_none() {
            active_style
        } else {
            inactive_style
        },
    ));

    // Named chips (in order: active first, then as collected)
    let mut ordered: Vec<&str> = Vec::new();
    if let Some(a) = active {
        if selected_chips.contains(&a) {
            ordered.push(a);
        }
    }
    for c in &selected_chips {
        if Some(*c) != active {
            ordered.push(c);
        }
    }

    for name in ordered {
        let is_active = active == Some(name);
        let cnt = counts.get(name).copied().unwrap_or(0);
        let display_name = truncate_with_ellipsis(name, 14);
        let chip_str = if cnt > 0 {
            format!(
                "{}  {} ·{}",
                if is_active { "  ●" } else { "  ○" },
                display_name,
                cnt
            )
        } else {
            format!(
                "{}  {}",
                if is_active { "  ●" } else { "  ○" },
                display_name
            )
        };
        spans.push(Span::styled(
            chip_str,
            if is_active {
                active_style
            } else {
                inactive_style
            },
        ));
    }

    if skipped > 0 {
        spans.push(Span::styled(
            format!("  +{}", skipped),
            Style::default().fg(t.muted_color()),
        ));
    }

    // Key hint
    spans.push(Span::styled(hint_full, muted));

    spans
}

fn chip_display_width(name: &str, count: usize) -> usize {
    let n = name.chars().count().min(14);
    if count > 0 {
        n + count.to_string().len() + 4 // "  ○  name·NN"
    } else {
        n + 3 // "  ○  name"
    }
}

fn draw_provider_chip_row(f: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let show_esc = app.scope_provider.is_some();

    // Count sessions per provider (unscoped, from full list)
    let mut counts: HashMap<String, usize> = HashMap::new();
    for s in &app.sessions_list {
        if !s.provider.is_empty() {
            *counts.entry(s.provider.clone()).or_insert(0) += 1;
        }
    }

    let hint = "[ ]  cycle";
    let spans = scope_chip_spans(
        "◈  Provider",
        &app.all_providers,
        app.scope_provider.as_deref(),
        &counts,
        hint,
        show_esc,
        area.width as usize,
        t,
    );

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_model_chip_row(f: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;

    // Count sessions per model, filtered by current provider scope
    let mut counts: HashMap<String, usize> = HashMap::new();
    for s in &app.sessions_list {
        let prov_match = app
            .scope_provider
            .as_ref()
            .map(|p| p == &s.provider)
            .unwrap_or(true);
        if prov_match {
            *counts.entry(s.model.clone()).or_insert(0) += 1;
        }
    }

    let hint = "{ }  cycle";
    let spans = scope_chip_spans(
        "◈  Model   ",
        &app.all_models,
        app.scope_model.as_deref(),
        &counts,
        hint,
        false,
        area.width as usize,
        t,
    );

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ── Scoped provider/model dashboards ─────────────────────────────────────────

/// Aggregated stats computed from `sessions_list` + `session_summaries` for a
/// given provider and/or model filter. Used by provider and model dashboards.
struct ScopedStats {
    session_count: usize,
    total_cost: f64,
    today_cost: f64,
    today_sessions: usize,
    week_sessions: usize,
    week_cost: f64,
    prev_week_cost: f64,
    avg_cost_per_session: f64,
    avg_cache_rate: f64,
    total_turns: i64,
    avg_turns_per_session: f64,
    top_models: Vec<(String, usize)>,
    top_projects: Vec<(String, usize)>,
    top_providers: Vec<(String, usize)>,
}

fn compute_scoped_stats(app: &App, provider: Option<&str>, model: Option<&str>) -> ScopedStats {
    let today_str = chrono::Local::now().format("%Y-%m-%d").to_string();
    let now_ms = chrono::Utc::now().timestamp_millis();
    let week_ms = 7 * 24 * 3600 * 1000i64;
    let prev_week_ms = 14 * 24 * 3600 * 1000i64;

    let sessions: Vec<&Session> = app
        .sessions_list
        .iter()
        .filter(|s| {
            provider.map(|p| s.provider == p).unwrap_or(true)
                && model.map(|m| s.model == m).unwrap_or(true)
        })
        .collect();

    let session_count = sessions.len();
    let mut total_cost = 0.0f64;
    let mut today_cost = 0.0f64;
    let mut today_sessions = 0usize;
    let mut week_sessions = 0usize;
    let mut week_cost = 0.0f64;
    let mut prev_week_cost = 0.0f64;
    let mut total_turns = 0i64;
    let mut cache_rates: Vec<f64> = Vec::new();
    let mut model_counts: HashMap<String, usize> = HashMap::new();
    let mut project_counts: HashMap<String, usize> = HashMap::new();
    let mut provider_counts: HashMap<String, usize> = HashMap::new();

    for s in &sessions {
        let cost = app
            .session_summaries
            .get(&s.id)
            .map(|sm| sm.estimated_cost_usd)
            .unwrap_or(0.0);
        let cache = app
            .session_summaries
            .get(&s.id)
            .map(|sm| sm.cache_hit_rate)
            .unwrap_or(0.0);

        total_cost += cost;
        total_turns += s.total_turns;
        cache_rates.push(cache);

        let sess_date = chrono::DateTime::from_timestamp_millis(s.started_at)
            .map(|dt| {
                dt.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d")
                    .to_string()
            })
            .unwrap_or_default();

        if sess_date == today_str {
            today_cost += cost;
            today_sessions += 1;
        }
        let age_ms = now_ms - s.started_at;
        if age_ms <= week_ms {
            week_sessions += 1;
            week_cost += cost;
        } else if age_ms <= prev_week_ms {
            prev_week_cost += cost;
        }

        if !s.model.is_empty() {
            *model_counts.entry(s.model.clone()).or_insert(0) += 1;
        }
        if !s.project_name.is_empty() {
            *project_counts.entry(s.project_name.clone()).or_insert(0) += 1;
        }
        if !s.provider.is_empty() {
            *provider_counts.entry(s.provider.clone()).or_insert(0) += 1;
        }
    }

    let avg_cost = if session_count > 0 {
        total_cost / session_count as f64
    } else {
        0.0
    };
    let avg_cache = if !cache_rates.is_empty() {
        cache_rates.iter().sum::<f64>() / cache_rates.len() as f64
    } else {
        0.0
    };
    let avg_turns = if session_count > 0 {
        total_turns as f64 / session_count as f64
    } else {
        0.0
    };

    let mut top_models: Vec<(String, usize)> = model_counts.into_iter().collect();
    top_models.sort_by_key(|(_, count)| Reverse(*count));
    let mut top_projects: Vec<(String, usize)> = project_counts.into_iter().collect();
    top_projects.sort_by_key(|(_, count)| Reverse(*count));
    let mut top_providers: Vec<(String, usize)> = provider_counts.into_iter().collect();
    top_providers.sort_by_key(|(_, count)| Reverse(*count));

    ScopedStats {
        session_count,
        total_cost,
        today_cost,
        today_sessions,
        week_sessions,
        week_cost,
        prev_week_cost,
        avg_cost_per_session: avg_cost,
        avg_cache_rate: avg_cache,
        total_turns,
        avg_turns_per_session: avg_turns,
        top_models,
        top_projects,
        top_providers,
    }
}

fn draw_provider_dashboard(f: &mut Frame, app: &App, area: Rect, provider: &str) {
    let stats = compute_scoped_stats(app, Some(provider), None);
    let t = app.theme;

    let cols = if area.width >= 90 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(35),
                Constraint::Percentage(33),
                Constraint::Percentage(32),
            ])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area)
    };

    // ── Column 1: Activity ──────────────────────────────────────────────────
    {
        let m = t.muted_color();
        let trend_str = week_trend_str(stats.week_cost, stats.prev_week_cost);
        let mut lines: Vec<Line<'static>> = vec![];

        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                format!("{}", stats.session_count),
                Style::default()
                    .fg(t.text_primary())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" sessions", Style::default().fg(m)),
            Span::styled(
                format!("   {}t total", fmt_k(stats.total_turns)),
                Style::default().fg(m),
            ),
        ]));

        if stats.avg_turns_per_session > 0.0 {
            lines.push(Line::from(vec![Span::styled(
                format!("  avg {:.0}t/session", stats.avg_turns_per_session),
                Style::default().fg(m),
            )]));
        }

        if stats.today_sessions > 0 || stats.today_cost > 0.0 {
            lines.push(Line::from(vec![
                Span::styled("  today  ", Style::default().fg(m)),
                Span::styled(
                    format!("{} sessions", stats.today_sessions),
                    Style::default().fg(t.text_secondary()),
                ),
                if stats.today_cost > 0.0 {
                    Span::styled(
                        format!("  ${:.2}", stats.today_cost),
                        Style::default().fg(t.cost_color()),
                    )
                } else {
                    Span::styled("", Style::default())
                },
            ]));
        }

        if stats.week_sessions > 0 {
            lines.push(Line::from(vec![
                Span::styled("  7d  ", Style::default().fg(m)),
                Span::styled(
                    format!("{} sessions", stats.week_sessions),
                    Style::default().fg(t.text_secondary()),
                ),
            ]));
        }

        if !trend_str.is_empty() {
            let trend_col = if trend_str.starts_with('↑') {
                t.error_color()
            } else if trend_str.starts_with('↓') {
                t.success_color()
            } else {
                m
            };
            lines.push(Line::from(Span::styled(
                format!("  {}", trend_str),
                Style::default().fg(trend_col),
            )));
        }

        let title = format!(" ◉ {} ", provider);
        f.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(t.border_type())
                    .border_style(t.active_border_style())
                    .title(title),
            ),
            cols[0],
        );
    }

    // ── Column 2: Cost & Cache ──────────────────────────────────────────────
    {
        let m = t.muted_color();
        let cache_pct = stats.avg_cache_rate * 100.0;
        let cache_col = t.cache_color(cache_pct);
        let bar_w = (cols[1].width.saturating_sub(14) as usize).clamp(6, 14);
        let cache_bar = fill_bar(stats.avg_cache_rate, bar_w);

        let mut lines: Vec<Line<'static>> = vec![];

        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                format!("${:.3}", stats.total_cost),
                Style::default()
                    .fg(t.cost_color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  total", Style::default().fg(m)),
        ]));

        if stats.avg_cost_per_session > 0.0 {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    format!("${:.3}", stats.avg_cost_per_session),
                    Style::default().fg(t.cost_color()),
                ),
                Span::styled("  avg / session", Style::default().fg(m)),
            ]));
        }

        if stats.today_cost > 0.0 {
            lines.push(Line::from(vec![
                Span::styled("  today  ", Style::default().fg(m)),
                Span::styled(
                    format!("${:.3}", stats.today_cost),
                    Style::default().fg(t.cost_color()),
                ),
            ]));
        }

        if cache_pct > 0.0 {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(cache_bar, Style::default().fg(cache_col)),
                Span::styled(
                    format!(" {:.0}% cache", cache_pct),
                    Style::default().fg(cache_col).add_modifier(Modifier::BOLD),
                ),
            ]));
        }

        f.render_widget(
            Paragraph::new(lines).block(themed_block_borders(
                t,
                "Cost",
                false,
                Borders::TOP | Borders::RIGHT | Borders::BOTTOM,
            )),
            cols[1],
        );
    }

    // ── Column 3: Models used (only on wide terminals) ──────────────────────
    if cols.len() > 2 {
        let m = t.muted_color();
        let total_sess = stats.session_count.max(1);
        let name_w = (cols[2].width.saturating_sub(18) as usize).clamp(6, 14);
        let bar_w = 6usize;

        let mut lines: Vec<Line<'static>> = vec![];
        for (model_name, cnt) in stats.top_models.iter().take(3) {
            let ratio = *cnt as f64 / total_sess as f64;
            let filled = (ratio * bar_w as f64) as usize;
            let bar = "█".repeat(filled) + &"░".repeat(bar_w - filled);
            let short = shorten_model(model_name);
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    format!("{:<w$}", truncate_with_ellipsis(&short, name_w), w = name_w),
                    Style::default().fg(t.model_color()),
                ),
                Span::styled(
                    format!(" {:>3}", cnt),
                    Style::default()
                        .fg(t.text_primary())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("  {}", bar), Style::default().fg(m)),
            ]));
        }

        if lines.is_empty() {
            lines.push(Line::from(Span::styled(
                "  No models yet",
                Style::default().fg(m),
            )));
        }

        f.render_widget(
            Paragraph::new(lines).block(themed_block_borders(
                t,
                "Models",
                false,
                Borders::TOP | Borders::RIGHT | Borders::BOTTOM,
            )),
            cols[2],
        );
    }
}

fn draw_model_dashboard(f: &mut Frame, app: &App, area: Rect, provider: Option<&str>, model: &str) {
    let stats = compute_scoped_stats(app, provider, Some(model));
    let t = app.theme;
    let ctx_window = context_window_for_model(model);

    let cols = if area.width >= 90 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(35),
                Constraint::Percentage(33),
                Constraint::Percentage(32),
            ])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area)
    };

    // ── Column 1: Activity ──────────────────────────────────────────────────
    {
        let m = t.muted_color();
        let short_model = shorten_model(model);
        let trend_str = week_trend_str(stats.week_cost, stats.prev_week_cost);

        let mut lines: Vec<Line<'static>> = vec![];

        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                format!("{}", stats.session_count),
                Style::default()
                    .fg(t.text_primary())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" sessions", Style::default().fg(m)),
            Span::styled(
                format!("   {}t total", fmt_k(stats.total_turns)),
                Style::default().fg(m),
            ),
        ]));

        if stats.avg_turns_per_session > 0.0 {
            lines.push(Line::from(vec![Span::styled(
                format!("  avg {:.0}t/session", stats.avg_turns_per_session),
                Style::default().fg(m),
            )]));
        }

        if stats.today_sessions > 0 {
            lines.push(Line::from(vec![
                Span::styled("  today  ", Style::default().fg(m)),
                Span::styled(
                    format!("{} sessions", stats.today_sessions),
                    Style::default().fg(t.text_secondary()),
                ),
            ]));
        }

        if stats.week_sessions > 0 {
            lines.push(Line::from(vec![
                Span::styled("  7d  ", Style::default().fg(m)),
                Span::styled(
                    format!("{} sessions", stats.week_sessions),
                    Style::default().fg(t.text_secondary()),
                ),
            ]));
        }

        if !stats.top_providers.is_empty() {
            let pvds = stats
                .top_providers
                .iter()
                .take(2)
                .map(|(p, _)| shorten_model(p))
                .collect::<Vec<_>>()
                .join(" · ");
            lines.push(Line::from(vec![
                Span::styled("  via  ", Style::default().fg(m)),
                Span::styled(pvds, Style::default().fg(t.text_secondary())),
            ]));
        }

        if !trend_str.is_empty() {
            let trend_col = if trend_str.starts_with('↑') {
                t.error_color()
            } else if trend_str.starts_with('↓') {
                t.success_color()
            } else {
                m
            };
            lines.push(Line::from(Span::styled(
                format!("  {}", trend_str),
                Style::default().fg(trend_col),
            )));
        }

        let title = format!(" ◉ {} ", short_model);
        f.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(t.border_type())
                    .border_style(t.active_border_style())
                    .title(title),
            ),
            cols[0],
        );
    }

    // ── Column 2: Cost, Cache & Context window ──────────────────────────────
    {
        let m = t.muted_color();
        let cache_pct = stats.avg_cache_rate * 100.0;
        let cache_col = t.cache_color(cache_pct);
        let bar_w = (cols[1].width.saturating_sub(14) as usize).clamp(6, 14);
        let cache_bar = fill_bar(stats.avg_cache_rate, bar_w);

        let ctx_str = if ctx_window >= 1_000_000 {
            format!("{:.0}M ctx", ctx_window as f64 / 1_000_000.0)
        } else {
            format!("{}k ctx", ctx_window / 1000)
        };

        let mut lines: Vec<Line<'static>> = vec![];

        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                format!("${:.3}", stats.total_cost),
                Style::default()
                    .fg(t.cost_color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  total", Style::default().fg(m)),
        ]));

        if stats.avg_cost_per_session > 0.0 {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    format!("${:.3}", stats.avg_cost_per_session),
                    Style::default().fg(t.cost_color()),
                ),
                Span::styled("  avg / session", Style::default().fg(m)),
            ]));
        }

        if cache_pct > 0.0 {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(cache_bar, Style::default().fg(cache_col)),
                Span::styled(
                    format!(" {:.0}% cache", cache_pct),
                    Style::default().fg(cache_col).add_modifier(Modifier::BOLD),
                ),
            ]));
        }

        if ctx_window > 0 {
            lines.push(Line::from(vec![Span::styled(
                format!("  {}", ctx_str),
                Style::default().fg(m),
            )]));
        }

        f.render_widget(
            Paragraph::new(lines).block(themed_block_borders(
                t,
                "Cost",
                false,
                Borders::TOP | Borders::RIGHT | Borders::BOTTOM,
            )),
            cols[1],
        );
    }

    // ── Column 3: Top projects (only on wide terminals) ─────────────────────
    if cols.len() > 2 {
        let m = t.muted_color();
        let total_sess = stats.session_count.max(1);
        let name_w = (cols[2].width.saturating_sub(16) as usize).clamp(6, 16);
        let bar_w = 6usize;

        let mut lines: Vec<Line<'static>> = vec![];
        for (proj, cnt) in stats.top_projects.iter().take(3) {
            let ratio = *cnt as f64 / total_sess as f64;
            let filled = (ratio * bar_w as f64) as usize;
            let bar = "█".repeat(filled) + &"░".repeat(bar_w - filled);
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    format!("{:<w$}", truncate_with_ellipsis(proj, name_w), w = name_w),
                    Style::default().fg(t.text_primary()),
                ),
                Span::styled(
                    format!(" {:>3}", cnt),
                    Style::default()
                        .fg(t.text_secondary())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("  {}", bar), Style::default().fg(m)),
            ]));
        }

        if lines.is_empty() {
            lines.push(Line::from(Span::styled(
                "  No projects yet",
                Style::default().fg(m),
            )));
        }

        f.render_widget(
            Paragraph::new(lines).block(themed_block_borders(
                t,
                "Projects",
                false,
                Borders::TOP | Borders::RIGHT | Borders::BOTTOM,
            )),
            cols[2],
        );
    }
}

/// Cost trend string comparing this week vs previous week.
fn week_trend_str(this_week: f64, prev_week: f64) -> String {
    if prev_week > 0.01 {
        let pct = (this_week - prev_week) / prev_week * 100.0;
        if pct > 10.0 {
            format!("↑ +{:.0}% vs prev week", pct)
        } else if pct < -10.0 {
            format!("↓ {:.0}% vs prev week", pct.abs())
        } else {
            "≈ stable spend".to_string()
        }
    } else if this_week > 0.0 {
        "● new this week".to_string()
    } else {
        String::new()
    }
}

// ── Overview cards ────────────────────────────────────────────────────────────

fn draw_overview_cards(f: &mut Frame, app: &App, area: Rect) {
    // Progressive disclosure: when a scope is active, replace global cards with
    // a scoped dashboard showing stats specific to the selected provider/model.
    match (&app.scope_provider, &app.scope_model) {
        (Some(p), Some(m)) => {
            draw_model_dashboard(f, app, area, Some(p.as_str()), m.as_str());
        },
        (Some(p), None) => {
            draw_provider_dashboard(f, app, area, p.as_str());
        },
        _ => {
            if app.show_trends {
                draw_trends_section(f, app, area);
            } else {
                draw_global_cards(f, app, area);
            }
        },
    }
}

fn draw_global_cards(f: &mut Frame, app: &App, area: Rect) {
    if area.width < 90 {
        // Narrow: two cards side-by-side
        let h = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);
        draw_today_card(f, app, h[0]);
        draw_efficiency_card(f, app, h[1]);
    } else {
        // Wide: three equal cards
        let h = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(35),
                Constraint::Percentage(33),
                Constraint::Percentage(32),
            ])
            .split(area);
        draw_today_card(f, app, h[0]);
        draw_efficiency_card(f, app, h[1]);
        draw_providers_card(f, app, h[2]);
    }
}

fn draw_today_card(f: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let today_str = chrono::Local::now().format("%Y-%m-%d").to_string();

    // Compute today's stats directly from sessions_list + session_summaries.
    let today_sessions = app
        .sessions_list
        .iter()
        .filter(|s| {
            chrono::DateTime::from_timestamp_millis(s.started_at)
                .map(|dt| {
                    dt.with_timezone(&chrono::Local)
                        .format("%Y-%m-%d")
                        .to_string()
                        == today_str
                })
                .unwrap_or(false)
        })
        .count();

    let today_cost: f64 = app
        .sessions_list
        .iter()
        .filter(|s| {
            chrono::DateTime::from_timestamp_millis(s.started_at)
                .map(|dt| {
                    dt.with_timezone(&chrono::Local)
                        .format("%Y-%m-%d")
                        .to_string()
                        == today_str
                })
                .unwrap_or(false)
        })
        .filter_map(|s| app.session_summaries.get(&s.id))
        .map(|sm| sm.estimated_cost_usd)
        .sum();

    let today_turns: i64 = app
        .sessions_list
        .iter()
        .filter(|s| {
            chrono::DateTime::from_timestamp_millis(s.started_at)
                .map(|dt| {
                    dt.with_timezone(&chrono::Local)
                        .format("%Y-%m-%d")
                        .to_string()
                        == today_str
                })
                .unwrap_or(false)
        })
        .map(|s| s.total_turns)
        .sum();

    let spent = today_cost.max(app.budget.daily_spent);
    let limit = app.budget.daily_limit;

    // When there's no activity today, show all-time totals instead so the card
    // always has meaningful data (consistent with the scoped provider/model views).
    let has_today = today_sessions > 0 || spent > 0.001;

    let (live_icon, live_color) = if app.is_live {
        ("◉", t.success_color())
    } else if app.copilot_active {
        ("◉", t.warning_color())
    } else {
        ("◎", t.muted_color())
    };

    let m = t.muted_color();
    let mut lines: Vec<Line<'static>> = vec![];

    if has_today {
        let trend_glyph = if app.trend_cost_pct > 2.0 {
            " ↑"
        } else if app.trend_cost_pct < -2.0 {
            " ↓"
        } else {
            " ≈"
        };

        lines.push(Line::from(vec![
            Span::styled(format!("  {} ", live_icon), Style::default().fg(live_color)),
            Span::styled(
                format!("${:.2}{}", spent, trend_glyph),
                Style::default()
                    .fg(t.cost_color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("   {}t  {}s", today_turns, today_sessions),
                Style::default().fg(t.text_secondary()),
            ),
        ]));

        if limit > 0.0 {
            let runway_pct = (spent / limit * 100.0).min(100.0);
            let bar_w = (area.width.saturating_sub(14) as usize).clamp(6, 14);
            let filled = (runway_pct / 100.0 * bar_w as f64) as usize;
            let bar = "█".repeat(filled) + &"░".repeat(bar_w - filled);
            let bar_color = if runway_pct >= 90.0 {
                t.error_color()
            } else if runway_pct >= 70.0 {
                t.warning_color()
            } else {
                t.success_color()
            };
            let remaining = (limit - spent).max(0.0);
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(bar, Style::default().fg(bar_color)),
                Span::styled(
                    format!(" ${:.2} left", remaining),
                    Style::default().fg(bar_color),
                ),
            ]));
        }

        let projected = app.budget.daily_projected_eod;
        if projected > spent + 0.01 && projected < spent * 5.0 {
            lines.push(Line::from(vec![Span::styled(
                format!("  → ${:.2} by EOD", projected),
                Style::default().fg(m),
            )]));
        }

        // 14-day spend sparkline (newest right)
        let spark_vals: Vec<f64> = {
            let daily = &app
                .global_stats
                .as_ref()
                .map(|g| g.daily.clone())
                .unwrap_or_default();
            let mut v: Vec<f64> = daily
                .iter()
                .rev()
                .take(14)
                .map(|d| d.estimated_cost_usd)
                .collect();
            v.reverse();
            v
        };
        if !spark_vals.is_empty() {
            let spark_w = area.width.saturating_sub(4) as usize;
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    micro_sparkline(&spark_vals, spark_w),
                    Style::default().fg(t.cost_color()),
                ),
            ]));
        }

        f.render_widget(
            Paragraph::new(lines).block(themed_block(t, "Today", false)),
            area,
        );
    } else {
        // No sessions today — show all-time overview so the card is never blank.
        let stats = compute_scoped_stats(app, None, None);

        lines.push(Line::from(vec![
            Span::styled(format!("  {} ", live_icon), Style::default().fg(live_color)),
            Span::styled(
                format!("{}", stats.session_count),
                Style::default()
                    .fg(t.text_primary())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" sessions", Style::default().fg(m)),
            Span::styled(
                format!("   {}t total", fmt_k(stats.total_turns)),
                Style::default().fg(m),
            ),
        ]));

        if stats.total_cost > 0.0 {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    format!("${:.3}", stats.total_cost),
                    Style::default()
                        .fg(t.cost_color())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("  total spend", Style::default().fg(m)),
            ]));
        }

        if stats.avg_cost_per_session > 0.0 {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    format!("${:.3}", stats.avg_cost_per_session),
                    Style::default().fg(t.cost_color()),
                ),
                Span::styled("  avg / session", Style::default().fg(m)),
            ]));
        }

        let trend = week_trend_str(stats.week_cost, stats.prev_week_cost);
        if !trend.is_empty() {
            let trend_col = if trend.starts_with('↑') {
                t.error_color()
            } else if trend.starts_with('↓') {
                t.success_color()
            } else {
                m
            };
            lines.push(Line::from(Span::styled(
                format!("  {}", trend),
                Style::default().fg(trend_col),
            )));
        }

        // 14-day spend sparkline
        let spark_vals: Vec<f64> = {
            let daily = &app
                .global_stats
                .as_ref()
                .map(|g| g.daily.clone())
                .unwrap_or_default();
            let mut v: Vec<f64> = daily
                .iter()
                .rev()
                .take(14)
                .map(|d| d.estimated_cost_usd)
                .collect();
            v.reverse();
            v
        };
        if !spark_vals.is_empty() {
            let spark_w = area.width.saturating_sub(4) as usize;
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    micro_sparkline(&spark_vals, spark_w),
                    Style::default().fg(t.cost_color()),
                ),
            ]));
        }

        f.render_widget(
            Paragraph::new(lines).block(themed_block(t, "All-time", false)),
            area,
        );
    }
}

fn draw_efficiency_card(f: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;

    // Use live session cache rate only if that session actually has token data.
    // Copilot-CLI sessions record 0 tokens, so falling through to global_stats
    // gives the accurate all-time rate computed from sessions that do have tokens.
    let live_has_tokens = app
        .live_stats
        .as_ref()
        .map(|ls| {
            ls.total_input_tokens + ls.total_cache_read_tokens + ls.total_cache_write_tokens > 0
        })
        .unwrap_or(false);

    // Compute all-time cache rate from session_summaries (same source as scoped views).
    let summaries_cache_rate: f64 = {
        let rates: Vec<f64> = app
            .session_summaries
            .values()
            .map(|sm| sm.cache_hit_rate)
            .filter(|&r| r > 0.0)
            .collect();
        if rates.is_empty() {
            0.0
        } else {
            rates.iter().sum::<f64>() / rates.len() as f64
        }
    };

    let (cache_rate, scope_label) = if live_has_tokens {
        (app.live_stats.as_ref().unwrap().cache_hit_rate, "live")
    } else if let Some(gs) = &app.global_stats {
        // Prefer global_stats (weighted by tokens) if it has real data.
        if gs.cache_hit_rate > 0.0 {
            (gs.cache_hit_rate, "all-time")
        } else {
            (summaries_cache_rate, "all-time")
        }
    } else {
        (summaries_cache_rate, "all-time")
    };

    let cache_pct = cache_rate * 100.0;
    let cache_col = t.cache_color(cache_pct);
    let cache_savings = if live_has_tokens {
        app.live_stats
            .as_ref()
            .map(|ls| ls.cache_savings_usd)
            .unwrap_or(0.0)
    } else {
        app.global_stats
            .as_ref()
            .map(|gs| gs.cache_savings_usd)
            .unwrap_or(0.0)
    };

    let bar_w = (area.width.saturating_sub(12) as usize).clamp(8, 16);
    let cache_bar = fill_bar(cache_rate, bar_w);

    let health = app.health_score;
    let hc = t.health_color(health);

    let mut lines: Vec<Line<'static>> = vec![];

    // Line 1: cache bar + % + scope
    lines.push(Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled(cache_bar, Style::default().fg(cache_col)),
        Span::styled(
            format!(" {:.0}%", cache_pct),
            Style::default().fg(cache_col).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            if !scope_label.is_empty() {
                format!("  {}", scope_label)
            } else {
                String::new()
            },
            Style::default().fg(t.muted_color()),
        ),
    ]));

    // Line 2: savings
    if cache_savings > 0.001 {
        lines.push(Line::from(vec![Span::styled(
            format!("  saved ${:.3}", cache_savings),
            Style::default().fg(t.success_color()),
        )]));
    }

    // Line 3: health score + top suggestion
    if health > 0.0 {
        let max_sugg_w = area.width.saturating_sub(12) as usize;
        let mut spans = vec![
            Span::styled("  ⬡ ", Style::default().fg(t.muted_color())),
            Span::styled(
                format!("{:.0}", health),
                Style::default().fg(hc).add_modifier(Modifier::BOLD),
            ),
        ];
        if let Some(s) = app.suggestions.first() {
            spans.push(Span::styled(
                "  ⚡ ",
                Style::default().fg(t.warning_color()),
            ));
            spans.push(Span::styled(
                truncate_with_ellipsis(s.title, max_sugg_w.max(10)),
                Style::default().fg(t.text_secondary()),
            ));
        }
        lines.push(Line::from(spans));
    }

    // Line 4: cache bust warning (when live session degrades)
    if let Some(drop_pct) = app.budget.cache_bust_drop {
        lines.push(Line::from(vec![Span::styled(
            format!("  ⚠ cache drop {:.0}%", drop_pct),
            Style::default().fg(t.warning_color()),
        )]));
    }

    let title = if area.width < 30 {
        "Cache"
    } else {
        "Efficiency"
    };
    f.render_widget(
        Paragraph::new(lines).block(themed_block_borders(
            t,
            title,
            false,
            Borders::TOP | Borders::RIGHT | Borders::BOTTOM,
        )),
        area,
    );
}

fn draw_providers_card(f: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let data = &app.budget.cost_by_provider_model;

    // Aggregate all-time cost by provider
    let mut provider_costs: HashMap<String, f64> = HashMap::new();
    for (prov, _, cost) in data {
        *provider_costs.entry(prov.clone()).or_insert(0.0) += cost;
    }
    let mut providers: Vec<(String, f64)> = provider_costs.into_iter().collect();
    providers.sort_by(|a, b| b.1.total_cmp(&a.1));

    let grand_total: f64 = providers.iter().map(|(_, c)| c).sum();
    let max_rows = area.height.saturating_sub(2) as usize;
    let bar_w = 6usize;
    let name_w = (area.width.saturating_sub(18) as usize).clamp(6, 14);

    let mut lines: Vec<Line<'static>> = vec![];

    for (prov, cost) in providers.iter().take(3) {
        if lines.len() >= max_rows.saturating_sub(1) {
            break;
        }
        let pct = if grand_total > 0.0 {
            cost / grand_total * 100.0
        } else {
            0.0
        };
        let filled = (pct / 100.0 * bar_w as f64) as usize;
        let bar = "█".repeat(filled) + &"░".repeat(bar_w - filled);
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                format!("{:<w$}", truncate_with_ellipsis(prov, name_w), w = name_w),
                Style::default()
                    .fg(t.text_primary())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" ${:.2}", cost),
                Style::default().fg(t.cost_color()),
            ),
            Span::styled(format!("  {}", bar), Style::default().fg(t.accent_dim())),
        ]));
    }

    if providers.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No data yet",
            Style::default().fg(t.muted_color()),
        )));
    }

    f.render_widget(
        Paragraph::new(lines).block(themed_block_borders(
            t,
            "Providers",
            false,
            Borders::TOP | Borders::RIGHT | Borders::BOTTOM,
        )),
        area,
    );
}

// ── Session list ──────────────────────────────────────────────────────────────

fn draw_session_list(f: &mut Frame, app: &App, sessions: &[&Session], area: Rect) {
    let selected = app.selected_session_idx;

    // Reserve one line for predicate hint chips when filter is active and empty.
    let show_filter_hints = app.sessions_filter_active && app.sessions_filter.is_empty();
    let (list_area, hint_area) = if show_filter_hints && area.height > 4 {
        let splits = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(area);
        (splits[0], Some(splits[1]))
    } else {
        (area, None)
    };

    let inner_w = list_area.width.saturating_sub(2) as usize; // subtract borders
    let visible_lines = list_area.height.saturating_sub(2) as usize; // subtract borders

    // ── Build visual rows ─────────────────────────────────────────────────────
    struct VisualEntry {
        lines: Vec<Line<'static>>,
        session_idx: usize,
    }

    let mut entries: Vec<VisualEntry> = Vec::new();

    for (sess_idx, s) in sessions.iter().enumerate() {
        let is_sel = sess_idx == selected;
        let sel_style = if is_sel {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        let muted_style = if is_sel {
            sel_style
        } else {
            Style::default().fg(app.theme.muted_color())
        };

        let summary = app.session_summaries.get(&s.id);
        let cost_str = match summary {
            Some(sm) if sm.estimated_cost_usd > 0.0 => {
                format!("${:.3}", sm.estimated_cost_usd)
            },
            _ => "—".to_string(),
        };
        let cache_pct = summary.map(|sm| sm.cache_hit_rate * 100.0).unwrap_or(0.0);
        let cache_str = if cache_pct > 0.0 {
            format!("{:.0}%", cache_pct)
        } else {
            "—".to_string()
        };
        let bar_w = 8usize;
        let cache_bar = fill_bar(cache_pct / 100.0, bar_w);
        let cache_color = if is_sel {
            app.theme.accent_color()
        } else {
            app.theme.cache_color(cache_pct)
        };

        let model_short = shorten_model(&s.model);
        let branch_str = if !s.git_branch.is_empty() && s.git_branch != "—" {
            format!(" ⎇ {}", truncate_with_ellipsis(&s.git_branch, 12))
        } else {
            String::new()
        };
        let time_ago = session_time_ago(s.started_at);
        let proj_w = inner_w
            .saturating_sub(
                branch_str.chars().count()
                    + time_ago.chars().count()
                    + model_short.chars().count()
                    + 6,
            )
            .clamp(6, 24);
        let proj_short = truncate_with_ellipsis(&s.project_name, proj_w);

        let sel_dot = if app.is_live
            && app
                .live_stats
                .as_ref()
                .and_then(|ls| ls.session.as_ref())
                .map(|ls| ls.id == s.id)
                .unwrap_or(false)
        {
            "◉"
        } else {
            "●"
        };

        // Line 1: ● project ⎇ branch  time  model
        let line1 = Line::from(vec![
            Span::styled(
                format!(" {} ", sel_dot),
                Style::default().fg(if app.is_live {
                    app.theme.success_color()
                } else {
                    app.theme.muted_color()
                }),
            ),
            Span::styled(
                proj_short,
                Style::default()
                    .fg(app.theme.text_primary())
                    .add_modifier(if is_sel {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
            Span::styled(branch_str, Style::default().fg(app.theme.warning_color())),
            Span::styled(format!("  {}  ", time_ago), muted_style),
            Span::styled(model_short, Style::default().fg(app.theme.model_color())),
        ]);

        // Line 2:   $cost  ·  Nt  ·  X% bar
        let turns_str = format!("{}t", s.total_turns);
        let line2 = Line::from(vec![
            Span::styled("   ", Style::default()),
            Span::styled(
                format!("{:<7}", cost_str),
                Style::default().fg(app.theme.cost_color()),
            ),
            Span::styled(" · ", muted_style),
            Span::styled(format!("{:>4}", turns_str), muted_style),
            Span::styled(" · ", muted_style),
            Span::styled(cache_bar, Style::default().fg(cache_color)),
            Span::styled(format!(" {:>4} ", cache_str), muted_style),
        ]);

        // When selected, apply reversed style across the entry lines.
        let (final_l1, final_l2) = if is_sel {
            // Render as a styled background rectangle by wrapping in a single full-width span.
            let w = inner_w;
            fn pad_line(l: Line<'static>, w: usize) -> Line<'static> {
                let content: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
                let pad = w.saturating_sub(content.chars().count());
                let mut spans = l.spans;
                spans.push(Span::styled(
                    " ".repeat(pad),
                    Style::default().add_modifier(Modifier::REVERSED),
                ));
                // Re-apply REVERSED to all spans
                let spans: Vec<Span<'static>> = spans
                    .into_iter()
                    .map(|s| {
                        Span::styled(
                            s.content.into_owned(),
                            s.style.add_modifier(Modifier::REVERSED),
                        )
                    })
                    .collect();
                Line::from(spans)
            }
            (pad_line(line1, w), pad_line(line2, w))
        } else {
            (line1, line2)
        };

        entries.push(VisualEntry {
            lines: vec![final_l1, final_l2],
            session_idx: sess_idx,
        });
    }

    // ── Compute scroll offset so the selected session's lines are visible ─────
    let mut visual_line = 0usize;
    let mut sel_visual_start = 0usize;
    for entry in &entries {
        if entry.session_idx == selected {
            sel_visual_start = visual_line;
        }
        visual_line += entry.lines.len();
    }

    // Scroll so selected session is at the bottom of the visible area.
    let scroll: u16 = if sel_visual_start + 2 > visible_lines {
        (sel_visual_start + 2 - visible_lines) as u16
    } else {
        0
    };

    // Flatten all entries into a single Vec<Line> for the Paragraph.
    let all_lines: Vec<Line<'static>> = entries.into_iter().flat_map(|e| e.lines).collect();

    // ── Block title ───────────────────────────────────────────────────────────
    let filter_suffix = if app.sessions_filter_active {
        if let Some(err) = &app.sessions_filter_error {
            format!("  /{} ⚠ {}", app.sessions_filter, err)
        } else {
            format!("  /{}_", app.sessions_filter)
        }
    } else if !app.sessions_filter.is_empty() {
        format!("  /{}", app.sessions_filter)
    } else {
        String::new()
    };

    let sort_label = app.sessions_sort.label();
    let total = app.sessions_list.len();
    let count_str = if sessions.len() != total {
        format!("({} of {}) ", sessions.len(), total)
    } else if total >= 200 {
        format!("({} max) ", total)
    } else {
        format!("({}) ", sessions.len())
    };

    let scope_suffix = match (&app.scope_provider, &app.scope_model) {
        (Some(p), Some(m)) => format!("  ● {} › {}", p, m),
        (Some(p), None) => format!("  ● {}", p),
        _ => String::new(),
    };

    let title = format!(
        " Sessions {}{}{} [{}] ",
        count_str, filter_suffix, scope_suffix, sort_label
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(app.theme.border_type())
        .border_style(app.theme.active_border_style())
        .title(title);

    f.render_widget(
        Paragraph::new(all_lines).block(block).scroll((scroll, 0)),
        list_area,
    );

    // Filter predicate hint chips — shown below the list when filter is active and empty.
    if let Some(hint_rect) = hint_area {
        let muted = app.theme.muted_color();
        let accent = app.theme.accent_dim();
        let chips = vec![
            Span::styled("  Predicates: ", Style::default().fg(muted)),
            Span::styled("cost>5 ", Style::default().fg(accent)),
            Span::styled("cache<40 ", Style::default().fg(accent)),
            Span::styled("tag:feat ", Style::default().fg(accent)),
            Span::styled("today ", Style::default().fg(accent)),
            Span::styled("anomaly ", Style::default().fg(accent)),
            Span::styled("model:sonnet ", Style::default().fg(accent)),
            Span::styled(" Esc:cancel", Style::default().fg(muted)),
        ];
        f.render_widget(Paragraph::new(Line::from(chips)), hint_rect);
    }
}

// ── Split-panel session preview (right panel) ─────────────────────────────────
//
// Shown when terminal width >= 120 cols. Displays full session stats for the
// currently-selected session. Uses a compact header block plus a turns list below.

fn draw_session_preview_panel(f: &mut Frame, app: &App, area: Rect) {
    let m = app.theme.muted_color();
    let accent = app.theme.accent_color();

    // If no stats loaded yet, show a loading placeholder.
    let Some(stats) = &app.selected_session_stats else {
        let placeholder = Paragraph::new(vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                "  Select a session to preview details",
                Style::default().fg(m),
            )]),
        ])
        .block(
            Block::default()
                .borders(Borders::TOP | Borders::RIGHT | Borders::BOTTOM)
                .border_type(app.theme.border_type())
                .border_style(app.theme.inactive_border_style())
                .title(" Session Detail "),
        );
        f.render_widget(placeholder, area);
        return;
    };

    // Layout: header (up to 9 rows — 3 base + sparkline + ctx + hint + borders) | turns | tools
    let has_tokens = stats.total_input_tokens + stats.total_cache_read_tokens > 0;
    let has_cost = stats.estimated_cost_usd > 0.001;
    let extra_lines = (if has_cost { 1u16 } else { 0 }) + (if has_tokens { 1u16 } else { 0 });
    let header_h = (7u16 + extra_lines).min(area.height / 2);
    // Show tools section below turns when enough height
    let has_tools = app
        .selected_session_tools
        .as_ref()
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    let tools_h = if has_tools && area.height >= 24 {
        5u16
    } else {
        0
    };
    let turns_h = area.height.saturating_sub(header_h + tools_h);

    let constraints: Vec<Constraint> = if tools_h > 0 {
        vec![
            Constraint::Length(header_h),
            Constraint::Min(turns_h),
            Constraint::Length(tools_h),
        ]
    } else {
        vec![Constraint::Length(header_h), Constraint::Min(turns_h)]
    };

    let splits = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let border_style = Style::default().fg(app.theme.accent_dim());
    draw_preview_header(f, app, stats, border_style, splits[0]);

    if turns_h > 2 {
        draw_preview_turns(f, app, stats, accent, splits[1]);
    }
    if tools_h > 0 {
        if let Some(tools) = &app.selected_session_tools {
            draw_tools_compact(f, app, tools, splits[2]);
        }
    }
}

fn draw_preview_header(
    f: &mut Frame,
    app: &App,
    stats: &SessionStats,
    border_style: Style,
    area: Rect,
) {
    let session = stats.session.as_ref();
    let model = session.map(|s| s.model.as_str()).unwrap_or("—");
    let project = session.map(|s| s.project_name.as_str()).unwrap_or("—");
    let branch = session.map(|s| s.git_branch.as_str()).unwrap_or("—");
    let provider = session
        .map(|s| s.provider.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("—");
    let provider_version = session
        .map(|s| s.provider_version.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("");

    let cache_pct = stats.cache_hit_rate * 100.0;
    let cache_bar = fill_bar(stats.cache_hit_rate, 14);
    let cache_col = app.theme.cache_color(cache_pct);

    // True when the provider doesn't export per-turn token breakdowns (e.g. copilot-cli).
    let no_token_data = stats.total_input_tokens == 0
        && stats.total_cache_read_tokens == 0
        && stats.total_cache_write_tokens == 0;

    let title = if !branch.is_empty() && branch != "—" {
        format!(" {} ⎇ {} ", project, branch)
    } else {
        format!(" {} ", project)
    };

    let m = app.theme.muted_color();
    let pv = if !provider_version.is_empty() {
        format!("  {}  {}", provider, provider_version)
    } else {
        format!("  {}", provider)
    };

    let time_str = session
        .map(|s| session_time_ago(s.started_at))
        .unwrap_or_default();

    let line1 = vec![
        Span::styled("  ", Style::default()),
        Span::styled(
            shorten_model(model),
            Style::default()
                .fg(app.theme.model_color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("   ", Style::default().fg(m)),
        Span::styled(
            format!("${:.4}", stats.estimated_cost_usd),
            Style::default()
                .fg(app.theme.cost_color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("   ", Style::default().fg(m)),
        Span::styled(
            format!("{}t", stats.total_turns),
            Style::default().fg(app.theme.text_primary()),
        ),
        Span::styled(pv, Style::default().fg(m)),
        Span::styled(format!("   {}", time_str), Style::default().fg(m)),
    ];

    let line2 = if no_token_data {
        vec![
            Span::styled("  ", Style::default()),
            Span::styled("cache —", Style::default().fg(m)),
            Span::styled(
                "  token data not available for this provider",
                Style::default().fg(m),
            ),
        ]
    } else {
        vec![
            Span::styled("  ", Style::default()),
            Span::styled(cache_bar, Style::default().fg(cache_col)),
            Span::styled(
                format!(" {:.0}%", cache_pct),
                Style::default().fg(cache_col).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  saved ", Style::default().fg(m)),
            Span::styled(
                format!("${:.4}", stats.cache_savings_usd),
                Style::default().fg(app.theme.success_color()),
            ),
            Span::styled("  MCP ", Style::default().fg(m)),
            Span::styled(
                stats.total_mcp_calls.to_string(),
                Style::default().fg(app.theme.warning_color()),
            ),
        ]
    };

    let line3 = vec![
        Span::styled("  ", Style::default()),
        Span::styled(
            if no_token_data {
                "—".to_string()
            } else {
                fmt_k(stats.total_input_tokens)
            },
            Style::default().fg(app.theme.accent_dim()),
        ),
        Span::styled(" in  ", Style::default().fg(m)),
        Span::styled(
            fmt_k(stats.total_output_tokens),
            Style::default().fg(app.theme.accent_color()),
        ),
        Span::styled(" out", Style::default().fg(m)),
        if stats.total_thinking_tokens > 0 {
            Span::styled(
                format!("  {} think", fmt_k(stats.total_thinking_tokens)),
                Style::default().fg(app.theme.cost_color()),
            )
        } else {
            Span::styled("", Style::default())
        },
    ];

    let hint = Line::from(vec![Span::styled(
        "  Enter: fullscreen (Tab → Turns/Context/MCP)  ·  ↑↓: scroll",
        Style::default().fg(m),
    )]);

    // Per-turn cost sparkline (only when there are turns with cost data)
    let has_tokens = stats.total_input_tokens + stats.total_cache_read_tokens > 0;
    let turn_costs: Vec<f64> = stats.turns.iter().map(|t| t.estimated_cost_usd).collect();
    let spark_line = if !turn_costs.is_empty() && stats.estimated_cost_usd > 0.001 {
        let spark_w = area.width.saturating_sub(14) as usize;
        let peak = turn_costs.iter().cloned().fold(0.0_f64, f64::max);
        Some(Line::from(vec![
            Span::styled("  cost/turn ", Style::default().fg(m)),
            Span::styled(
                micro_sparkline(&turn_costs, spark_w.max(4)),
                Style::default().fg(app.theme.cost_color()),
            ),
            Span::styled(
                format!(" ▲${:.4}", peak),
                Style::default().fg(app.theme.muted_color()),
            ),
        ]))
    } else {
        None
    };

    // Context line (only when token data available)
    let ctx_line = if has_tokens {
        let ctx_window = stats
            .session
            .as_ref()
            .and_then(|s| s.context_window_tokens)
            .unwrap_or_else(|| {
                stats
                    .session
                    .as_ref()
                    .map(|s| context_window_for_model(&s.model))
                    .unwrap_or(200_000)
            });
        let peak_ctx = stats
            .turns
            .iter()
            .map(|t| t.input_tokens + t.cache_read_tokens)
            .max()
            .unwrap_or(0);
        let compactions = stats.turns.iter().filter(|t| t.is_compaction_event).count();
        let ctx_pct = if ctx_window > 0 {
            peak_ctx as f64 / ctx_window as f64 * 100.0
        } else {
            0.0
        };
        let ctx_col = app.theme.context_color(ctx_pct);
        let bar_w = 10usize;
        let ctx_bar = fill_bar(ctx_pct / 100.0, bar_w);
        let mut spans = vec![
            Span::styled("  CTX ", Style::default().fg(m)),
            Span::styled(ctx_bar, Style::default().fg(ctx_col)),
            Span::styled(
                format!(" {:.0}% peak", ctx_pct),
                Style::default().fg(ctx_col),
            ),
        ];
        if compactions > 0 {
            spans.push(Span::styled(
                format!("  ⟳ {}×", compactions),
                Style::default().fg(app.theme.warning_color()),
            ));
        }
        Some(Line::from(spans))
    } else {
        None
    };

    let mut lines = vec![Line::from(line1), Line::from(line2), Line::from(line3)];
    if let Some(sl) = spark_line {
        lines.push(sl);
    }
    if let Some(cl) = ctx_line {
        lines.push(cl);
    }
    lines.push(hint);

    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::TOP | Borders::RIGHT | Borders::BOTTOM)
                .border_type(app.theme.border_type())
                .border_style(border_style)
                .title(title),
        ),
        area,
    );
}

fn draw_preview_turns(f: &mut Frame, app: &App, stats: &SessionStats, accent: Color, area: Rect) {
    let m = app.theme.muted_color();
    let border_style = app.theme.inactive_border_style();

    let visible = area.height.saturating_sub(3) as usize;
    let total = stats.turns.len();
    let scroll = app.turn_scroll_detail;

    let hdr_style = Style::default()
        .fg(app.theme.heading_color())
        .add_modifier(Modifier::BOLD);

    let header = Row::new(vec![
        Cell::from("#").style(hdr_style),
        Cell::from("Cost").style(hdr_style),
        Cell::from("Cache%").style(hdr_style),
        Cell::from("In").style(hdr_style),
        Cell::from("Out").style(hdr_style),
        Cell::from("MCP").style(hdr_style),
        Cell::from("ms").style(hdr_style),
    ]);

    let rows: Vec<Row> = stats
        .turns
        .iter()
        .rev()
        .skip(scroll)
        .take(visible)
        .map(|t| {
            let cache_pct = if t.input_tokens + t.cache_read_tokens > 0 {
                t.cache_read_tokens as f64 / (t.input_tokens + t.cache_read_tokens) as f64 * 100.0
            } else {
                0.0
            };
            let cache_col = app.theme.cache_color(cache_pct);
            let ms = t
                .duration_ms
                .map(|d| format!("{}ms", d))
                .unwrap_or_else(|| "—".into());
            Row::new(vec![
                Cell::from(t.turn_index.to_string()).style(Style::default().fg(m)),
                Cell::from(format!("${:.4}", t.estimated_cost_usd))
                    .style(Style::default().fg(app.theme.cost_color())),
                Cell::from(format!("{:.0}%", cache_pct)).style(Style::default().fg(cache_col)),
                Cell::from(fmt_k(t.input_tokens))
                    .style(Style::default().fg(app.theme.accent_dim())),
                Cell::from(fmt_k(t.output_tokens)).style(Style::default().fg(accent)),
                Cell::from(t.mcp_call_count.to_string())
                    .style(Style::default().fg(app.theme.warning_color())),
                Cell::from(ms).style(Style::default().fg(m)),
            ])
        })
        .collect();

    let scroll_hint = if total > visible {
        format!(
            " Recent Turns (showing {}/{}) ",
            (scroll + visible).min(total),
            total
        )
    } else {
        format!(" All Turns ({}) ", total)
    };

    let table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Length(8),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Length(4),
            Constraint::Length(7),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::RIGHT | Borders::BOTTOM)
            .border_type(app.theme.border_type())
            .border_style(border_style)
            .title(scroll_hint),
    );

    f.render_widget(table, area);
}

// ── MCP & Skills section ──────────────────────────────────────────────────────

/// Compact 3-line tools summary for the split-panel preview (gated on height ≥ 24).
fn draw_tools_compact(f: &mut Frame, app: &App, tools: &[ToolBreakdownItem], area: Rect) {
    let m = app.theme.muted_color();
    let accent = app.theme.accent_color();
    let warn = app.theme.warning_color();

    // Group MCPs by server
    let mut mcp_servers: Vec<(String, i64, Vec<String>)> = Vec::new();
    let mut tool_parts: Vec<String> = Vec::new();
    let mut hook_count: i64 = 0;
    let mut skill_count: i64 = 0;
    let mut subagent_count: i64 = 0;

    for item in tools {
        match item.kind.as_str() {
            "mcp" => {
                if let Some(e) = mcp_servers.iter_mut().find(|(s, _, _)| s == &item.server) {
                    e.1 += item.count;
                    if e.2.len() < 3 {
                        e.2.push(format!("{}({})", item.name, item.count));
                    }
                } else {
                    mcp_servers.push((
                        item.server.clone(),
                        item.count,
                        vec![format!("{}({})", item.name, item.count)],
                    ));
                }
            },
            "tool" => {
                if tool_parts.len() < 5 {
                    tool_parts.push(format!("{}({})", item.name, item.count));
                }
            },
            "hook" => hook_count += item.count,
            "skill" => skill_count += item.count,
            "subagent" => subagent_count += item.count,
            _ => {},
        }
    }

    let mut lines: Vec<Line<'static>> = Vec::new();

    // Line 1: MCP servers
    if !mcp_servers.is_empty() {
        let mut spans = vec![Span::styled("  MCP  ", Style::default().fg(m))];
        for (srv, cnt, tool_list) in mcp_servers.iter().take(3) {
            spans.push(Span::styled(
                format!("{}({})", truncate_with_ellipsis(srv, 16), cnt),
                Style::default().fg(accent),
            ));
            if !tool_list.is_empty() {
                spans.push(Span::styled(
                    format!(" [{}]  ", tool_list.join(" ")),
                    Style::default().fg(m),
                ));
            } else {
                spans.push(Span::styled("  ", Style::default()));
            }
        }
        lines.push(Line::from(spans));
    }

    // Line 2: Built-in tools
    if !tool_parts.is_empty() {
        let mut spans = vec![Span::styled("  Tools  ", Style::default().fg(m))];
        spans.push(Span::styled(
            tool_parts.join("  "),
            Style::default().fg(app.theme.text_primary()),
        ));
        lines.push(Line::from(spans));
    }

    // Line 3: Hooks / skills / subagents
    {
        let mut spans = vec![Span::styled("  ", Style::default())];
        if hook_count > 0 {
            spans.push(Span::styled(
                format!("Hooks {}  ", hook_count),
                Style::default().fg(m),
            ));
        }
        if skill_count > 0 {
            spans.push(Span::styled(
                format!("Skills {}  ", skill_count),
                Style::default().fg(warn),
            ));
        }
        if subagent_count > 0 {
            spans.push(Span::styled(
                format!("Subagents {}", subagent_count),
                Style::default().fg(app.theme.accent_color()),
            ));
        }
        if hook_count > 0 || skill_count > 0 || subagent_count > 0 {
            lines.push(Line::from(spans));
        }
    }

    if lines.is_empty() {
        return;
    }

    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::RIGHT | Borders::BOTTOM)
                .border_type(app.theme.border_type())
                .border_style(app.theme.inactive_border_style())
                .title(" MCP & Skills "),
        ),
        area,
    );
}

/// Full MCP & Skills breakdown for the fullscreen detail section.
fn draw_tools_section(f: &mut Frame, app: &App, tools: &[ToolBreakdownItem], area: Rect) {
    let m = app.theme.muted_color();
    let accent = app.theme.accent_color();
    let warn = app.theme.warning_color();
    let sec = app.theme.text_secondary();
    let pri = app.theme.text_primary();

    // Group by kind
    let mut mcp_map: std::collections::BTreeMap<String, Vec<(String, i64)>> =
        std::collections::BTreeMap::new();
    let mut tool_items: Vec<(String, i64)> = Vec::new();
    let mut hook_items: Vec<(String, i64)> = Vec::new();
    let mut skill_items: Vec<(String, i64)> = Vec::new();
    let mut subagent_count: i64 = 0;
    let mut compaction_count: i64 = 0;

    for item in tools {
        match item.kind.as_str() {
            "mcp" => {
                mcp_map
                    .entry(item.server.clone())
                    .or_default()
                    .push((item.name.clone(), item.count));
            },
            "tool" => tool_items.push((item.name.clone(), item.count)),
            "hook" => hook_items.push((item.name.clone(), item.count)),
            "skill" => skill_items.push((item.name.clone(), item.count)),
            "subagent" => subagent_count += item.count,
            "compaction" => compaction_count += item.count,
            _ => {},
        }
    }

    // Sort each tool list by count desc (already ordered from DB, but make explicit)
    tool_items.sort_by_key(|(_, count)| Reverse(*count));
    hook_items.sort_by_key(|(_, count)| Reverse(*count));

    // Build vertical lines
    let mut lines: Vec<Line<'static>> = Vec::new();
    let inner_w = area.width.saturating_sub(4) as usize; // 2 border + 2 indent

    // ── MCP Servers ──────────────────────────────────────────────────────────
    if !mcp_map.is_empty() {
        lines.push(Line::from(Span::styled(
            "  MCP Servers",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        )));
        for (srv, mut tool_list) in mcp_map {
            tool_list.sort_by_key(|(_, count)| Reverse(*count));
            let total: i64 = tool_list.iter().map(|t| t.1).sum();
            lines.push(Line::from(vec![
                Span::styled("    ", Style::default()),
                Span::styled(
                    truncate_with_ellipsis(&srv, inner_w.saturating_sub(12)),
                    Style::default().fg(accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("  {} calls", total), Style::default().fg(m)),
            ]));
            for (i, (name, count)) in tool_list.iter().enumerate() {
                let prefix = if i + 1 < tool_list.len() {
                    "      ├ "
                } else {
                    "      └ "
                };
                lines.push(Line::from(vec![
                    Span::styled(prefix, Style::default().fg(m)),
                    Span::styled(
                        truncate_with_ellipsis(name, inner_w.saturating_sub(16)),
                        Style::default().fg(sec),
                    ),
                    Span::styled(format!("  {}", count), Style::default().fg(m)),
                ]));
            }
        }
        lines.push(Line::from(Span::styled("", Style::default()))); // spacer
    }

    // ── Built-in Tools ───────────────────────────────────────────────────────
    if !tool_items.is_empty() {
        lines.push(Line::from(Span::styled(
            "  Built-in Tools",
            Style::default().fg(pri).add_modifier(Modifier::BOLD),
        )));
        for (name, count) in &tool_items {
            lines.push(Line::from(vec![
                Span::styled("    ", Style::default()),
                Span::styled(
                    truncate_with_ellipsis(name, inner_w.saturating_sub(10)),
                    Style::default().fg(pri),
                ),
                Span::styled(format!("  {}", count), Style::default().fg(m)),
            ]));
        }
        lines.push(Line::from(Span::styled("", Style::default())));
    }

    // ── Hooks ────────────────────────────────────────────────────────────────
    if !hook_items.is_empty() {
        lines.push(Line::from(Span::styled(
            "  Hooks",
            Style::default().fg(m).add_modifier(Modifier::BOLD),
        )));
        for (name, count) in &hook_items {
            lines.push(Line::from(vec![
                Span::styled("    ", Style::default()),
                Span::styled(
                    truncate_with_ellipsis(name, inner_w.saturating_sub(10)),
                    Style::default().fg(m),
                ),
                Span::styled(format!("  {}", count), Style::default().fg(m)),
            ]));
        }
        lines.push(Line::from(Span::styled("", Style::default())));
    }

    // ── Skills ───────────────────────────────────────────────────────────────
    if !skill_items.is_empty() {
        lines.push(Line::from(Span::styled(
            "  Skills",
            Style::default().fg(warn).add_modifier(Modifier::BOLD),
        )));
        for (name, count) in &skill_items {
            lines.push(Line::from(vec![
                Span::styled("    ", Style::default()),
                Span::styled(
                    truncate_with_ellipsis(name, inner_w.saturating_sub(10)),
                    Style::default().fg(warn),
                ),
                Span::styled(format!("  {}", count), Style::default().fg(m)),
            ]));
        }
        lines.push(Line::from(Span::styled("", Style::default())));
    }

    // ── Footer: subagents / compactions ──────────────────────────────────────
    if subagent_count > 0 || compaction_count > 0 {
        let mut spans = vec![Span::styled("  ", Style::default())];
        if subagent_count > 0 {
            spans.push(Span::styled("Subagents  ", Style::default().fg(m)));
            spans.push(Span::styled(
                subagent_count.to_string(),
                Style::default().fg(accent),
            ));
            spans.push(Span::styled("   ", Style::default()));
        }
        if compaction_count > 0 {
            spans.push(Span::styled("Compactions  ", Style::default().fg(m)));
            spans.push(Span::styled(
                compaction_count.to_string(),
                Style::default().fg(warn),
            ));
        }
        lines.push(Line::from(spans));
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No interaction events recorded for this session.",
            Style::default().fg(m),
        )));
    }

    // Scrollable rendering — use turn_scroll_detail as offset
    let border_h = 2u16;
    let visible_h = area.height.saturating_sub(border_h) as usize;
    let total = lines.len();
    let scroll = app.turn_scroll_detail.min(total.saturating_sub(1));
    let visible_lines: Vec<Line<'static>> =
        lines.into_iter().skip(scroll).take(visible_h).collect();

    // Build title with scroll indicator
    let title = if total > visible_h {
        format!(" MCP & Skills  ↑↓ scroll  {}/{} ", scroll + 1, total)
    } else {
        " MCP & Skills ".to_string()
    };

    f.render_widget(
        Paragraph::new(visible_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(app.theme.border_type())
                .border_style(app.theme.inactive_border_style())
                .title(title),
        ),
        area,
    );
}

// ── Context section ───────────────────────────────────────────────────────────

fn draw_context_section(f: &mut Frame, app: &App, stats: &SessionStats, area: Rect) {
    let m = app.theme.muted_color();
    let spark_w = area.width.saturating_sub(6) as usize;

    let has_token_data = stats.total_input_tokens + stats.total_cache_read_tokens > 0;

    if has_token_data {
        draw_context_token_view(f, app, stats, area, m, spark_w);
    } else {
        draw_context_activity_view(f, app, stats, area, m, spark_w);
    }
}

/// Context window fill view — shown when provider reports input/cache token data (e.g. claude-code).
fn draw_context_token_view(
    f: &mut Frame,
    app: &App,
    stats: &SessionStats,
    area: Rect,
    m: Color,
    spark_w: usize,
) {
    let ctx_window = stats
        .session
        .as_ref()
        .and_then(|s| s.context_window_tokens)
        .unwrap_or_else(|| {
            stats
                .session
                .as_ref()
                .map(|s| context_window_for_model(&s.model))
                .unwrap_or(200_000)
        });

    let ctx_series: Vec<f64> = stats
        .turns
        .iter()
        .map(|t| {
            if ctx_window > 0 {
                (t.input_tokens + t.cache_read_tokens) as f64 / ctx_window as f64 * 100.0
            } else {
                0.0
            }
        })
        .collect();

    let peak_ctx_tokens = stats
        .turns
        .iter()
        .map(|t| t.input_tokens + t.cache_read_tokens)
        .max()
        .unwrap_or(0);
    let peak_pct = if ctx_window > 0 {
        peak_ctx_tokens as f64 / ctx_window as f64 * 100.0
    } else {
        0.0
    };
    let ctx_col = app.theme.context_color(peak_pct);
    let compaction_count = stats.turns.iter().filter(|t| t.is_compaction_event).count();
    let cache_write_total: i64 = stats.turns.iter().map(|t| t.cache_write_tokens).sum();
    let provider = stats
        .session
        .as_ref()
        .map(|s| s.provider.as_str())
        .unwrap_or("");

    let peak_bar = fill_bar(peak_pct / 100.0, 18);

    let mut lines: Vec<Line<'static>> = vec![
        Line::from(vec![
            Span::styled("  Peak  ", Style::default().fg(m)),
            Span::styled(peak_bar, Style::default().fg(ctx_col)),
            Span::styled(
                format!(
                    " {:.1}%  ({} / {}k)",
                    peak_pct,
                    fmt_k(peak_ctx_tokens),
                    ctx_window / 1000
                ),
                Style::default().fg(ctx_col).add_modifier(Modifier::BOLD),
            ),
            if compaction_count > 0 {
                Span::styled(
                    format!("   ⟳ compacted {}×", compaction_count),
                    Style::default().fg(app.theme.warning_color()),
                )
            } else {
                Span::styled("", Style::default())
            },
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                micro_sparkline(&ctx_series, spark_w.max(8)),
                Style::default().fg(ctx_col),
            ),
            Span::styled("  ctx% per turn", Style::default().fg(m)),
        ]),
    ];

    if cache_write_total > 0 {
        lines.push(Line::from(vec![
            Span::styled("  Cached  ", Style::default().fg(m)),
            Span::styled(
                format!(
                    "{}k tokens written to prompt cache",
                    cache_write_total / 1000
                ),
                Style::default().fg(app.theme.success_color()),
            ),
        ]));
        if provider != "copilot-cli" {
            lines.push(Line::from(Span::styled(
                "  Tip: new session in same project reloads this cache automatically",
                Style::default().fg(m),
            )));
        }
    }

    if compaction_count > 0 {
        lines.push(Line::from(Span::styled(
            format!(
                "  Context was compacted {}× — session exceeded context window capacity",
                compaction_count
            ),
            Style::default().fg(app.theme.warning_color()),
        )));
    }

    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(app.theme.border_type())
                .border_style(app.theme.inactive_border_style())
                .title(" Context Buildup "),
        ),
        area,
    );
}

/// Activity timeline view — shown when provider doesn't report token data (e.g. copilot-cli).
/// Uses output_tokens (response size), duration_ms (response time), and mcp_call_count.
fn draw_context_activity_view(
    f: &mut Frame,
    app: &App,
    stats: &SessionStats,
    area: Rect,
    m: Color,
    spark_w: usize,
) {
    let t = app.theme;

    if stats.turns.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  No turn data recorded for this session.",
                Style::default().fg(m),
            )))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(t.border_type())
                    .title(" Activity "),
            ),
            area,
        );
        return;
    }

    // Build per-turn series
    let out_series: Vec<f64> = stats.turns.iter().map(|t| t.output_tokens as f64).collect();
    let dur_series: Vec<f64> = stats
        .turns
        .iter()
        .map(|t| t.duration_ms.unwrap_or(0) as f64 / 1000.0)
        .collect();
    let mcp_series: Vec<f64> = stats
        .turns
        .iter()
        .map(|t| t.mcp_call_count as f64)
        .collect();

    // Summary stats
    let peak_output = stats
        .turns
        .iter()
        .map(|t| t.output_tokens)
        .max()
        .unwrap_or(0);
    let avg_output = if stats.turns.is_empty() {
        0
    } else {
        stats.turns.iter().map(|t| t.output_tokens).sum::<i64>() / stats.turns.len() as i64
    };
    let peak_dur_ms = stats
        .turns
        .iter()
        .map(|t| t.duration_ms.unwrap_or(0))
        .max()
        .unwrap_or(0);
    let total_dur_ms: i64 = stats.turns.iter().map(|t| t.duration_ms.unwrap_or(0)).sum();
    let total_mcp: i64 = stats.turns.iter().map(|t| t.mcp_call_count).sum();
    let turn_count = stats.turns.len();

    // Normalise output bar (peak = 100%)
    let out_pct = if peak_output > 0 {
        avg_output as f64 / peak_output as f64
    } else {
        0.0
    };
    let out_bar = fill_bar(out_pct, 14);
    let out_col = t.accent_color();
    let dur_col = t.warning_color();
    let mcp_col = t.cost_color();

    let fmt_dur = |ms: i64| -> String {
        if ms >= 60_000 {
            format!("{:.1}m", ms as f64 / 60_000.0)
        } else {
            format!("{:.1}s", ms as f64 / 1000.0)
        }
    };

    let lines: Vec<Line<'static>> = vec![
        // Row 1: output tokens sparkline
        Line::from(vec![
            Span::styled("  Output  ", Style::default().fg(m)),
            Span::styled(out_bar, Style::default().fg(out_col)),
            Span::styled(
                format!(
                    "  avg {}  peak {}  across {} turns",
                    fmt_k(avg_output),
                    fmt_k(peak_output),
                    turn_count
                ),
                Style::default().fg(out_col).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                micro_sparkline(&out_series, spark_w.max(8)),
                Style::default().fg(out_col),
            ),
            Span::styled("  tokens/turn", Style::default().fg(m)),
        ]),
        // Row 3: response time
        Line::from(vec![
            Span::styled("  Time    ", Style::default().fg(m)),
            Span::styled(
                format!(
                    "avg {}  peak {}  total {}",
                    fmt_dur(if turn_count > 0 {
                        total_dur_ms / turn_count as i64
                    } else {
                        0
                    }),
                    fmt_dur(peak_dur_ms),
                    fmt_dur(total_dur_ms)
                ),
                Style::default().fg(dur_col),
            ),
        ]),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                micro_sparkline(&dur_series, spark_w.max(8)),
                Style::default().fg(dur_col),
            ),
            Span::styled("  seconds/turn", Style::default().fg(m)),
        ]),
        // Row 5: MCP calls (only if any)
        if total_mcp > 0 {
            Line::from(vec![
                Span::styled("  Tools   ", Style::default().fg(m)),
                Span::styled(
                    format!("{} tool calls  ", total_mcp),
                    Style::default().fg(mcp_col).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    micro_sparkline(&mcp_series, spark_w.saturating_sub(20).max(8)),
                    Style::default().fg(mcp_col),
                ),
                Span::styled("  calls/turn", Style::default().fg(m)),
            ])
        } else {
            Line::from(Span::styled("", Style::default()))
        },
    ];

    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(t.border_type())
                .border_style(t.inactive_border_style())
                .title(" Activity Timeline "),
        ),
        area,
    );
}

// ── Trends section (replaces cards when `t` is pressed) ──────────────────────

fn draw_trends_section(f: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let daily = app
        .global_stats
        .as_ref()
        .map(|g| g.daily.clone())
        .unwrap_or_default();

    if daily.is_empty() {
        let p = Paragraph::new("  No daily history yet.").block(themed_block(
            t,
            "Trends  t: back to cards",
            false,
        ));
        f.render_widget(p, area);
        return;
    }

    // Build data newest→oldest then reverse for left=oldest, right=newest
    let mut days: Vec<&scopeon_core::DailyRollup> = daily.iter().rev().take(14).collect();
    days.reverse();

    // ── 3-col layout: Cost | Sessions | Cache% ────────────────────────────────
    let cols = if area.width >= 100 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(38),
                Constraint::Percentage(31),
                Constraint::Percentage(31),
            ])
            .split(area)
    } else {
        // Narrow: two cols
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(area)
    };

    // ── Cost/day BarChart ─────────────────────────────────────────────────────
    let max_cost = days
        .iter()
        .map(|d| d.estimated_cost_usd)
        .fold(0.0_f64, f64::max)
        .max(0.001);
    let cost_bars: Vec<Bar> = days
        .iter()
        .map(|d| {
            let label = if let Ok(date) = chrono::NaiveDate::parse_from_str(&d.date, "%Y-%m-%d") {
                format!("{}/{}", date.format("%m"), date.format("%d"))
            } else {
                d.date[5..].to_string()
            };
            let pct = d.estimated_cost_usd / max_cost;
            let color = if app.budget.daily_limit > 0.0
                && d.estimated_cost_usd >= app.budget.daily_limit * 0.9
            {
                t.error_color()
            } else if pct > 0.7 {
                t.warning_color()
            } else {
                t.cost_color()
            };
            Bar::default()
                .label(label.into())
                .value((d.estimated_cost_usd * 100.0) as u64)
                .style(Style::default().fg(color))
        })
        .collect();

    let cost_chart = BarChart::default()
        .block(themed_block(
            t,
            &format!("Cost/day  max ${:.2}  t: back", max_cost),
            false,
        ))
        .bar_width(2)
        .bar_gap(1)
        .max((max_cost * 100.0) as u64)
        .data(BarGroup::default().bars(&cost_bars));
    f.render_widget(cost_chart, cols[0]);

    // ── Sessions/day BarChart ─────────────────────────────────────────────────
    let max_sess = days
        .iter()
        .map(|d| d.session_count)
        .max()
        .unwrap_or(1)
        .max(1);
    let sess_bars: Vec<Bar> = days
        .iter()
        .map(|d| {
            Bar::default()
                .value(d.session_count as u64)
                .style(Style::default().fg(t.accent_dim()))
        })
        .collect();

    let sess_chart = BarChart::default()
        .block(themed_block_borders(
            t,
            &format!("Sessions/day  max {}", max_sess),
            false,
            Borders::TOP | Borders::RIGHT | Borders::BOTTOM,
        ))
        .bar_width(2)
        .bar_gap(1)
        .max(max_sess as u64)
        .data(BarGroup::default().bars(&sess_bars));
    f.render_widget(sess_chart, cols[1]);

    // ── Cache%/day (only when 3 cols available) ───────────────────────────────
    if cols.len() > 2 {
        // Use session_summaries to derive daily avg cache rate
        let cache_rates: Vec<f64> = days
            .iter()
            .map(|d| {
                let day_sessions: Vec<f64> = app
                    .sessions_list
                    .iter()
                    .filter(|s| {
                        chrono::DateTime::from_timestamp_millis(s.started_at)
                            .map(|dt| {
                                dt.with_timezone(&chrono::Local)
                                    .format("%Y-%m-%d")
                                    .to_string()
                                    == d.date
                            })
                            .unwrap_or(false)
                    })
                    .filter_map(|s| app.session_summaries.get(&s.id))
                    .map(|sm| sm.cache_hit_rate * 100.0)
                    .filter(|&r| r > 0.0)
                    .collect();
                if day_sessions.is_empty() {
                    0.0
                } else {
                    day_sessions.iter().sum::<f64>() / day_sessions.len() as f64
                }
            })
            .collect();

        let cache_bars: Vec<Bar> = cache_rates
            .iter()
            .map(|&r| {
                let color = t.cache_color(r);
                Bar::default()
                    .value(r as u64)
                    .style(Style::default().fg(color))
            })
            .collect();

        let cache_chart = BarChart::default()
            .block(themed_block_borders(
                t,
                "Cache%/day",
                false,
                Borders::TOP | Borders::RIGHT | Borders::BOTTOM,
            ))
            .bar_width(2)
            .bar_gap(1)
            .max(100)
            .data(BarGroup::default().bars(&cache_bars));
        f.render_widget(cache_chart, cols[2]);
    }
}

// ── Full-screen detail mode (Enter key) ──────────────────────────────────────

fn draw_detail_header(
    f: &mut Frame,
    app: &App,
    stats: &SessionStats,
    border_style: Style,
    area: Rect,
) {
    let session = stats.session.as_ref();
    let model = session.map(|s| s.model.as_str()).unwrap_or("—");
    let project = session.map(|s| s.project_name.as_str()).unwrap_or("—");
    let branch = session.map(|s| s.git_branch.as_str()).unwrap_or("—");
    let provider = session
        .map(|s| s.provider.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("—");
    let provider_version = session
        .map(|s| s.provider_version.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("");

    let cache_pct = stats.cache_hit_rate * 100.0;
    let cache_bar = fill_bar(stats.cache_hit_rate, 14);
    let cache_col = app.theme.cache_color(cache_pct);

    // True when the provider doesn't export per-turn token breakdowns (e.g. copilot-cli).
    let no_token_data = stats.total_input_tokens == 0
        && stats.total_cache_read_tokens == 0
        && stats.total_cache_write_tokens == 0;

    let shadow_haiku = shadow_cost(
        model,
        "claude-haiku-4",
        stats.total_input_tokens,
        stats.total_output_tokens,
        stats.total_cache_write_tokens,
        stats.total_cache_read_tokens,
    );

    // Block title: project + branch (primary context at a glance)
    let title = if !branch.is_empty() && branch != "—" {
        format!(" {} ⎇ {} ", project, branch)
    } else {
        format!(" {} ", project)
    };

    let m = app.theme.muted_color();
    let pv = if !provider_version.is_empty() {
        format!("  {}  {}", provider, provider_version)
    } else {
        format!("  {}", provider)
    };

    // Line 1: model · cost · turns · provider
    let mut line1 = vec![
        Span::styled("  ", Style::default()),
        Span::styled(
            shorten_model(model),
            Style::default()
                .fg(app.theme.model_color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("   ", Style::default().fg(m)),
        Span::styled(
            format!("${:.4}", stats.estimated_cost_usd),
            Style::default()
                .fg(app.theme.cost_color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("   ", Style::default().fg(m)),
        Span::styled(
            format!("{}t", stats.total_turns),
            Style::default().fg(app.theme.text_primary()),
        ),
        Span::styled(pv, Style::default().fg(m)),
    ];
    if let Some(h) = shadow_haiku {
        line1.push(Span::styled("   ↓ haiku ", Style::default().fg(m)));
        line1.push(Span::styled(
            format!("${:.4}", h),
            Style::default().fg(app.theme.accent_dim()),
        ));
    }

    // Line 2: cache bar + % + saved + MCP
    let line2 = if no_token_data {
        vec![
            Span::styled("  ", Style::default()),
            Span::styled("cache —", Style::default().fg(m)),
            Span::styled(
                "  token data not available for this provider",
                Style::default().fg(m),
            ),
        ]
    } else {
        vec![
            Span::styled("  ", Style::default()),
            Span::styled(cache_bar, Style::default().fg(cache_col)),
            Span::styled(
                format!(" {:.0}%", cache_pct),
                Style::default().fg(cache_col).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  saved ", Style::default().fg(m)),
            Span::styled(
                format!("${:.4}", stats.cache_savings_usd),
                Style::default().fg(app.theme.success_color()),
            ),
            Span::styled("  MCP ", Style::default().fg(m)),
            Span::styled(
                stats.total_mcp_calls.to_string(),
                Style::default().fg(app.theme.warning_color()),
            ),
        ]
    };

    // Line 3: tokens (input / output / think)
    let line3 = vec![
        Span::styled("  ", Style::default()),
        Span::styled(
            if no_token_data {
                "—".to_string()
            } else {
                fmt_k(stats.total_input_tokens)
            },
            Style::default().fg(app.theme.accent_dim()),
        ),
        Span::styled(" in  ", Style::default().fg(m)),
        Span::styled(
            fmt_k(stats.total_output_tokens),
            Style::default().fg(app.theme.accent_color()),
        ),
        Span::styled(" out", Style::default().fg(m)),
        if stats.total_thinking_tokens > 0 {
            Span::styled(
                format!("  {} think", fmt_k(stats.total_thinking_tokens)),
                Style::default().fg(app.theme.cost_color()),
            )
        } else {
            Span::styled("", Style::default())
        },
    ];

    // Line 4: global health + top suggestion — only when there's insight data
    let show_health = app.health_score > 0.0 || !app.suggestions.is_empty();
    let health_line = if show_health {
        let hc = app.theme.health_color(app.health_score);
        let mut spans = vec![
            Span::styled("  ⬡ ", Style::default().fg(m)),
            Span::styled(
                format!("{:.0}", app.health_score),
                Style::default().fg(hc).add_modifier(Modifier::BOLD),
            ),
        ];
        if let Some(s) = app.suggestions.first() {
            spans.push(Span::styled(
                "  ⚡ ",
                Style::default().fg(app.theme.warning_color()),
            ));
            spans.push(Span::styled(
                truncate_with_ellipsis(s.title, 48),
                Style::default().fg(app.theme.text_secondary()),
            ));
        }
        Some(Line::from(spans))
    } else {
        None
    };

    // Line 5: nav hint — contextual based on active section
    let hint_text = match app.detail_section {
        DetailSection::Turns => "  [ ]: sections  ·  Esc: back  ·  ↑↓: scroll  ·  ← →: replay",
        DetailSection::McpSkills => "  [ ]: sections  ·  Esc: back  ·  ↑↓: scroll",
        DetailSection::Context => "  [ ]: sections  ·  Esc: back",
    };
    let hint_line = Line::from(vec![Span::styled(hint_text, Style::default().fg(m))]);

    let mut lines = vec![Line::from(line1), Line::from(line2), Line::from(line3)];
    if let Some(hl) = health_line {
        lines.push(hl);
    }
    lines.push(hint_line);

    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(app.theme.border_type())
                .border_style(border_style)
                .title(title),
        ),
        area,
    );
}

// ── Full-screen detail mode (Enter key) ──────────────────────────────────────

fn draw_session_detail_fullscreen(f: &mut Frame, app: &App, area: Rect) {
    let Some(stats) = &app.selected_session_stats else {
        let p = Paragraph::new("  No session selected. Press Esc to go back.").block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Session Detail "),
        );
        f.render_widget(p, area);
        return;
    };

    let header_h = 7u16;
    let section_bar_h = 1u16;

    // Layout: header | section bar | content
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_h),
            Constraint::Length(section_bar_h),
            Constraint::Min(0),
        ])
        .split(area);

    draw_detail_header(f, app, stats, app.theme.active_border_style(), v[0]);

    // ── Section selector bar ──────────────────────────────────────────────────
    let m = app.theme.muted_color();
    let acc = app.theme.accent_color();
    let sections = [
        DetailSection::Turns,
        DetailSection::Context,
        DetailSection::McpSkills,
    ];
    let mut bar_spans = vec![Span::styled("  ", Style::default())];
    for sec in &sections {
        let is_active = *sec == app.detail_section;
        let label = format!(" {} ", sec.label());
        let style = if is_active {
            Style::default()
                .fg(acc)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            Style::default().fg(m)
        };
        bar_spans.push(Span::styled(label, style));
        bar_spans.push(Span::styled("   ", Style::default()));
    }
    let bar_hint = match app.detail_section {
        DetailSection::Turns => "  [ ]: sections  ·  Esc: back  ·  ← →: replay",
        DetailSection::McpSkills => "  [ ]: sections  ·  Esc: back  ·  ↑↓: scroll",
        DetailSection::Context => "  [ ]: sections  ·  Esc: back",
    };
    bar_spans.push(Span::styled(bar_hint, Style::default().fg(m)));
    f.render_widget(Paragraph::new(Line::from(bar_spans)), v[1]);

    // ── Section content ───────────────────────────────────────────────────────
    let content_area = v[2];
    match (
        app.detail_section,
        app.replay_turn_idx,
        &app.selected_session_tools,
    ) {
        (DetailSection::Turns, Some(turn_idx), _) => {
            // IS-2: When replay mode is active, show a 3-row snapshot panel above the turn table.
            let replay_h = 3u16;
            let inner = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(replay_h), Constraint::Min(0)])
                .split(content_area);
            let n_turns = stats.turns.len();
            let t_idx = turn_idx.min(n_turns.saturating_sub(1));
            if let Some(t) = stats.turns.get(t_idx) {
                let ctx_pct = (t.input_tokens + t.cache_read_tokens) as f64 / 200_000.0 * 100.0;
                let ctx_color = app.theme.context_color(ctx_pct);
                let muted = app.theme.muted_color();
                let acc = app.theme.accent_color();
                let cum_cost: f64 = stats
                    .turns
                    .iter()
                    .take(t_idx + 1)
                    .map(|t| t.estimated_cost_usd)
                    .sum();
                let snapshot_line = Line::from(vec![
                    Span::styled(
                        format!(" ◈ Turn {} / {}  ", t_idx + 1, n_turns),
                        Style::default().fg(acc).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("│ ", Style::default().fg(muted)),
                    Span::styled(
                        format!("Ctx {:.0}%  ", ctx_pct),
                        Style::default().fg(ctx_color).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(
                            "Input {}K  Cache↓ {}K  Output {}K  ",
                            fmt_k(t.input_tokens),
                            fmt_k(t.cache_read_tokens),
                            fmt_k(t.output_tokens)
                        ),
                        Style::default().fg(muted),
                    ),
                    Span::styled(
                        format!(
                            "Turn ${:.4}  Cumulative ${:.3}  ",
                            t.estimated_cost_usd, cum_cost
                        ),
                        Style::default().fg(app.theme.cost_color()),
                    ),
                    Span::styled("← → scrub  Esc exit replay", Style::default().fg(muted)),
                ]);
                f.render_widget(
                    Paragraph::new(snapshot_line).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_type(app.theme.border_type())
                            .border_style(Style::default().fg(acc))
                            .title(" ◈ Temporal Replay "),
                    ),
                    inner[0],
                );
                draw_replay_turn_table(f, app, stats, t_idx, inner[1]);
            } else {
                draw_fullscreen_turn_table(f, app, stats, None, inner[1]);
            }
        },
        (DetailSection::Turns, None, _) => {
            draw_fullscreen_turn_table(f, app, stats, None, content_area);
        },
        (DetailSection::Context, _, _) => {
            draw_context_section(f, app, stats, content_area);
        },
        (DetailSection::McpSkills, _, Some(tools)) => {
            draw_tools_section(f, app, tools, content_area);
        },
        (DetailSection::McpSkills, _, None) => {
            let p = Paragraph::new("  No interaction events found for this session.").block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" MCP & Skills "),
            );
            f.render_widget(p, content_area);
        },
    }
}

fn draw_replay_turn_table(
    f: &mut Frame,
    app: &App,
    stats: &SessionStats,
    highlighted: usize,
    area: Rect,
) {
    draw_fullscreen_turn_table(f, app, stats, Some(highlighted), area);
}

fn make_turn_row<'a>(
    t: &'a scopeon_core::Turn,
    storage_i: usize,
    is_highlighted: bool,
    app: &'a App,
) -> Row<'a> {
    let ctx_pct = (t.input_tokens + t.cache_read_tokens) as f64 / 200_000.0 * 100.0;
    let ctx_color = app.theme.context_color(ctx_pct);
    let ms = t
        .duration_ms
        .map(|d| format!("{}ms", d))
        .unwrap_or("—".into());
    let base_style = if is_highlighted {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };
    let _ = storage_i; // used by caller for highlight comparison
    Row::new(vec![
        Cell::from(t.turn_index.to_string()).style(base_style.fg(app.theme.muted_color())),
        Cell::from(fmt_k(t.input_tokens)).style(base_style.fg(Color::Blue)),
        Cell::from(fmt_k(t.cache_read_tokens)).style(base_style.fg(app.theme.success_color())),
        Cell::from(fmt_k(t.cache_write_tokens)).style(base_style.fg(app.theme.success_color())),
        Cell::from(fmt_k(t.thinking_tokens)).style(base_style.fg(app.theme.cost_color())),
        Cell::from(fmt_k(t.output_tokens)).style(base_style.fg(app.theme.accent_color())),
        Cell::from(t.mcp_call_count.to_string()).style(base_style.fg(app.theme.warning_color())),
        Cell::from(ms).style(base_style),
        Cell::from(format!("${:.4}", t.estimated_cost_usd))
            .style(base_style.fg(app.theme.cost_color())),
        Cell::from(format!("{:.0}%", ctx_pct)).style(base_style.fg(ctx_color)),
    ])
}

fn draw_fullscreen_turn_table(
    f: &mut Frame,
    app: &App,
    stats: &SessionStats,
    highlight: Option<usize>,
    area: Rect,
) {
    let scroll = app.turn_scroll_detail;
    let hdr_style = Style::default()
        .fg(app.theme.warning_color())
        .add_modifier(Modifier::BOLD);
    let header = Row::new(vec![
        Cell::from("#").style(hdr_style),
        Cell::from("Input").style(hdr_style),
        Cell::from("Cache↓").style(hdr_style),
        Cell::from("Cache↑").style(hdr_style),
        Cell::from("Think").style(hdr_style),
        Cell::from("Output").style(hdr_style),
        Cell::from("MCP").style(hdr_style),
        Cell::from("ms").style(hdr_style),
        Cell::from("Cost").style(hdr_style),
        Cell::from("Context%").style(hdr_style),
    ]);

    // In replay mode: oldest-first so → (increment storage index = newer turn) moves
    // highlight downward, which is the natural "forward in time" direction.
    // In normal mode: newest-first so the most recent turn is visible at top.
    let n = stats.turns.len();
    let rows: Vec<Row> = if highlight.is_some() {
        // Replay: oldest-first. display_i == storage_i.
        stats
            .turns
            .iter()
            .enumerate()
            .skip(scroll)
            .map(|(storage_i, t)| {
                let is_highlighted = highlight.map(|h| h == storage_i).unwrap_or(false);
                make_turn_row(t, storage_i, is_highlighted, app)
            })
            .collect()
    } else {
        // Normal: newest-first.
        stats
            .turns
            .iter()
            .rev()
            .enumerate()
            .skip(scroll)
            .map(|(display_i, t)| {
                let storage_i = n.saturating_sub(1 + display_i);
                make_turn_row(t, storage_i, false, app)
            })
            .collect()
    };

    let title = if highlight.is_some() {
        " All Turns (oldest→newest)  ← prev  → next  Esc return "
    } else {
        " All Turns  → replay  ↑↓ scroll  Esc return  g top  G bottom "
    };

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
            Constraint::Length(9),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(app.theme.border_type())
            .border_style(app.theme.active_border_style())
            .title(title),
    );

    f.render_widget(table, area);
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Human-readable "time ago" string for session list rows.
fn session_time_ago(ts_ms: i64) -> String {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let age_ms = (now_ms - ts_ms).max(0);
    if age_ms < 60_000 {
        "just now".to_string()
    } else if age_ms < 3_600_000 {
        format!("{}m", age_ms / 60_000)
    } else if age_ms < 86_400_000 {
        format!("{}h", age_ms / 3_600_000)
    } else {
        format!("{}d", age_ms / 86_400_000)
    }
}

fn shorten_model(model: &str) -> String {
    if let Some(s) = model.strip_prefix("claude-") {
        let parts: Vec<&str> = s.split('-').collect();
        if parts.len() >= 2 {
            let name = format!("{}-{}", parts[0], parts[1]);
            return truncate_to_chars(&name, 14);
        }
    }
    if model.starts_with("gpt-") {
        return truncate_with_ellipsis(model, 14);
    }
    truncate_with_ellipsis(model, 14)
}

fn fill_bar(ratio: f64, width: usize) -> String {
    let filled = (ratio * width as f64).min(width as f64) as usize;
    "█".repeat(filled) + &"░".repeat(width - filled)
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── fill_bar ──────────────────────────────────────────────────────────────

    #[test]
    fn fill_bar_zero_ratio_all_empty() {
        let bar = fill_bar(0.0, 10);
        assert_eq!(bar.chars().count(), 10);
        assert!(
            bar.chars().all(|c| c == '░'),
            "0.0 ratio must be all empty: {bar}"
        );
    }

    #[test]
    fn fill_bar_full_ratio_all_filled() {
        let bar = fill_bar(1.0, 10);
        assert_eq!(bar.chars().count(), 10);
        assert!(
            bar.chars().all(|c| c == '█'),
            "1.0 ratio must be all filled: {bar}"
        );
    }

    #[test]
    fn fill_bar_half_ratio_half_filled() {
        let bar = fill_bar(0.5, 10);
        assert_eq!(bar.chars().count(), 10);
        let filled = bar.chars().filter(|&c| c == '█').count();
        let empty = bar.chars().filter(|&c| c == '░').count();
        assert_eq!(filled, 5, "half ratio: expected 5 filled, got {filled}");
        assert_eq!(empty, 5, "half ratio: expected 5 empty, got {empty}");
    }

    #[test]
    fn fill_bar_over_one_clamps_to_full() {
        // ratio > 1.0 must clamp — bar must not panic or produce wrong length
        let bar = fill_bar(1.5, 8);
        assert_eq!(
            bar.chars().count(),
            8,
            "over-ratio bar must still be exactly width chars"
        );
        assert!(
            bar.chars().all(|c| c == '█'),
            "clamped ratio must be all filled: {bar}"
        );
    }

    #[test]
    fn fill_bar_width_zero_returns_empty_string() {
        let bar = fill_bar(0.5, 0);
        assert_eq!(bar, "", "width=0 must return empty string");
    }

    #[test]
    fn fill_bar_width_one_filled() {
        let bar = fill_bar(1.0, 1);
        assert_eq!(bar, "█");
    }

    #[test]
    fn fill_bar_width_one_empty() {
        let bar = fill_bar(0.0, 1);
        assert_eq!(bar, "░");
    }

    #[test]
    fn fill_bar_total_chars_always_equals_width() {
        for width in [0, 1, 5, 10, 20, 40] {
            for ratio_tenth in [0, 1, 3, 5, 7, 9, 10, 15] {
                let ratio = ratio_tenth as f64 / 10.0;
                let bar = fill_bar(ratio, width);
                assert_eq!(
                    bar.chars().count(),
                    width,
                    "fill_bar({ratio}, {width}).chars().count() != {width}"
                );
            }
        }
    }

    // ── fmt_k ─────────────────────────────────────────────────────────────────

    #[test]
    fn fmt_k_zero_returns_zero_string() {
        assert_eq!(fmt_k(0), "0");
    }

    #[test]
    fn fmt_k_below_thousand_returns_plain_number() {
        assert_eq!(fmt_k(999), "999");
        assert_eq!(fmt_k(1), "1");
        assert_eq!(fmt_k(100), "100");
    }

    #[test]
    fn fmt_k_exactly_thousand_returns_k_notation() {
        let result = fmt_k(1000);
        assert!(result.ends_with('k'), "1000 must show as xk: {result}");
        assert!(!result.is_empty());
    }

    #[test]
    fn fmt_k_never_empty_for_any_nonnegative_input() {
        for n in [
            0_i64,
            1,
            999,
            1000,
            1001,
            999_999,
            1_000_000,
            1_500_000,
            i64::MAX / 2,
        ] {
            let result = fmt_k(n);
            assert!(!result.is_empty(), "fmt_k({n}) returned empty string");
        }
    }

    #[test]
    fn fmt_k_million_uses_m_notation() {
        let result = fmt_k(1_000_000);
        assert!(result.ends_with('M'), "1M must use M notation: {result}");
    }

    #[test]
    fn fmt_k_1_5_million() {
        let result = fmt_k(1_500_000);
        assert_eq!(result, "1.5M");
    }

    #[test]
    fn fmt_k_thresholds_are_consistent() {
        // 999 → plain, 1000 → k notation
        let below = fmt_k(999);
        let above = fmt_k(1000);
        assert!(!below.contains('k'), "999 must not use k: {below}");
        assert!(above.contains('k'), "1000 must use k: {above}");
    }

    // ── shorten_model ─────────────────────────────────────────────────────────

    #[test]
    fn shorten_model_never_exceeds_14_chars() {
        let models = [
            "claude-opus-4-5-20251101",
            "claude-sonnet-4-5-20251201",
            "claude-haiku-4-5-20251001",
            "gpt-4o-mini-2024-07-18",
            "gemini-1.5-pro-latest",
            "a-very-long-model-name-that-exceeds-limits",
        ];
        for model in models {
            let result = shorten_model(model);
            assert!(
                result.chars().count() <= 14,
                "shorten_model({model:?}) = {result:?} exceeds 14 chars"
            );
        }
    }

    #[test]
    fn shorten_model_strips_claude_prefix() {
        // "claude-opus-4-5..." → "opus-4" (prefix stripped)
        let result = shorten_model("claude-opus-4-5-20251101");
        assert!(
            !result.starts_with("claude-"),
            "should strip 'claude-' prefix: {result}"
        );
    }

    #[test]
    fn shorten_model_never_empty_for_any_nonempty_input() {
        let models = ["claude-sonnet-4-5", "gpt-4o", "unknown-model-xyz", "x"];
        for model in models {
            let result = shorten_model(model);
            assert!(
                !result.is_empty(),
                "shorten_model({model:?}) returned empty string"
            );
        }
    }

    #[test]
    fn shorten_model_gpt_prefix_preserved_up_to_14_chars() {
        let result = shorten_model("gpt-4o");
        assert!(
            result.starts_with("gpt-"),
            "gpt prefix should be preserved: {result}"
        );
        assert!(result.chars().count() <= 14);
    }

    // ── week_trend_str ────────────────────────────────────────────────────────

    #[test]
    fn week_trend_str_zero_both_returns_empty() {
        let result = week_trend_str(0.0, 0.0);
        assert_eq!(
            result, "",
            "zero/zero should return empty string, got: {result:?}"
        );
    }

    #[test]
    fn week_trend_str_new_this_week_no_prev() {
        let result = week_trend_str(5.0, 0.0);
        assert!(
            !result.is_empty(),
            "new spend with no prev week must return non-empty string"
        );
        assert!(result.contains("new"), "should mention 'new': {result}");
    }

    #[test]
    fn week_trend_str_big_increase_shows_up_arrow() {
        let result = week_trend_str(100.0, 50.0); // +100%
        assert!(result.contains('↑'), "100% increase must show ↑: {result}");
        assert!(!result.is_empty());
    }

    #[test]
    fn week_trend_str_big_decrease_shows_down_arrow() {
        let result = week_trend_str(10.0, 100.0); // -90%
        assert!(result.contains('↓'), "90% decrease must show ↓: {result}");
        assert!(!result.is_empty());
    }

    #[test]
    fn week_trend_str_stable_shows_stable_indicator() {
        let result = week_trend_str(10.0, 10.5); // ~5% delta — stable
        assert!(
            result.contains('≈') || result.to_lowercase().contains("stable"),
            "small delta must show stable indicator: {result}"
        );
    }

    #[test]
    fn week_trend_str_never_panics_on_extreme_values() {
        // Should not panic on large/small floats
        let _ = week_trend_str(f64::MAX / 2.0, 1.0);
        let _ = week_trend_str(0.0, f64::MAX / 2.0);
        let _ = week_trend_str(1.0, f64::MIN_POSITIVE);
    }
}
