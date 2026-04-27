/// Tab 3: Health — per-provider activity summary + health score + waste signals + visual metric bars
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph},
    Frame,
};

use scopeon_core::PRICING_VERIFIED_DATE;
use scopeon_metrics::{MetricCategory, MetricValue};

use crate::app::App;
use crate::text::truncate_with_ellipsis;
use crate::views::components::{themed_block, themed_block_borders};
use crate::views::dashboard::health_color;

/// Returns the number of days since [`PRICING_VERIFIED_DATE`], or `None` if
/// the date cannot be parsed (treated as non-stale to avoid false alarms).
fn pricing_staleness_days() -> Option<i64> {
    let verified = chrono::NaiveDate::parse_from_str(PRICING_VERIFIED_DATE, "%Y-%m-%d").ok()?;
    let today = chrono::Utc::now().date_naive();
    Some((today - verified).num_days())
}

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    // Per-provider activity summary at top (only when ≥2 providers)
    let show_provider_summary = app.all_providers.len() >= 2;
    let summary_h = if show_provider_summary { 2u16 } else { 0u16 };

    // Optionally show a 1-line pricing staleness warning at the bottom.
    let stale_days = pricing_staleness_days();
    let show_warning = stale_days.map(|d| d > 90).unwrap_or(false);

    let mut constraints: Vec<Constraint> = Vec::new();
    if show_provider_summary {
        constraints.push(Constraint::Length(summary_h));
    }
    constraints.push(Constraint::Min(0));
    if show_warning {
        constraints.push(Constraint::Length(1));
    }

    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let mut idx = 0usize;

    if show_provider_summary {
        draw_provider_summary(f, app, v[idx]);
        idx += 1;
    }

    let main_area = v[idx];
    idx += 1;

    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),  // health score gauge + breakdown
            Constraint::Length(12), // waste signals + suggestions (2-col) — taller for wrapping
            Constraint::Min(0),     // visual metric rows
        ])
        .split(main_area);

    draw_health_gauge(f, app, inner[0]);
    draw_waste_and_suggestions(f, app, inner[1]);
    draw_metrics_visual(f, app, inner[2]);

    if show_warning {
        let days = stale_days.unwrap_or(0);
        let warning = Paragraph::new(Line::from(vec![
            Span::styled(
                " ⚠ ".to_string(),
                Style::default()
                    .fg(app.theme.warning_color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    "Pricing last verified {} ({} days ago). Costs may be inaccurate. \
                     Verify at anthropic.com/pricing · openai.com/api/pricing",
                    PRICING_VERIFIED_DATE, days
                ),
                Style::default().fg(app.theme.warning_color()),
            ),
        ]));
        f.render_widget(warning, v[idx]);
    }
}

fn draw_provider_summary(f: &mut Frame, app: &App, area: Rect) {
    // Build per-provider stats from sessions_list + session_summaries
    let mut stats: std::collections::HashMap<String, (usize, f64)> =
        std::collections::HashMap::new();
    for s in &app.sessions_list {
        if s.provider.is_empty() {
            continue;
        }
        let entry = stats.entry(s.provider.clone()).or_insert((0, 0.0));
        entry.0 += 1;
        if let Some(sm) = app.session_summaries.get(&s.id) {
            entry.1 += sm.cache_hit_rate;
        }
    }

    // Cost per provider from budget data
    let mut costs: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    for (provider, _, cost) in &app.budget.cost_by_provider_model {
        *costs.entry(provider.clone()).or_insert(0.0) += cost;
    }

    let t = app.theme;

    // Title line
    let title_line = Line::from(vec![Span::styled(
        "  ◈ Provider Activity ".to_string(),
        Style::default().fg(t.muted_color()),
    )]);

    // Stats line: key metrics per provider separated by bullets
    let mut stat_spans: Vec<Span<'static>> = vec![Span::raw("  ".to_string())];
    for (i, provider) in app.all_providers.iter().enumerate() {
        if i > 0 {
            stat_spans.push(Span::styled("   ·   ".to_string(), Style::default().fg(t.muted_color())));
        }
        let (count, cache_sum) = stats.get(provider).copied().unwrap_or((0, 0.0));
        let avg_cache = if count > 0 { cache_sum / count as f64 * 100.0 } else { 0.0 };
        let cost = costs.get(provider).copied().unwrap_or(0.0);
        let pname = truncate_with_ellipsis(provider, 14);
        stat_spans.push(Span::styled(
            format!("◈ {}  ", pname),
            Style::default().fg(t.heading_color()).add_modifier(Modifier::BOLD),
        ));
        stat_spans.push(Span::styled(
            format!("${:.2}  ", cost),
            Style::default().fg(t.cost_color()),
        ));
        stat_spans.push(Span::styled(
            format!("{}sess  ", count),
            Style::default().fg(t.text_secondary()),
        ));
        if avg_cache > 0.0 {
            stat_spans.push(Span::styled(
                format!("cache ~{:.0}%  ", avg_cache),
                Style::default().fg(t.cache_color(avg_cache)),
            ));
        }
    }

    f.render_widget(
        Paragraph::new(vec![title_line, Line::from(stat_spans)]),
        area,
    );
}

