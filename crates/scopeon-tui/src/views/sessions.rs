//! Tab 1: Sessions — live banner + interactive master/detail session browser.
//!
//! Top: compact live session banner when a session is active.
//! Left panel: scrollable session list (newest first), selectable with ↑↓.
//! Right panel: selected session detail (key stats + turn table).
//! Enter: full-screen session detail mode.
//! /: filter sessions. s: cycle sort order.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Frame,
};

use scopeon_core::{shadow_cost, Session, SessionStats};

use crate::app::{App, PaneFocus};
use crate::text::{truncate_to_chars, truncate_with_ellipsis};
use crate::views::components::{empty_state_lines, themed_block};

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    // Full-screen detail mode (Enter key)
    if app.session_detail_mode {
        draw_session_detail_fullscreen(f, app, area);
        return;
    }

    // Live session banner — compact strip at top when a session is active
    let area = if app.is_live && area.height > 5 {
        let v = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(area);
        draw_live_banner(f, app, v[0]);
        v[1]
    } else {
        area
    };

    let sessions = app.filtered_sessions();

    if sessions.is_empty() && !app.sessions_filter_active {
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

    // Chip rows: provider row when ≥2 providers, model row when provider scoped and ≥2 models.
    let show_provider_chips = app.all_providers.len() >= 2;
    let show_model_chips = app.scope_provider.is_some() && app.all_models.len() >= 2;
    let chip_rows = (show_provider_chips as u16) + (show_model_chips as u16);

    let main_area = if chip_rows > 0 && area.height > chip_rows + 4 {
        let v = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(chip_rows), Constraint::Min(0)])
            .split(area);
        draw_filter_chips(f, app, v[0], show_provider_chips, show_model_chips);
        v[1]
    } else {
        area
    };

    // C-05: Proportional list width — 38% on wide terminals, fixed 44 on narrow.
    let list_w = if main_area.width >= 100 {
        (main_area.width as f32 * 0.38) as u16
    } else {
        44u16
    };

    let h = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(list_w), Constraint::Min(0)])
        .split(main_area);

    draw_session_list(f, app, &sessions, h[0]);
    draw_session_detail(f, app, h[1]);
}

// ── Provider / model chip navigation row ─────────────────────────────────────

/// Renders 1–2 chip rows showing provider and model scope selection.
/// Keys `[`/`]` cycle providers; `{`/`}` cycle models.
fn draw_filter_chips(
    f: &mut Frame,
    app: &App,
    area: Rect,
    show_providers: bool,
    show_models: bool,
) {
    let t = app.theme;
    let mut lines: Vec<Line<'static>> = Vec::new();

    if show_providers {
        lines.push(build_chip_row(
            "Provider",
            &app.all_providers,
            &app.scope_provider,
            t,
            "  ]  next   [  prev",
            app.scope_provider.is_some() || app.scope_model.is_some(),
        ));
    }
    if show_models {
        lines.push(build_chip_row(
            "Model   ",
            &app.all_models,
            &app.scope_model,
            t,
            "  }  next   {  prev",
            false,
        ));
    }

    f.render_widget(Paragraph::new(lines), area);
}

fn build_chip_row(
    label: &'static str,
    options: &[String],
    current: &Option<String>,
    t: crate::theme::Theme,
    nav_hint: &'static str,
    show_esc: bool,
) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = vec![
        Span::styled(format!("  ◈ {} ", label), Style::default().fg(t.muted_color())),
    ];

    // "All" chip
    if current.is_none() {
        spans.push(Span::styled(
            "● All".to_string(),
            Style::default().fg(t.heading_color()).add_modifier(Modifier::BOLD),
        ));
    } else {
        spans.push(Span::styled("○ All".to_string(), Style::default().fg(t.muted_color())));
    }

    // Option chips
    for opt in options {
        spans.push(Span::styled("  ".to_string(), Style::default()));
        let is_sel = current.as_deref() == Some(opt.as_str());
        let display = shorten_model(opt);
        if is_sel {
            spans.push(Span::styled(
                format!("● {}", display),
                Style::default().fg(t.accent_color()).add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                format!("○ {}", display),
                Style::default().fg(t.muted_color()),
            ));
        }
    }

    // Navigation hint
    spans.push(Span::styled(nav_hint.to_string(), Style::default().fg(t.muted_color())));
    if show_esc {
        spans.push(Span::styled("   Esc all".to_string(), Style::default().fg(t.muted_color())));
    }

    Line::from(spans)
}

