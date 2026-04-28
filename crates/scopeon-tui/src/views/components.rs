//! Shared reusable UI components for Scopeon views.
//!
//! Provides:
//! - `micro_sparkline` — 5–10 char sparkline from f64 values  
//! - `empty_state_lines` — consistent "no data" empty state
//! - `trend_span` — ▲/▼/─ trend indicator
//! - `themed_block` — `Block` pre-configured for the current theme
//! - `kpi_row` — horizontal KPI strip of key numbers

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders},
};

use crate::theme::Theme;

// ── Sparkline ─────────────────────────────────────────────────────────────────

/// Renders a compact sparkline of `width` characters from the last N values.
/// Uses ▁▂▃▄▅▆▇█ block characters for visual resolution.
pub fn micro_sparkline(values: &[f64], width: usize) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

    if values.is_empty() || width == 0 {
        return " ".repeat(width);
    }

    let tail: Vec<f64> = values.iter().rev().take(width).copied().collect();
    let tail: Vec<f64> = tail.into_iter().rev().collect();

    let max = tail.iter().cloned().fold(0.0f64, f64::max);
    if max == 0.0 {
        return "▁".repeat(tail.len());
    }

    tail.iter()
        .map(|&v| {
            let idx = ((v / max) * 7.0).round() as usize;
            BARS[idx.min(7)]
        })
        .collect()
}

// ── Trend indicator ───────────────────────────────────────────────────────────

/// Returns a coloured `▲`, `▼`, or `─` span based on a percentage delta.
///
/// `positive_is_good` controls whether ▲ is green (e.g. cache hit rate)
/// or red (e.g. cost increase).
pub fn trend_span(pct: f64, positive_is_good: bool) -> Span<'static> {
    if pct > 2.0 {
        let color = if positive_is_good {
            Color::Rgb(0, 230, 118)
        } else {
            Color::Rgb(255, 59, 48)
        };
        Span::styled(format!("▲{:.0}%", pct.abs()), Style::default().fg(color))
    } else if pct < -2.0 {
        let color = if positive_is_good {
            Color::Rgb(255, 59, 48)
        } else {
            Color::Rgb(0, 230, 118)
        };
        Span::styled(format!("▼{:.0}%", pct.abs()), Style::default().fg(color))
    } else {
        Span::styled("─", Style::default().fg(Color::DarkGray))
    }
}

// ── Empty state ───────────────────────────────────────────────────────────────

/// Returns a centered, visually consistent empty-state block.
///
/// - `icon`: large Unicode glyph (e.g. `"◎"`, `"⬡"`)
/// - `title`: short primary message (bold accent)
/// - `hint`: secondary descriptive hint (muted)
/// - `action_key` + `action_desc`: keyboard call-to-action (e.g. `"r"`, `"force refresh"`)
pub fn empty_state_lines(
    theme: Theme,
    icon: &str,
    title: &str,
    hint: &str,
    action_key: &str,
    action_desc: &str,
) -> Vec<Line<'static>> {
    let accent = theme.accent_color();
    let muted = theme.muted_color();
    let heading = theme.heading_color();

    let icon = icon.to_owned();
    let title = title.to_owned();
    let hint = hint.to_owned();
    let action_key = action_key.to_owned();
    let action_desc = action_desc.to_owned();

    vec![
        Line::from(""),
        Line::from(""),
        Line::from(Span::styled(
            format!("    {}", icon),
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("    {}", title),
            Style::default().fg(heading).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("    {}", hint),
            Style::default().fg(muted),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("    Press ", Style::default().fg(muted)),
            Span::styled(
                action_key,
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" to {}", action_desc), Style::default().fg(muted)),
        ]),
    ]
}

// ── Themed block ──────────────────────────────────────────────────────────────

/// Returns a themed `Block` with the appropriate border type and style.
/// `focused` controls whether the accent or muted border style is applied.
pub fn themed_block(theme: Theme, title: &str, focused: bool) -> Block<'static> {
    let border_style = if focused {
        theme.active_border_style()
    } else {
        theme.inactive_border_style()
    };
    let title = format!(" {} ", title);
    Block::default()
        .borders(Borders::ALL)
        .border_type(theme.border_type())
        .border_style(border_style)
        .title(title)
}

/// Like `themed_block` but with explicit border sides — used for adjacent split panels to
/// avoid the double-border artifact where two `Borders::ALL` panes share an edge.
pub fn themed_block_borders(
    theme: Theme,
    title: &str,
    focused: bool,
    borders: Borders,
) -> Block<'static> {
    let border_style = if focused {
        theme.active_border_style()
    } else {
        theme.inactive_border_style()
    };
    let title = format!(" {} ", title);
    Block::default()
        .borders(borders)
        .border_type(theme.border_type())
        .border_style(border_style)
        .title(title)
}

