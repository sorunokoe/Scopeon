//! Ratatui-based first-run onboarding wizard.
//!
//! Called from the main crate's `onboarding::run_wizard_if_needed()`.
//! Renders 4 pages inside the alternate screen using Cockpit theme.

use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Gauge, Paragraph, Row, Table},
    Frame, Terminal,
};

use crate::logo::logo_lines;
use crate::theme::Theme;
use crate::views::components::themed_block;

// ── Entry point ───────────────────────────────────────────────────────────────

/// Run the interactive Ratatui onboarding wizard.
///
/// `providers` is a slice of (name, found, hint) tuples detected by the
/// caller.  The wizard renders in the alternate screen and returns once the
/// user either completes all pages or presses `q`/`Esc`.
pub fn run_wizard_tui(providers: &[(String, bool, String)]) -> Result<()> {
    enable_raw_mode()?;
    std::io::stdout().execute(EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // Flush any events that arrived before raw mode was active — most notably
    // the Enter key the user pressed to run `scopeon` from their shell, which
    // would otherwise advance the very first wizard page immediately.
    std::thread::sleep(Duration::from_millis(50));
    while event::poll(Duration::from_millis(0))? {
        let _ = event::read()?;
    }

    let theme = Theme::Cockpit;
    let total = 4usize;
    // Minimum time a page must be visible before ENTER/Space advances it.
    // Prevents key-repeat from blasting through the wizard accidentally.
    const PAGE_HOLD: Duration = Duration::from_millis(1500);

    for page in 0..total {
        // Drain any queued events (key-repeat from the previous ENTER press)
        // so they don't immediately advance this new page.
        while event::poll(Duration::from_millis(0))? {
            let _ = event::read()?;
        }
        let page_shown_at = Instant::now();
        loop {
            terminal.draw(|f| draw_wizard_page(f, page, total, providers, theme))?;

            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    match key.code {
                        KeyCode::Enter | KeyCode::Char(' ') => {
                            // Ignore if the page hasn't been on screen long enough.
                            if page_shown_at.elapsed() >= PAGE_HOLD {
                                break;
                            }
                        },
                        KeyCode::Char('q') | KeyCode::Esc => {
                            wizard_cleanup()?;
                            return Ok(());
                        },
                        _ => {},
                    }
                }
            }
        }
    }

    wizard_cleanup()
}

fn wizard_cleanup() -> Result<()> {
    disable_raw_mode()?;
    std::io::stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

// ── Page renderer ─────────────────────────────────────────────────────────────

fn draw_wizard_page(
    f: &mut Frame,
    page: usize,
    total: usize,
    providers: &[(String, bool, String)],
    theme: Theme,
) {
    let area = f.area();
    f.render_widget(ratatui::widgets::Clear, area);

    // Outer layout: padding on sides for a centered card feel
    let h_margin = (area.width.saturating_sub(76)) / 2;
    let card = Rect {
        x: h_margin,
        y: 1,
        width: area.width.saturating_sub(h_margin * 2),
        height: area.height.saturating_sub(2),
    };

    // Card sections: logo(7) | content(min) | progress(3) | footer(2)
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7), // ASCII logo
            Constraint::Min(8),    // page content
            Constraint::Length(3), // progress gauge
            Constraint::Length(2), // footer hints
        ])
        .split(card);

    draw_logo_section(f, theme, v[0]);
    draw_content_section(f, page, providers, theme, v[1]);
    draw_progress_gauge(f, page, total, theme, v[2]);
    draw_footer(f, page, total, theme, v[3]);
}

fn draw_logo_section(f: &mut Frame, theme: Theme, area: Rect) {
    let logo = logo_lines(theme);
    f.render_widget(Paragraph::new(logo), area);
}