// ── Left panel: session list ──────────────────────────────────────────────────
//
// C-05: 2-line rich rows (project + time + model / cost + turns + cache bar).

fn draw_session_list(f: &mut Frame, app: &App, sessions: &[&Session], area: Rect) {
    let is_focused = app.pane_focus == PaneFocus::Left;
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
        let sel_style = if is_sel && is_focused {
            Style::default().add_modifier(Modifier::REVERSED)
        } else if is_sel {
            Style::default().fg(app.theme.accent_color()).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let muted_style = if is_sel && is_focused {
            sel_style
        } else {
            Style::default().fg(app.theme.muted_color())
        };

        let summary = app.session_summaries.get(&s.id);
        let cost_str = match summary {
            Some(sm) if sm.estimated_cost_usd > 0.0 => {
                format!("${:.3}", sm.estimated_cost_usd)
            }
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
        let cache_color = if is_sel && is_focused {
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
            .saturating_sub(branch_str.chars().count() + time_ago.chars().count() + model_short.chars().count() + 6)
            .clamp(6, 24);
        let proj_short = truncate_with_ellipsis(&s.project_name, proj_w);

        let sel_dot = if app.is_live
            && app.live_stats.as_ref().and_then(|ls| ls.session.as_ref()).map(|ls| ls.id == s.id).unwrap_or(false)
        {
            "◉"
        } else {
            "●"
        };

        // Line 1: ● project ⎇ branch  time  model
        let line1 = Line::from(vec![
            Span::styled(
                format!(" {} ", sel_dot),
                Style::default().fg(if is_sel && !is_focused {
                    app.theme.accent_color()
                } else if app.is_live {
                    app.theme.success_color()
                } else {
                    app.theme.muted_color()
                }),
            ),
            Span::styled(
                proj_short,
                Style::default()
                    .fg(app.theme.text_primary())
                    .add_modifier(if is_sel { Modifier::BOLD } else { Modifier::empty() }),
            ),
            Span::styled(branch_str, Style::default().fg(app.theme.warning_color())),
            Span::styled(
                format!("  {}  ", time_ago),
                muted_style,
            ),
            Span::styled(
                model_short,
                Style::default().fg(app.theme.model_color()),
            ),
        ]);

        // Line 2:   $cost  ·  Nt  ·  Cache X% bar
        let turns_str = format!("{}t", s.total_turns);
        let line2 = Line::from(vec![
            Span::styled("   ", Style::default()),
            Span::styled(
                format!("{:<7}", cost_str),
                Style::default().fg(app.theme.cost_color()),
            ),
            Span::styled(" · ", muted_style),
            Span::styled(
                format!("{:>4}", turns_str),
                muted_style,
            ),
            Span::styled(" · ", muted_style),
            Span::styled(
                format!("Cache {:>4} ", cache_str),
                muted_style,
            ),
            Span::styled(cache_bar, Style::default().fg(cache_color)),
        ]);

        // When selected, apply reversed style across the entry lines.
        let (final_l1, final_l2) = if is_sel && is_focused {
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
                    .map(|s| Span::styled(s.content.into_owned(), s.style.add_modifier(Modifier::REVERSED)))
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
    let all_lines: Vec<Line<'static>> = entries
        .into_iter()
        .flat_map(|e| e.lines)
        .collect();

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

    let border_style = if is_focused {
        app.theme.active_border_style()
    } else {
        app.theme.inactive_border_style()
    };

    let title = format!(
        " Sessions {}{} [{}] ",
        count_str, filter_suffix, sort_label
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(app.theme.border_type())
        .border_style(border_style)
        .title(title);

    f.render_widget(
        Paragraph::new(all_lines)
            .block(block)
            .scroll((scroll, 0)),
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

// ── Right panel: selected session detail ──────────────────────────────────────

fn draw_session_detail(f: &mut Frame, app: &App, area: Rect) {
    let is_focused = app.pane_focus == PaneFocus::Right;
    let border_style = if is_focused {
        app.theme.active_border_style()
    } else {
        app.theme.inactive_border_style()
    };

    let Some(stats) = &app.selected_session_stats else {
        // No session selected or stats not loaded yet
        if app.sessions_list.is_empty() {
            let p = Paragraph::new("  Select a session with ↑↓").block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(app.theme.border_type())
                    .border_style(border_style)
                    .title(" Session Detail "),
            );
            f.render_widget(p, area);
        } else {
            let p = Paragraph::new("  Loading…").block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(app.theme.border_type())
                    .border_style(border_style)
                    .title(" Session Detail "),
            );
            f.render_widget(p, area);
        }
        return;
    };

    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(11), Constraint::Min(0)])
        .split(area);

    draw_detail_header(f, app, stats, border_style, v[0]);
    draw_detail_turns(f, app, stats, border_style, v[1]);
}

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
        .unwrap_or("—");
    let interaction_count = app.selected_session_interaction_events.len();
    let task_count = app.selected_session_task_runs.len();
    let skill_count = app
        .selected_session_interaction_events
        .iter()
        .filter(|event| event.kind == "skill")
        .count();
    let hook_count = app
        .selected_session_interaction_events
        .iter()
        .filter(|event| event.kind == "hook")
        .count();
    let recent_tasks = app
        .selected_session_task_runs
        .iter()
        .rev()
        .take(2)
        .map(|task| {
            task.display_name
                .as_deref()
                .unwrap_or(&task.name)
                .to_string()
        })
        .collect::<Vec<_>>()
        .join(", ");

    let cache_pct = stats.cache_hit_rate * 100.0;
    let cache_bar = fill_bar(stats.cache_hit_rate, 16);
    let cache_col = app.theme.cache_color(cache_pct);

    // IS-I: Compute shadow costs for Haiku and Sonnet comparisons.
    // Use the session model and aggregate token counts.
    let shadow_haiku = shadow_cost(
        model,
        "claude-haiku-4",
        stats.total_input_tokens,
        stats.total_output_tokens,
        stats.total_cache_write_tokens,
        stats.total_cache_read_tokens,
    );
    let shadow_sonnet = shadow_cost(
        model,
        "claude-sonnet-4",
        stats.total_input_tokens,
        stats.total_output_tokens,
        stats.total_cache_write_tokens,
        stats.total_cache_read_tokens,
    );

    let lines = vec![
        Line::from(vec![
            Span::styled("  Model:   ", Style::default().fg(app.theme.muted_color())),
            Span::styled(
                shorten_model(model),
                Style::default()
                    .fg(app.theme.model_color())
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Project: ", Style::default().fg(app.theme.muted_color())),
            Span::styled(
                project.to_string(),
                Style::default().fg(app.theme.text_primary()),
            ),
            Span::styled("  Branch: ", Style::default().fg(app.theme.muted_color())),
            Span::styled(
                branch.to_string(),
                Style::default().fg(app.theme.warning_color()),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Provider: ", Style::default().fg(app.theme.muted_color())),
            Span::styled(
                provider.to_string(),
                Style::default().fg(app.theme.text_primary()),
            ),
            Span::styled("  Version: ", Style::default().fg(app.theme.muted_color())),
            Span::styled(
                provider_version.to_string(),
                Style::default().fg(app.theme.accent_color()),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Turns:  ", Style::default().fg(app.theme.muted_color())),
            Span::styled(
                stats.total_turns.to_string(),
                Style::default().fg(app.theme.text_primary()),
            ),
            Span::styled("   Cost: ", Style::default().fg(app.theme.muted_color())),
            Span::styled(
                format!("${:.4}", stats.estimated_cost_usd),
                Style::default()
                    .fg(app.theme.cost_color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("   Saved: ", Style::default().fg(app.theme.muted_color())),
            Span::styled(
                format!("${:.4}", stats.cache_savings_usd),
                Style::default().fg(app.theme.success_color()),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Cache:  ", Style::default().fg(app.theme.muted_color())),
            Span::styled(cache_bar, Style::default().fg(cache_col)),
            Span::styled(
                format!(" {:.1}%", cache_pct),
                Style::default().fg(cache_col),
            ),
            Span::styled("  MCP: ", Style::default().fg(app.theme.muted_color())),
            Span::styled(
                stats.total_mcp_calls.to_string(),
                Style::default().fg(app.theme.warning_color()),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Input:  ", Style::default().fg(app.theme.muted_color())),
            Span::styled(
                fmt_k(stats.total_input_tokens),
                Style::default().fg(app.theme.accent_dim()),
            ),
            Span::styled("   Think: ", Style::default().fg(app.theme.muted_color())),
            Span::styled(
                fmt_k(stats.total_thinking_tokens),
                Style::default().fg(app.theme.cost_color()),
            ),
            Span::styled("   Output: ", Style::default().fg(app.theme.muted_color())),
            Span::styled(
                fmt_k(stats.total_output_tokens),
                Style::default().fg(app.theme.accent_color()),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "  Provenance: ",
                Style::default().fg(app.theme.muted_color()),
            ),
            Span::styled(
                format!("{} interactions", interaction_count),
                Style::default().fg(app.theme.text_primary()),
            ),
            Span::styled("  Tasks: ", Style::default().fg(app.theme.muted_color())),
            Span::styled(
                task_count.to_string(),
                Style::default().fg(app.theme.warning_color()),
            ),
            Span::styled("  Skills: ", Style::default().fg(app.theme.muted_color())),
            Span::styled(
                skill_count.to_string(),
                Style::default().fg(app.theme.success_color()),
            ),
            Span::styled("  Hooks: ", Style::default().fg(app.theme.muted_color())),
            Span::styled(
                hook_count.to_string(),
                Style::default().fg(app.theme.cost_color()),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                "  Recent tasks: ",
                Style::default().fg(app.theme.muted_color()),
            ),
            Span::styled(
                if recent_tasks.is_empty() {
                    "—".to_string()
                } else {
                    recent_tasks
                },
                Style::default().fg(app.theme.text_primary()),
            ),
        ]),
        Line::from(vec![Span::styled(
            "  [Enter] full-screen detail  [Tab] switch pane",
            Style::default().fg(app.theme.muted_color()),
        )]),
    ];

    // IS-I: Append shadow pricing rows only when relevant comparisons exist.
    let mut all_lines = lines;
    if shadow_haiku.is_some() || shadow_sonnet.is_some() {
        let mut shadow_spans = vec![Span::styled(
            "  Shadow: ",
            Style::default().fg(app.theme.muted_color()),
        )];
        if let Some(h) = shadow_haiku {
            shadow_spans.push(Span::styled(
                "Haiku ",
                Style::default().fg(app.theme.muted_color()),
            ));
            shadow_spans.push(Span::styled(
                format!("${:.4}", h),
                Style::default().fg(app.theme.accent_color()),
            ));
        }
        if let Some(s) = shadow_sonnet {
            shadow_spans.push(Span::styled(
                "  Sonnet ",
                Style::default().fg(app.theme.muted_color()),
            ));
            shadow_spans.push(Span::styled(
                format!("${:.4}", s),
                Style::default().fg(app.theme.accent_color()),
            ));
        }
        all_lines.insert(all_lines.len() - 1, Line::from(shadow_spans));
    }

    f.render_widget(
        Paragraph::new(all_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(app.theme.border_type())
                .border_style(border_style)
                .title(" Session Detail "),
        ),
        area,
    );
}

fn draw_detail_turns(
    f: &mut Frame,
    app: &App,
    stats: &SessionStats,
    border_style: Style,
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
        Cell::from("Think").style(hdr_style),
        Cell::from("Output").style(hdr_style),
        Cell::from("MCP").style(hdr_style),
        Cell::from("Cost").style(hdr_style),
    ]);

    let rows: Vec<Row> = stats
        .turns
        .iter()
        .rev()
        .skip(scroll)
        .map(|t| {
            Row::new(vec![
                Cell::from(t.turn_index.to_string())
                    .style(Style::default().fg(app.theme.muted_color())),
                Cell::from(fmt_k(t.input_tokens)).style(Style::default().fg(app.theme.accent_dim())),
                Cell::from(fmt_k(t.cache_read_tokens))
                    .style(Style::default().fg(app.theme.success_color())),
                Cell::from(fmt_k(t.thinking_tokens))
                    .style(Style::default().fg(app.theme.cost_color())),
                Cell::from(fmt_k(t.output_tokens))
                    .style(Style::default().fg(app.theme.accent_color())),
                Cell::from(t.mcp_call_count.to_string())
                    .style(Style::default().fg(app.theme.warning_color())),
                Cell::from(format!("${:.4}", t.estimated_cost_usd))
                    .style(Style::default().fg(app.theme.cost_color())),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Length(7),
            Constraint::Length(8),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Length(4),
            Constraint::Length(9),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(app.theme.border_type())
            .border_style(border_style)
            .title(" Turns (newest first)  ↑↓ scroll "),
    );

    f.render_widget(table, area);
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

    // IS-2: When replay mode is active, show a 3-row snapshot panel above the turn table.
    let (header_h, replay_h) = if app.replay_turn_idx.is_some() {
        (11, 3)
    } else {
        (11, 0)
    };

    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if replay_h > 0 {
            vec![
                Constraint::Length(header_h),
                Constraint::Length(replay_h),
                Constraint::Min(0),
            ]
        } else {
            vec![Constraint::Length(header_h), Constraint::Min(0)]
        })
        .split(area);

    draw_detail_header(f, app, stats, app.theme.active_border_style(), v[0]);

    // IS-2: Replay snapshot panel
    if let Some(turn_idx) = app.replay_turn_idx {
        let turn_area = v[1];
        let table_area = v[2];
        let n_turns = stats.turns.len();
        // turns are stored oldest-first in stats.turns
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
                turn_area,
            );
            draw_replay_turn_table(f, app, stats, t_idx, table_area);
        } else {
            draw_fullscreen_turn_table(f, app, stats, None, table_area);
        }
    } else {
        draw_fullscreen_turn_table(f, app, stats, None, v[1]);
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

// ── Live session banner ───────────────────────────────────────────────────────

/// Renders a compact 1-line banner at the top of the Sessions tab when a
/// session is actively running. Shows project, branch, cost, turns, ctx%, cache%.
fn draw_live_banner(f: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;
    let session = app.live_stats.as_ref().and_then(|ls| ls.session.as_ref());

    let project = session
        .map(|s| truncate_with_ellipsis(&s.project_name, 18))
        .unwrap_or_default();
    let branch = session
        .filter(|s| !s.git_branch.is_empty() && s.git_branch != "—")
        .map(|s| format!(" ⎇ {}", truncate_with_ellipsis(&s.git_branch, 10)))
        .unwrap_or_default();
    let model = session.map(|s| shorten_model(&s.model)).unwrap_or_default();

    let cost = app.budget.daily_spent;
    let ctx_pct = app.budget.context_pressure_pct;
    let cache_pct = app
        .live_stats
        .as_ref()
        .map(|s| s.cache_hit_rate * 100.0)
        .unwrap_or(0.0);
    let turns = session.map(|s| s.total_turns).unwrap_or(0);
    let w = area.width as usize;

    let base_w = 10 + project.len() + branch.len();

    let mut spans: Vec<Span<'static>> = vec![
        Span::styled(
            "  ◉ LIVE  ".to_string(),
            Style::default()
                .fg(t.success_color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            project.clone(),
            Style::default()
                .fg(t.text_primary())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(branch.clone(), Style::default().fg(t.warning_color())),
    ];

    if w > base_w + 15 {
        spans.push(Span::styled(
            format!("  ${:.3}", cost),
            Style::default().fg(t.cost_color()),
        ));
        spans.push(Span::styled(
            format!("  {}t", turns),
            Style::default().fg(t.muted_color()),
        ));
    }
    if w > base_w + 32 {
        spans.push(Span::styled(
            format!("  ctx {:.0}%", ctx_pct),
            Style::default().fg(t.context_color(ctx_pct)),
        ));
        spans.push(Span::styled(
            format!("  cache {:.0}%", cache_pct),
            Style::default().fg(t.cache_color(cache_pct)),
        ));
    }
    if w > base_w + 50 && !model.is_empty() {
        spans.push(Span::styled(
            format!("  {}", model),
            Style::default().fg(t.model_color()),
        ));
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
