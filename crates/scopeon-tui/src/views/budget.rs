//! Tab 2: Spend — spending tracker with period cards, projections, breakdowns, and provider sources.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::App;
use crate::text::truncate_with_ellipsis;
use crate::views::components::themed_block;
use chrono::Datelike;

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    // On wide terminals (≥110 cols): left = budget, right = provider sources
    let show_providers = !app.providers.is_empty() && area.width >= 110;
    let (budget_area, providers_area) = if show_providers {
        let h = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
            .split(area);
        (h[0], Some(h[1]))
    } else {
        (area, None)
    };

    draw_budget(f, app, budget_area);

    if let Some(pa) = providers_area {
        super::providers::draw(f, app, pa);
    }
}

fn draw_budget(f: &mut Frame, app: &App, area: Rect) {
    // Compact (< 80w): stack period cards + projection but skip sparkline column.
    // Very narrow (< 55w): period cards go vertical (one per row).
    let narrow = area.width < 55;
    let compact = area.width < 80;

    let card_h = if narrow { 18u16 } else { 6u16 }; // 3 cards × 6h when vertical

    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(card_h), // period cards
            Constraint::Length(2),      // projection strip
            Constraint::Min(0),         // breakdowns [+ sparkline]
        ])
        .split(area);

    draw_period_cards(f, app, v[0], narrow);
    draw_projection(f, app, v[1]);
    draw_breakdowns_and_sparkline(f, app, v[2], compact);
}

// ── Period Cards: Daily | Weekly | Monthly ────────────────────────────────────

fn draw_period_cards(f: &mut Frame, app: &App, area: Rect, vertical: bool) {
    if vertical {
        // Stack cards vertically for very narrow terminals.
        let v = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(6),
                Constraint::Length(6),
                Constraint::Length(6),
            ])
            .split(area);
        draw_period_card(
            f,
            app,
            "Daily",
            app.budget.daily_spent,
            app.budget.daily_limit,
            v[0],
        );
        draw_period_card(
            f,
            app,
            "Weekly",
            app.budget.weekly_spent,
            app.budget.weekly_limit,
            v[1],
        );
        draw_period_card(
            f,
            app,
            "Monthly",
            app.budget.monthly_spent,
            app.budget.monthly_limit,
            v[2],
        );
    } else {
        let h = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(33),
                Constraint::Percentage(34),
                Constraint::Percentage(33),
            ])
            .split(area);
        draw_period_card(
            f,
            app,
            "Daily",
            app.budget.daily_spent,
            app.budget.daily_limit,
            h[0],
        );
        draw_period_card(
            f,
            app,
            "Weekly",
            app.budget.weekly_spent,
            app.budget.weekly_limit,
            h[1],
        );
        draw_period_card(
            f,
            app,
            "Monthly",
            app.budget.monthly_spent,
            app.budget.monthly_limit,
            h[2],
        );
    }
}