fn draw_health_gauge(f: &mut Frame, app: &App, area: Rect) {
    let health = app.health_score;
    let color = health_color(health);

    // Split: gauge bar (top 3 rows) + breakdown line (bottom 2 rows)
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(app.theme.border_type())
                .title(" Health Score "),
        )
        .gauge_style(Style::default().fg(color).add_modifier(Modifier::BOLD))
        .percent(health as u16)
        .label(format!("{:.0} / 100", health));

    f.render_widget(gauge, v[0]);

    // CONC-2: Health score breakdown — shows per-component contribution.
    if let Some(bd) = &app.health_breakdown {
        let muted = app.theme.muted_color();
        let mut spans: Vec<Span> = vec![Span::styled("  Score: ", Style::default().fg(muted))];
        for (label, earned, max) in bd.as_rows() {
            let c = if earned >= max * 0.8 {
                app.theme.success_color()
            } else if earned >= max * 0.5 {
                app.theme.warning_color()
            } else {
                app.theme.error_color()
            };
            spans.push(Span::styled(
                format!("{} {:.0}/{:.0}  ", label, earned, max),
                Style::default().fg(c),
            ));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), v[1]);
    } else {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  No session data",
                Style::default().fg(app.theme.muted_color()),
            ))),
            v[1],
        );
    }
}

fn draw_waste_and_suggestions(f: &mut Frame, app: &App, area: Rect) {
    let h = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    draw_waste_signals(f, app, h[0]);
    draw_suggestions(f, app, h[1]);
}

fn draw_waste_signals(f: &mut Frame, app: &App, area: Rect) {
    let empty_waste = scopeon_metrics::WasteReport {
        signals: vec![],
        waste_score: 0.0,
    };
    let waste = app.waste_report.as_ref().unwrap_or(&empty_waste);

    let mut lines: Vec<Line> = vec![];
    if waste.signals.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  ✓ No waste signals detected",
            Style::default().fg(app.theme.success_color()),
        )));
    } else {
        for signal in &waste.signals {
            let (icon, color) = match signal.severity {
                scopeon_metrics::waste::Severity::Critical => ("✗", app.theme.error_color()),
                scopeon_metrics::waste::Severity::Warning => ("⚠", app.theme.warning_color()),
                scopeon_metrics::waste::Severity::Info => ("ℹ", app.theme.accent_color()),
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {} ", icon), Style::default().fg(color)),
                Span::raw(signal.message.clone()),
            ]));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  Waste: ", Style::default().fg(app.theme.muted_color())),
        Span::styled(
            format!("{:.0}/100", waste.waste_score),
            Style::default().fg(if waste.waste_score >= 75.0 {
                app.theme.error_color()
            } else if waste.waste_score >= 50.0 {
                app.theme.warning_color()
            } else {
                app.theme.success_color()
            }),
        ),
        Span::styled(
            if waste.waste_score < 30.0 {
                "  ✓ below avg"
            } else {
                ""
            },
            Style::default().fg(app.theme.muted_color()),
        ),
    ]));

    f.render_widget(
        Paragraph::new(lines).block(themed_block(app.theme, "Waste Signals", false)),
        area,
    );
}

