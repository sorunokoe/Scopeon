//! Tab 2: Sessions — interactive master/detail session browser.
//!
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

use scopeon_core::{branch_to_tag, shadow_cost, Session, SessionStats};

use crate::app::{App, PaneFocus};
use crate::text::{truncate_to_chars, truncate_with_ellipsis};
use crate::views::components::{empty_state_lines, themed_block};

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    // Full-screen detail mode (Enter key)
    if app.session_detail_mode {
        draw_session_detail_fullscreen(f, app, area);
        return;
    }

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

    let h = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(46), Constraint::Min(0)])
        .split(area);

    draw_session_list(f, app, &sessions, h[0]);
    draw_session_detail(f, app, h[1]);
}

// ── Left panel: session list ──────────────────────────────────────────────────

fn draw_session_list(f: &mut Frame, app: &App, sessions: &[&Session], area: Rect) {
    let is_focused = app.pane_focus == PaneFocus::Left;
    let selected = app.selected_session_idx;
    let visible_height = (area.height.saturating_sub(3)) as usize; // subtract borders + header

    // Compute scroll offset to keep selected visible
    let scroll = if selected >= visible_height {
        selected - visible_height + 1
    } else {
        0
    };

    let header = Row::new(vec![
        Cell::from("Date/Time").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("Model").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("$").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("Cache").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ]);

    let rows: Vec<Row> = sessions
        .iter()
        .enumerate()
        .skip(scroll)
        .map(|(i, s)| {
            let is_sel = i == selected;

            let date = format_session_date(s.started_at);
            let model = shorten_model(&s.model);

            // Use pre-computed summary from the batch query (no per-session DB round-trips)
            let summary = app.session_summaries.get(&s.id);
            let cost = match summary {
                Some(sm) if sm.estimated_cost_usd > 0.0 => format!("${:.3}", sm.estimated_cost_usd),
                _ => "—".to_string(),
            };
            let cache_str = match summary {
                Some(sm) if sm.cache_hit_rate > 0.0 => format!("{:.0}%", sm.cache_hit_rate * 100.0),
                _ => "—".to_string(),
            };

            let base_style = if is_sel && is_focused {
                Style::default().add_modifier(Modifier::REVERSED)
            } else if is_sel {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default()
            };

            let project_branch = if s.git_branch.is_empty() || s.git_branch == "—" {
                s.project_name.clone()
            } else {
                format!("{}/{}", s.project_name, s.git_branch)
            };
            // Append auto-derived task-type tag from the branch prefix (e.g. feat/ → [feature])
            let tag_suffix = branch_to_tag(&s.git_branch)
                .map(|t| format!(" [{}]", t))
                .unwrap_or_default();
            let project_branch = format!("{}{}", project_branch, tag_suffix);
            let pb_short = truncate_with_ellipsis(&project_branch, 20);

            Row::new(vec![
                Cell::from(format!("{}\n{}", date, pb_short)).style(base_style),
                Cell::from(model).style(if is_sel && is_focused {
                    base_style
                } else {
                    Style::default().fg(Color::Green)
                }),
                Cell::from(cost).style(if is_sel && is_focused {
                    base_style
                } else {
                    Style::default().fg(Color::Magenta)
                }),
                Cell::from(cache_str).style(base_style),
            ])
            .height(2)
        })
        .collect();

    let border_style = if is_focused {
        app.theme.active_border_style()
    } else {
        app.theme.inactive_border_style()
    };

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
    // MINOR-14: Show truncation indicator when the list is at its cap.
    let count_str = if sessions.len() != total {
        format!("({} of {}) ", sessions.len(), total)
    } else if total >= 200 {
        format!("({} — showing 200 most recent) ", total)
    } else {
        format!("({}) ", sessions.len())
    };

    let hints = if is_focused {
        "↑↓ · Enter:detail · /:filter · s:sort"
    } else {
        "Tab:←"
    };
    let title = format!(
        " Sessions {}{} [{}] {} ",
        count_str, filter_suffix, sort_label, hints
    );

    let table = Table::new(
        rows,
        [
            Constraint::Length(20),
            Constraint::Length(14),
            Constraint::Length(7),
            Constraint::Min(5),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(app.theme.border_type())
            .border_style(border_style)
            .title(title),
    );

    f.render_widget(table, area);
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
    let cache_col = if cache_pct >= 70.0 {
        Color::Green
    } else if cache_pct >= 40.0 {
        Color::Yellow
    } else {
        Color::Red
    };

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
                Style::default().fg(Color::Blue),
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
                Style::default().fg(Color::Cyan),
            ));
        }
        if let Some(s) = shadow_sonnet {
            shadow_spans.push(Span::styled(
                "  Sonnet ",
                Style::default().fg(app.theme.muted_color()),
            ));
            shadow_spans.push(Span::styled(
                format!("${:.4}", s),
                Style::default().fg(Color::Cyan),
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
                Cell::from(fmt_k(t.input_tokens)).style(Style::default().fg(Color::Blue)),
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
    let ctx_color = if ctx_pct >= 80.0 {
        Color::Red
    } else if ctx_pct >= 60.0 {
        Color::Yellow
    } else {
        Color::Green
    };
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

fn format_session_date(ts_ms: i64) -> String {
    let dt =
        chrono::DateTime::from_timestamp_millis(ts_ms).map(|dt| dt.with_timezone(&chrono::Local));
    match dt {
        Some(dt) => dt.format("%m-%d %H:%M").to_string(),
        None => "—".to_string(),
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