fn draw_period_card(f: &mut Frame, app: &App, label: &str, spent: f64, limit: f64, area: Rect) {
    let has_limit = limit > 0.0;
    let ratio = if has_limit {
        (spent / limit).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let pct = (ratio * 100.0) as u16;

    let (status_icon, bar_color) = if !has_limit {
        ("·", app.theme.muted_color())
    } else if pct >= 90 {
        ("✗", app.theme.error_color())
    } else if pct >= 70 {
        ("⚠", app.theme.warning_color())
    } else {
        ("✓", app.theme.success_color())
    };

    let inner_w = area.width.saturating_sub(4) as usize;

    // Hero line: REMAINING (the most important number for the user).
    let hero_line = if has_limit {
        let remaining = (limit - spent).max(0.0);
        let over = (spent - limit).max(0.0);
        if spent > limit {
            Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    format!("${:.2} over budget", over),
                    Style::default()
                        .fg(app.theme.error_color())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {}", status_icon),
                    Style::default().fg(app.theme.error_color()),
                ),
            ])
        } else {
            Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    format!("${:.2} remaining", remaining),
                    Style::default().fg(bar_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("  {}", status_icon), Style::default().fg(bar_color)),
            ])
        }
    } else {
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                format!("${:.2} spent", spent),
                Style::default().fg(bar_color).add_modifier(Modifier::BOLD),
            ),
        ])
    };

    // Secondary line: progress bar + "of $limit" or "no limit" hint.
    let (bar_line, detail_line) = if has_limit {
        let bar_w = inner_w.saturating_sub(5);
        let filled = ((ratio * bar_w as f64) as usize).min(bar_w);
        let bar = "█".repeat(filled) + &"░".repeat(bar_w - filled);

        let bl = Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(bar, Style::default().fg(bar_color)),
            Span::styled(
                format!(" {:>3}%", pct),
                Style::default().fg(app.theme.muted_color()),
            ),
        ]);

        let dl = Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                format!("${:.2} used", spent),
                Style::default().fg(app.theme.muted_color()),
            ),
            Span::styled(
                format!("  of ${:.2}", limit),
                Style::default().fg(app.theme.muted_color()),
            ),
        ]);
        (bl, dl)
    } else {
        let bl = Line::from(Span::styled(
            "  No limit configured",
            Style::default().fg(app.theme.muted_color()),
        ));
        let dl = Line::from(Span::styled(
            "  Set in ~/.scopeon/config.toml",
            Style::default().fg(app.theme.muted_color()),
        ));
        (bl, dl)
    };

    let lines = vec![Line::from(""), hero_line, bar_line, detail_line];

    let border_color = if !has_limit {
        app.theme.muted_color()
    } else if pct >= 90 {
        app.theme.error_color()
    } else if pct >= 70 {
        app.theme.warning_color()
    } else {
        app.theme.success_color()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(app.theme.border_type())
        .border_style(Style::default().fg(border_color))
        .title(format!(" {} ", label));

    f.render_widget(Paragraph::new(lines).block(block), area);
}

// ── Projection strip ──────────────────────────────────────────────────────────