fn draw_suggestions(f: &mut Frame, app: &App, area: Rect) {
    // IS-6: Render each suggestion as a bordered anomaly card with severity, cost impact, and action.
    if app.suggestions.is_empty() {
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  ✓ No optimization suggestions",
                Style::default().fg(app.theme.muted_color()),
            )),
        ];
        f.render_widget(
            // Right panel: share left border with the waste signals panel.
            Paragraph::new(lines).block(themed_block_borders(
                app.theme,
                "Suggestions",
                false,
                Borders::TOP | Borders::RIGHT | Borders::BOTTOM,
            )),
            area,
        );
        return;
    }

    // Outer container block — no left border to avoid double-border with the waste panel.
    let block = themed_block_borders(
        app.theme,
        "Suggestions",
        false,
        Borders::TOP | Borders::RIGHT | Borders::BOTTOM,
    );
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Each card gets a minimum of 4 rows. Divide available height.
    let n = app.suggestions.len().min(4);
    let card_h = (inner.height / n.max(1) as u16).clamp(3, 6);
    let constraints: Vec<Constraint> = (0..n).map(|_| Constraint::Length(card_h)).collect();
    let card_areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    for (i, suggestion) in app.suggestions.iter().take(n).enumerate() {
        let (icon, sev_color, border_color) = match suggestion.severity {
            scopeon_metrics::suggestions::Severity::Critical => {
                ("⚡", app.theme.error_color(), app.theme.error_color())
            },
            scopeon_metrics::suggestions::Severity::Warning => {
                ("⚠", app.theme.warning_color(), app.theme.warning_color())
            },
            scopeon_metrics::suggestions::Severity::Info => {
                ("→", app.theme.accent_color(), app.theme.accent_color())
            },
        };

        // Card title: icon + suggestion title.
        let title_text = format!(" {} {} ", icon, suggestion.title);
        let card_block = Block::default()
            .borders(Borders::ALL)
            .border_type(app.theme.border_type())
            .border_style(Style::default().fg(border_color))
            .title(Span::styled(
                title_text,
                Style::default().fg(sev_color).add_modifier(Modifier::BOLD),
            ));
        let card_inner = card_block.inner(card_areas[i]);
        f.render_widget(card_block, card_areas[i]);

        // Body: suggestion body text wrapped to card width.
        let body_w = card_inner.width.saturating_sub(2) as usize;
        let wrapped = word_wrap(&suggestion.body, body_w);
        let mut body_lines: Vec<Line> = wrapped
            .iter()
            .take(3)
            .map(|part| {
                Line::from(Span::styled(
                    format!(" {}", part),
                    Style::default().fg(app.theme.text_secondary()),
                ))
            })
            .collect();

        // C-06: Show actionable command hint based on suggestion id.
        if let Some(cmd) = suggestion_action_hint(suggestion.id) {
            body_lines.push(Line::from(vec![
                Span::styled(" → ", Style::default().fg(app.theme.accent_dim())),
                Span::styled(
                    cmd,
                    Style::default()
                        .fg(app.theme.accent_color())
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        }

        f.render_widget(Paragraph::new(body_lines), card_inner);
    }
}

/// Word-wrap `text` to at most `max_width` display characters per line.
fn word_wrap(text: &str, max_width: usize) -> Vec<String> {
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

fn draw_metrics_visual(f: &mut Frame, app: &App, area: Rect) {
    if app.cached_metrics.is_empty() {
        let p = Paragraph::new("  No session data yet.")
            .block(themed_block(app.theme, "Metrics", false));
        f.render_widget(p, area);
        return;
    }

    // Inner width = total width minus borders (2) and left-pad (2)
    let inner_w = area.width.saturating_sub(4) as usize;

    let mut lines: Vec<Line> = vec![];
    let mut last_cat: Option<MetricCategory> = None;

    for (cat, name, val, fmted, desc) in &app.cached_metrics {
        // Emit a category header when the category changes
        if last_cat.as_ref() != Some(cat) {
            let cat_label = match cat {
                MetricCategory::Cache => "  ─ Cache ─────────────────────────────────────────",
                MetricCategory::Cost => "  ─ Cost ──────────────────────────────────────────",
                MetricCategory::Velocity => "  ─ Velocity ───────────────────────────────────────",
                MetricCategory::Quality => "  ─ Quality ────────────────────────────────────────",
                MetricCategory::Pattern => "  ─ Pattern ────────────────────────────────────────",
                MetricCategory::Session => "  ─ Session ────────────────────────────────────────",
            };
            lines.push(Line::from(Span::styled(
                cat_label.to_string(),
                Style::default().fg(Color::DarkGray),
            )));
            last_cat = Some(cat.clone());
        }
        lines.push(metric_visual_line(name, val, fmted, desc, inner_w));
    }

    f.render_widget(
        Paragraph::new(lines)
            .block(themed_block(app.theme, "Metrics", false))
            .scroll((0, 0)),
        area,
    );
}

fn metric_visual_line(
    name: &str,
    value: &MetricValue,
    formatted: &str,
    description: &str,
    inner_w: usize,
) -> Line<'static> {
    // Fixed-width columns: left-pad(2) + name(16) + bar(12) + value(9) + trend(3) = 42
    const NAME_W: usize = 16;
    const BAR_W: usize = 12;
    const VAL_W: usize = 9;
    const TREND_W: usize = 3; // " ─ "
    const FIXED: usize = NAME_W + BAR_W + VAL_W + TREND_W; // 40 (plus the leading 2-space pad)

    let (bar_ratio, val_color) = metric_to_ratio_and_color(value);

    let filled = (bar_ratio * BAR_W as f64).min(BAR_W as f64) as usize;
    let bar = "█".repeat(filled) + &"░".repeat(BAR_W - filled);

    let name_padded = format!("{:<width$}", name, width = NAME_W);
    let val_padded = format!("{:>width$}", formatted, width = VAL_W);

    // Compute how many chars are available for description after fixed columns + 2-space left pad
    let used = 2 + FIXED;
    let desc_max = inner_w.saturating_sub(used);

    let desc = if desc_max == 0 {
        String::new()
    } else if description.len() > desc_max {
        // Truncate on a char boundary to avoid panic on multibyte chars
        let mut end = desc_max.saturating_sub(1);
        while !description.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &description[..end])
    } else {
        description.to_string()
    };

    Line::from(vec![
        Span::styled(
            format!("  {}", name_padded),
            Style::default().fg(Color::White),
        ),
        Span::styled(bar, Style::default().fg(val_color)),
        Span::styled(
            val_padded,
            Style::default().fg(val_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ─ ", Style::default().fg(Color::DarkGray)),
        Span::styled(desc, Style::default().fg(Color::DarkGray)),
    ])
}

fn metric_to_ratio_and_color(value: &MetricValue) -> (f64, Color) {
    match value {
        MetricValue::Percentage(p) => {
            let ratio = (*p / 100.0).clamp(0.0, 1.0);
            let color = if *p >= 70.0 {
                Color::Green
            } else if *p >= 40.0 {
                Color::Yellow
            } else {
                Color::Red
            };
            (ratio, color)
        },
        MetricValue::Currency(c) => {
            // Scale: $0 = empty bar, >$1 = full bar (logarithmic feel)
            let ratio = (c.log10() + 3.0).clamp(0.0, 6.0) / 6.0;
            (ratio, Color::Magenta)
        },
        MetricValue::Float(v) => {
            let ratio = (*v / 10.0).clamp(0.0, 1.0);
            (ratio, Color::Cyan)
        },
        MetricValue::Count(n) => {
            let ratio = (*n as f64 / 100.0).clamp(0.0, 1.0);
            (ratio, Color::Yellow)
        },
        MetricValue::Duration(ms) => {
            let ratio = (*ms / 60_000.0).clamp(0.0, 1.0);
            (ratio, Color::Blue)
        },
        MetricValue::Unavailable => (0.0, Color::DarkGray),
        _ => (0.5, Color::White),
    }
}

/// Map a suggestion id to a short actionable command hint, or `None` if no hint applies.
fn suggestion_action_hint(id: &str) -> Option<&'static str> {
    match id {
        "cache-warmup" => Some("/compact  — compact context to warm cache faster"),
        "compaction-freq" => Some("/compact  — run before context exceeds 60%"),
        "high-cache-miss" => Some("Reuse existing sessions instead of starting fresh"),
        "no-cache" => Some("Start a longer session; cache activates after first write"),
        "high-cost-turn" => Some("Use /clear or /compact to reset expensive context"),
        "spike-input" => Some("Break task into smaller sub-tasks or use /clear"),
        "hook-overhead" => Some("Review ~/.claude/hooks — remove unused or slow hooks"),
        "no-skill" | "skill-gap" => Some("Create CLAUDE.md in project root with domain context"),
        _ => None,
    }
}