fn draw_content_section(
    f: &mut Frame,
    page: usize,
    providers: &[(String, bool, String)],
    theme: Theme,
    area: Rect,
) {
    match page {
        0 => draw_page_welcome(f, theme, area),
        1 => draw_page_providers(f, providers, theme, area),
        2 => draw_page_shortcuts(f, theme, area),
        3 => draw_page_shell(f, theme, area),
        _ => {},
    }
}

fn draw_progress_gauge(f: &mut Frame, page: usize, total: usize, theme: Theme, area: Rect) {
    let ratio = (page + 1) as f64 / total as f64;
    let pct = ((ratio * 100.0) as u16).min(100);
    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(theme.border_type())
                .border_style(Style::default().fg(theme.muted_color()))
                .title(format!(" Step {} of {} ", page + 1, total)),
        )
        .gauge_style(
            Style::default()
                .fg(theme.accent_color())
                .add_modifier(Modifier::BOLD),
        )
        .percent(pct);
    f.render_widget(gauge, area);
}

fn draw_footer(f: &mut Frame, page: usize, total: usize, theme: Theme, area: Rect) {
    let action = if page < total - 1 {
        "▶  Press ENTER to continue"
    } else {
        "▶  Press ENTER to open your dashboard"
    };
    let line = Line::from(vec![
        Span::styled(
            format!("  {}", action),
            Style::default()
                .fg(theme.success_color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "    [q] Skip setup",
            Style::default().fg(theme.muted_color()),
        ),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

// ── Individual pages ──────────────────────────────────────────────────────────

fn draw_page_welcome(f: &mut Frame, theme: Theme, area: Rect) {
    let text = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Scopeon watches your AI coding agent so you don't have to.",
            Style::default().fg(theme.text_primary()),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  It tracks token usage, cache efficiency, cost, and context",
            Style::default().fg(theme.text_primary()),
        )),
        Line::from(Span::styled(
            "  pressure — 100% locally, no cloud, no accounts.",
            Style::default().fg(theme.text_primary()),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  Your dashboard has been pre-loaded with demo data so you",
            Style::default().fg(theme.muted_color()),
        )),
        Line::from(Span::styled(
            "  can explore every feature right now, before any AI session.",
            Style::default().fg(theme.muted_color()),
        )),
    ];
    f.render_widget(
        Paragraph::new(text).block(themed_block(theme, "Welcome to Scopeon", false)),
        area,
    );
}

fn draw_page_providers(
    f: &mut Frame,
    providers: &[(String, bool, String)],
    theme: Theme,
    area: Rect,
) {
    let accent = theme.accent_color();
    let muted = theme.muted_color();
    let success = theme.success_color();
    let text = theme.text_primary();

    let rows: Vec<Row> = providers
        .iter()
        .map(|(name, found, hint)| {
            if *found {
                Row::new(vec![
                    Cell::from(Span::styled(
                        "  ✓",
                        Style::default().fg(success).add_modifier(Modifier::BOLD),
                    )),
                    Cell::from(Span::styled(
                        name.as_str(),
                        Style::default().fg(text).add_modifier(Modifier::BOLD),
                    )),
                    Cell::from(Span::styled(hint.as_str(), Style::default().fg(accent))),
                ])
            } else {
                Row::new(vec![
                    Cell::from(Span::styled("  ·", Style::default().fg(muted))),
                    Cell::from(Span::styled(name.as_str(), Style::default().fg(muted))),
                    Cell::from(Span::styled(hint.as_str(), Style::default().fg(muted))),
                ])
            }
        })
        .collect();

    let subheading = Line::from(vec![Span::styled(
        "  Scopeon auto-monitors every ✓ provider — no config needed.",
        Style::default().fg(muted),
    )]);

    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    f.render_widget(Paragraph::new(vec![subheading]), inner[0]);

    let table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Length(22),
            Constraint::Min(20),
        ],
    )
    .block(themed_block(theme, "Detected AI Providers", false));

    f.render_widget(table, inner[1]);
}