fn draw_projection(f: &mut Frame, app: &App, area: Rect) {
    let proj = app.budget.projected_monthly;
    let monthly_spent = app.budget.monthly_spent;
    let monthly_limit = app.budget.monthly_limit;

    let (trend_icon, trend_label, trend_color) = if monthly_limit > 0.0 {
        let ratio = proj / monthly_limit;
        if ratio > 1.1 {
            ("↗", "OVER PACE", app.theme.error_color())
        } else if ratio > 0.9 {
            ("→", "ON TRACK", app.theme.warning_color())
        } else {
            ("↘", "UNDER BUDGET", app.theme.success_color())
        }
    } else {
        ("↗", "Projection", app.theme.muted_color())
    };

    // TRIZ D3: Budget exhaustion forecast via linear regression on 7d spend.
    let forecast_span = app.budget.predicted_days_until_monthly_limit.map(|days| {
        let label = if days < 1.0 {
            "  ⚠  Budget limit reached today!".to_string()
        } else if days < 7.0 {
            format!("  ⚠  ~{:.0} days until monthly limit", days)
        } else {
            format!("  ~{:.0} days until monthly limit", days)
        };
        let color = if days < 7.0 {
            app.theme.error_color()
        } else if days < 14.0 {
            app.theme.warning_color()
        } else {
            app.theme.muted_color()
        };
        Span::styled(label, Style::default().fg(color))
    });

    let mut spans = vec![
        Span::styled(
            format!("  {} {} ", trend_icon, trend_label),
            Style::default()
                .fg(trend_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("${:.2}/month estimated", proj),
            Style::default().fg(app.theme.cost_color()),
        ),
        Span::styled(
            format!("  ·  this month so far: ${:.2}", monthly_spent),
            Style::default().fg(app.theme.muted_color()),
        ),
    ];
    if let Some(fs) = forecast_span {
        spans.push(fs);
    } else {
        spans.push(Span::styled(
            "  ·  based on this week's pace",
            Style::default().fg(app.theme.muted_color()),
        ));
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);

    // IS-14: EOD (End-of-Day) spend projection row — rendered in the line below if there's room.
    // Caller must allocate 2 rows for this function when daily projection data is available.
    // Here we append a second paragraph if the area is tall enough.
    if area.height >= 2 && app.budget.daily_projected_eod > 0.0 {
        let eod_area = Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: 1,
        };
        let eod_ratio = if app.budget.daily_limit > 0.0 {
            app.budget.daily_projected_eod / app.budget.daily_limit
        } else {
            0.0
        };
        let (eod_icon, eod_color) = if eod_ratio > 0.9 {
            ("⚠ APPROACHING LIMIT", app.theme.error_color())
        } else if eod_ratio > 0.7 {
            ("↗ elevated", app.theme.warning_color())
        } else {
            ("✓ on track", app.theme.success_color())
        };
        let limit_str = if app.budget.daily_limit > 0.0 {
            format!(" of ${:.2} daily limit", app.budget.daily_limit)
        } else {
            String::new()
        };
        let eod_spans = vec![
            Span::styled(
                format!("  {} ", eod_icon),
                Style::default().fg(eod_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("EOD: ${:.2}{}", app.budget.daily_projected_eod, limit_str),
                Style::default().fg(app.theme.cost_color()),
            ),
            Span::styled(
                format!("  ·  at ${:.3}/hr pace", app.budget.daily_hourly_rate),
                Style::default().fg(app.theme.muted_color()),
            ),
        ];
        f.render_widget(Paragraph::new(Line::from(eod_spans)), eod_area);
    }
}

// ── Breakdowns + Sparkline ────────────────────────────────────────────────────

fn draw_breakdowns_and_sparkline(f: &mut Frame, app: &App, area: Rect, compact: bool) {
    if compact || area.height < 10 {
        // No sparkline row — just show side-by-side breakdowns (3 columns).
        let h = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(40),
                Constraint::Percentage(30),
                Constraint::Percentage(30),
            ])
            .split(area);
        draw_provider_model_tree(f, app, h[0]);
        draw_project_breakdown(f, app, h[1]);
        draw_tag_breakdown(f, app, h[2]);
    } else {
        let v = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(7)])
            .split(area);

        let h = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(40),
                Constraint::Percentage(30),
                Constraint::Percentage(30),
            ])
            .split(v[0]);

        draw_provider_model_tree(f, app, h[0]);
        draw_project_breakdown(f, app, h[1]);
        draw_tag_breakdown(f, app, h[2]);
        draw_sparkline(f, app, v[1]);
    }
}

