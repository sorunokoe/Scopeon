//! ASCII art logo for Scopeon.
//!
//! The logo is a 5-line compact design used in the splash screen,
//! help overlay, and onboarding wizard.

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::theme::Theme;

/// Returns the 5-line ASCII logo as ratatui `Line`s, coloured by theme.
pub fn logo_lines(theme: Theme) -> Vec<Line<'static>> {
    let accent = theme.accent_color();
    let dim = theme.muted_color();

    vec![
        Line::from(vec![Span::styled(
            "  ◈  ╔═╗╔═╗╔═╗╔═╗╔═╗╔═╗╔╗╔  ",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![Span::styled(
            "     ╚═╗║  ║ ║╠═╝║╣ ║ ║║║║  ",
            Style::default().fg(accent),
        )]),
        Line::from(vec![Span::styled(
            "     ╚═╝╚═╝╚═╝╩  ╚═╝╚═╝╝╚╝  ",
            Style::default().fg(accent),
        )]),
        Line::from(vec![Span::styled(
            "     AI Context Observability  ",
            Style::default().fg(dim),
        )]),
        Line::from(vec![Span::styled(
            "     for Claude Code & friends ",
            Style::default().fg(dim),
        )]),
    ]
}

/// A single-line compact logo badge for the tab bar.
pub fn logo_badge(theme: Theme) -> Span<'static> {
    Span::styled(
        " ◈ Scopeon  ",
        Style::default()
            .fg(theme.accent_color())
            .add_modifier(Modifier::BOLD),
    )
}