fn draw_page_shortcuts(f: &mut Frame, theme: Theme, area: Rect) {
    let key_style = Style::default()
        .fg(theme.accent_color())
        .add_modifier(Modifier::BOLD);
    let text = theme.text_primary();
    let muted = theme.muted_color();
    let heading = Style::default()
        .fg(theme.heading_color())
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);

    let rows = vec![
        Row::new(vec![
            Cell::from(Span::styled("  Navigation", heading)),
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
            Cell::from(Span::styled("  Tab", key_style)),
            Cell::from(Span::styled(
                "  Next tab / switch pane",
                Style::default().fg(text),
            )),
        ]),
        Row::new(vec![Cell::from(""), Cell::from("")]),
        Row::new(vec![
            Cell::from(Span::styled("  Actions", heading)),
            Cell::from(""),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  r", key_style)),
            Cell::from(Span::styled(
                "  Force refresh data",
                Style::default().fg(text),
            )),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  c", key_style)),
            Cell::from(Span::styled(
                "  Copy stats to clipboard",
                Style::default().fg(text),
            )),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  /", key_style)),
            Cell::from(Span::styled(
                "  Filter sessions (Sessions tab)",
                Style::default().fg(text),
            )),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  s", key_style)),
            Cell::from(Span::styled(
                "  Cycle sort order (Sessions tab)",
                Style::default().fg(text),
            )),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  ?", key_style)),
            Cell::from(Span::styled(
                "  Open help overlay",
                Style::default().fg(text),
            )),
        ]),
        Row::new(vec![
            Cell::from(Span::styled("  q", key_style)),
            Cell::from(Span::styled("  Quit Scopeon", Style::default().fg(text))),
        ]),
        Row::new(vec![Cell::from(""), Cell::from("")]),
        Row::new(vec![
            Cell::from(Span::styled(
                "  Mouse: click tab bar to switch, scroll to navigate",
                Style::default().fg(muted),
            )),
            Cell::from(""),
        ]),
    ];

    let table = Table::new(rows, [Constraint::Length(16), Constraint::Min(0)]).block(themed_block(
        theme,
        "Key Shortcuts",
        false,
    ));

    f.render_widget(table, area);
}

fn draw_page_shell(f: &mut Frame, theme: Theme, area: Rect) {
    let accent = theme.accent_color();
    let muted = theme.muted_color();
    let text = theme.text_primary();
    let success = theme.success_color();

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Add one line to your shell for zero-attention status in your",
            Style::default().fg(text),
        )),
        Line::from(Span::styled(
            "  prompt — no TUI window required:",
            Style::default().fg(text),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  bash / zsh",
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  — add to ~/.bashrc or ~/.zshrc:",
                Style::default().fg(muted),
            ),
        ]),
        Line::from(Span::styled(
            "    eval \"$(scopeon shell-hook)\"",
            Style::default().fg(success),
        )),
        Line::from(Span::styled(
            "    # then add $SCOPEON_STATUS to your PS1 / RPROMPT",
            Style::default().fg(muted),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  fish",
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "             — add to ~/.config/fish/config.fish:",
                Style::default().fg(muted),
            ),
        ]),
        Line::from(Span::styled(
            "    scopeon shell-hook --shell fish | source",
            Style::default().fg(success),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  Prompt shows:  ⬡87 73% $2.41  (health · context · daily cost)",
            Style::default().fg(muted),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  This step is optional — skip with q and set up later.",
            Style::default().fg(muted),
        )),
    ];

    f.render_widget(
        Paragraph::new(lines).block(themed_block(
            theme,
            "Ambient Intelligence (Optional)",
            false,
        )),
        area,
    );
}

// ── Stub for when providers list is not needed ────────────────────────────────

/// Run wizard with no providers list (used in tests or fallback).
#[allow(dead_code)]
pub fn run_wizard_tui_no_providers() -> Result<()> {
    run_wizard_tui(&[])
}