/// C-19: Provider → Model cost breakdown tree.
/// Shows each provider as a top-level row (bold, total cost, bar), with model
/// sub-rows indented below it.
fn draw_provider_model_tree(f: &mut Frame, app: &App, area: Rect) {
    let data = &app.budget.cost_by_provider_model;
    let muted = app.theme.muted_color();
    let cost_color = app.theme.cost_color();
    let model_color = app.theme.model_color();

    // Build per-provider totals.
    let mut provider_totals: std::collections::BTreeMap<&str, f64> =
        std::collections::BTreeMap::new();
    for (p, _, c) in data {
        *provider_totals.entry(p.as_str()).or_default() += c;
    }

    // Sort providers by total cost descending.
    let mut sorted_providers: Vec<(&str, f64)> = provider_totals.into_iter().collect();
    sorted_providers
        .sort_by(|(_, a), (_, b)| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));

    let grand_total: f64 = sorted_providers.iter().map(|(_, c)| c).sum();
    let max = sorted_providers
        .first()
        .map(|(_, c)| *c)
        .unwrap_or(1.0)
        .max(1.0);

    let bar_w = 8usize;
    let inner_w = area.width.saturating_sub(2) as usize;
    let name_w = inner_w.saturating_sub(2 + 8 + 2 + bar_w).clamp(10, 22);

    let mut lines: Vec<Line> = vec![];
    let available_rows = (area.height as usize).saturating_sub(2);

    'outer: for (provider, prov_total) in &sorted_providers {
        if lines.len() >= available_rows {
            break;
        }
        let ratio = prov_total / max;
        let filled = (ratio * bar_w as f64) as usize;
        let bar = "█".repeat(filled) + &"░".repeat(bar_w - filled);
        let pct = if grand_total > 0.0 {
            prov_total / grand_total * 100.0
        } else {
            0.0
        };
        let short_name = truncate_with_ellipsis(provider, name_w);
        // Provider row — bold, full-width bar
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<name_w$}", short_name, name_w = name_w),
                Style::default()
                    .fg(app.theme.heading_color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:>7} {:>4.0}%", format!("${:.2}", prov_total), pct),
                Style::default().fg(cost_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" {}", bar), Style::default().fg(cost_color)),
        ]));

        // Model sub-rows
        for (p2, model, model_cost) in data {
            if p2.as_str() != *provider {
                continue;
            }
            if lines.len() >= available_rows {
                break 'outer;
            }
            let model_ratio = model_cost / prov_total.max(1.0);
            let mfilled = (model_ratio * bar_w as f64) as usize;
            let mbar = "▒".repeat(mfilled) + &"░".repeat(bar_w - mfilled);
            let short_model = shorten_model(model);
            let short_model = truncate_with_ellipsis(&short_model, name_w.saturating_sub(2));
            lines.push(Line::from(vec![
                Span::styled(
                    format!(
                        "   └─{:<name_w$}",
                        short_model,
                        name_w = name_w.saturating_sub(2)
                    ),
                    Style::default().fg(model_color),
                ),
                Span::styled(
                    format!("{:>7}     ", format!("${:.2}", model_cost)),
                    Style::default().fg(muted),
                ),
                Span::styled(format!(" {}", mbar), Style::default().fg(muted)),
            ]));
        }
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No data yet",
            Style::default().fg(muted),
        )));
    }

    f.render_widget(
        Paragraph::new(lines).block(themed_block(app.theme, "By Source → Model", false)),
        area,
    );
}

fn draw_project_breakdown(f: &mut Frame, app: &App, area: Rect) {
    let items = &app.budget.cost_by_project;
    let max = items.iter().map(|(_, c)| *c).fold(0.0_f64, f64::max);

    let inner_w = area.width.saturating_sub(2) as usize;
    let fixed = 2 + 8 + 2 + 10;
    let name_w = inner_w.saturating_sub(fixed).clamp(10, 32);

    let mut lines: Vec<Line> = vec![];
    for (project, cost) in items.iter().take(area.height as usize - 2) {
        let ratio = if max > 0.0 { cost / max } else { 0.0 };
        let bar_w = 10usize;
        let filled = (ratio * bar_w as f64) as usize;
        let bar = "█".repeat(filled) + &"░".repeat(bar_w - filled);
        let short = truncate_with_ellipsis(project, name_w);
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:name_w$}", short, name_w = name_w),
                Style::default().fg(app.theme.text_primary()),
            ),
            Span::styled(
                format!("{:>8}", format!("${:.2}", cost)),
                Style::default().fg(app.theme.cost_color()),
            ),
            Span::styled(
                format!("  {}", bar),
                Style::default().fg(app.theme.cost_color()),
            ),
        ]));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No data yet",
            Style::default().fg(app.theme.muted_color()),
        )));
    }

    f.render_widget(
        Paragraph::new(lines).block(themed_block(app.theme, "By Project", false)),
        area,
    );
}

fn draw_tag_breakdown(f: &mut Frame, app: &App, area: Rect) {
    let items = &app.budget.cost_by_tag;
    let max = items.iter().map(|(_, c, _)| *c).fold(0.0_f64, f64::max);

    let mut lines: Vec<Line> = vec![];
    for (tag, cost, count) in items.iter().take(area.height as usize - 2) {
        let ratio = if max > 0.0 { cost / max } else { 0.0 };
        let bar_w = 8usize;
        let filled = (ratio * bar_w as f64) as usize;
        let bar = "█".repeat(filled) + &"░".repeat(bar_w - filled);
        let label = capitalize_tag(tag);
        let short = truncate_with_ellipsis(&label, 12);
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<12}", short),
                Style::default().fg(app.theme.text_primary()),
            ),
            Span::styled(
                format!("{:>7}", format!("${:.2}", cost)),
                Style::default().fg(app.theme.warning_color()),
            ),
            Span::styled(
                format!(" ({:>2})", count),
                Style::default().fg(app.theme.muted_color()),
            ),
            Span::styled(
                format!(" {}", bar),
                Style::default().fg(app.theme.warning_color()),
            ),
        ]));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No tags yet",
            Style::default().fg(app.theme.muted_color()),
        )));
        lines.push(Line::from(Span::styled(
            "  scopeon tag set --session <id> feat",
            Style::default().fg(app.theme.muted_color()),
        )));
    }

    f.render_widget(
        Paragraph::new(lines).block(themed_block(app.theme, "By Task Type", false)),
        area,
    );
}