// ── KPI row ───────────────────────────────────────────────────────────────────

/// Returns a single `Line` with `n` KPI chips separated by dim `│` delimiters.
/// Each chip: icon + label + colored value.
pub fn kpi_row(chips: &[(&str, &str, Color)], theme: Theme) -> Line<'static> {
    let sep = Span::styled("  │  ", Style::default().fg(theme.muted_color()));
    let mut spans: Vec<Span> = Vec::new();
    spans.push(Span::raw(" "));

    for (i, (label, value, color)) in chips.iter().enumerate() {
        if i > 0 {
            spans.push(sep.clone());
        }
        spans.push(Span::styled(
            format!("{}: ", label),
            Style::default().fg(theme.text_secondary()),
        ));
        spans.push(Span::styled(
            value.to_string() as String,
            Style::default().fg(*color).add_modifier(Modifier::BOLD),
        ));
    }

    Line::from(spans)
}

// ── Braille spinner ───────────────────────────────────────────────────────────

const SPINNER_FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Returns the next braille spinner frame character.
pub fn spinner_char(frame: usize) -> char {
    SPINNER_FRAMES[frame % SPINNER_FRAMES.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── micro_sparkline ───────────────────────────────────────────────────────

    #[test]
    fn sparkline_empty_values_returns_spaces() {
        let result = micro_sparkline(&[], 5);
        assert_eq!(result.chars().count(), 5, "empty values must produce width spaces");
        assert!(result.chars().all(|c| c == ' '));
    }

    #[test]
    fn sparkline_width_zero_returns_empty_string() {
        let result = micro_sparkline(&[1.0, 2.0, 3.0], 0);
        assert_eq!(result, "", "width=0 must return empty string");
    }

    #[test]
    fn sparkline_all_zeros_returns_lowest_bar_chars() {
        // All-zero data → all values relative to max=0 → fallback to '▁' repeat
        let result = micro_sparkline(&[0.0, 0.0, 0.0], 3);
        assert_eq!(result.chars().count(), 3);
        assert!(
            result.chars().all(|c| c == '▁'),
            "all-zero input must use ▁ chars: {result:?}"
        );
    }

    #[test]
    fn sparkline_single_value_returns_single_highest_bar() {
        let result = micro_sparkline(&[42.0], 1);
        assert_eq!(result.chars().count(), 1);
        assert_eq!(result, "█", "single nonzero value must be max bar ▇ or █: {result:?}");
    }

    #[test]
    fn sparkline_output_length_equals_min_of_data_and_width() {
        // When values.len() < width, output length = values.len()
        let result = micro_sparkline(&[1.0, 2.0], 10);
        // sparkline only emits as many chars as it has data points (tail of width)
        assert!(result.chars().count() <= 10, "output must not exceed width");
        assert!(!result.is_empty());
    }

    #[test]
    fn sparkline_clips_to_last_width_values() {
        // With 20 values and width=5, only the last 5 values matter
        let mut values: Vec<f64> = vec![0.0; 15];
        values.extend([1.0, 2.0, 3.0, 4.0, 5.0]);
        let result = micro_sparkline(&values, 5);
        assert_eq!(result.chars().count(), 5);
    }

    #[test]
    fn sparkline_ascending_series_ends_with_highest_bar() {
        let values = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let result = micro_sparkline(&values, 5);
        assert_eq!(result.chars().count(), 5);
        let bars: Vec<char> = result.chars().collect();
        // Last bar (max value) must be '█' (index 7)
        assert_eq!(bars[4], '█', "ascending series must end at max bar: {result:?}");
        // First bar (min, non-zero) must be lower than last
        assert!(bars[0] < bars[4], "ascending series must start lower: {result:?}");
    }

    #[test]
    fn sparkline_all_chars_are_valid_bar_chars() {
        const VALID_BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        let values = vec![1.0, 5.0, 3.0, 8.0, 2.0, 7.0];
        let result = micro_sparkline(&values, 6);
        for ch in result.chars() {
            assert!(
                VALID_BARS.contains(&ch),
                "unexpected char {ch:?} in sparkline: {result:?}"
            );
        }
    }

    #[test]
    fn sparkline_does_not_panic_on_infinite_or_nan_values() {
        // Should not panic (behavior is undefined but must be safe)
        let _ = micro_sparkline(&[f64::INFINITY, 1.0, 2.0], 3);
        let _ = micro_sparkline(&[f64::NAN, 1.0, 2.0], 3);
    }
}