fn capitalize_tag(tag: &str) -> String {
    let mut c = tag.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

fn draw_sparkline(f: &mut Frame, app: &App, area: Rect) {
    let history = &app.budget.daily_history; // newest first
    let data: Vec<(String, f64)> = history.iter().rev().map(|(d, c)| (d.clone(), *c)).collect();

    if data.is_empty() {
        let p = Paragraph::new("  No daily history yet.").block(themed_block(
            app.theme,
            "Daily Cost (last 14 days)",
            false,
        ));
        f.render_widget(p, area);
        return;
    }

    let max = data
        .iter()
        .map(|(_, c)| *c)
        .fold(0.0_f64, f64::max)
        .max(0.001);
    let avg = data.iter().map(|(_, c)| c).sum::<f64>() / data.len() as f64;
    let today_cost = data.last().map(|(_, c)| *c).unwrap_or(0.0);
    let daily_limit = app.budget.daily_limit;

    let inner_w = area.width.saturating_sub(4) as usize;
    let bar_w = (inner_w / data.len()).clamp(1, 10);

    let bar_spans: Vec<Span> = data
        .iter()
        .map(|(_, v)| {
            let ratio = v / max;
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
            let color = if daily_limit > 0.0 {
                let pct = v / daily_limit;
                if pct >= 0.8 {
                    app.theme.error_color()
                } else if pct >= 0.5 {
                    app.theme.warning_color()
                } else {
                    app.theme.success_color()
                }
            } else {
                app.theme.cost_color()
            };
            Span::styled(
                ch.to_string().repeat(filled) + &"▁".repeat(bar_w - filled),
                Style::default().fg(color),
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

    let mut header_spans = vec![
        Span::styled("  max ", Style::default().fg(app.theme.muted_color())),
        Span::styled(
            format!("${:.2}", max),
            Style::default().fg(app.theme.cost_color()),
        ),
        Span::styled("  avg ", Style::default().fg(app.theme.muted_color())),
        Span::styled(
            format!("${:.2}", avg),
            Style::default().fg(app.theme.cost_color()),
        ),
        Span::styled("  today ", Style::default().fg(app.theme.muted_color())),
        Span::styled(
            format!("${:.2}", today_cost),
            Style::default()
                .fg(app.theme.cost_color())
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if daily_limit > 0.0 {
        header_spans.push(Span::styled(
            format!("  limit ${:.2}", daily_limit),
            Style::default().fg(app.theme.muted_color()),
        ));
    }

    let mut bar_line = vec![Span::styled("  ", Style::default())];
    bar_line.extend(bar_spans);
    let mut date_line = vec![Span::styled("  ", Style::default())];
    date_line.extend(date_spans);

    let lines = vec![
        Line::from(header_spans),
        Line::from(bar_line),
        Line::from(date_line),
    ];

    f.render_widget(
        Paragraph::new(lines).block(themed_block(app.theme, "Daily Cost (last 14 days)", false)),
        area,
    );
}

fn shorten_model(model: &str) -> String {
    if let Some(s) = model.strip_prefix("claude-") {
        let parts: Vec<&str> = s.split('-').collect();
        if parts.len() >= 2 {
            return format!("{}-{}", parts[0], parts[1]);
        }
    }
    truncate_with_ellipsis(model, 24)
}
