//! Visual theme system for the Scopeon TUI.
//!
//! Provides colour selection and progress-bar rendering for three themes:
//!
//! | `theme` value    | Description                                                 |
//! |------------------|-------------------------------------------------------------|
//! | `standard`       | 16-colour ANSI palette — works everywhere.                  |
//! | `high-contrast`  | Maximum-brightness colours for accessibility.               |
//! | `cockpit`        | Aviation-grade visuals: RGB truecolour, sub-cell smooth     |
//! |                  | progress bars, rounded borders, and a pulsing crisis border |
//! |                  | at context ≥ 95%. **Default theme.**                        |
//!
//! # Configuration
//! Set in `~/.scopeon/config.toml`:
//! ```toml
//! [general]
//! theme = "cockpit"   # default
//! ```

use ratatui::{
    style::{Color, Modifier, Style},
    widgets::BorderType,
};

/// Active visual theme, derived from `[general] theme` in user config.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Theme {
    Standard,
    HighContrast,
    #[default]
    Cockpit,
}

impl Theme {
    pub fn from_name(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "cockpit" => Theme::Cockpit,
            "high-contrast" | "highcontrast" => Theme::HighContrast,
            _ => Theme::Standard,
        }
    }

    /// Colour for a health score in 0–100.
    pub fn health_color(self, score: f64) -> Color {
        match self {
            Theme::Cockpit => {
                if score >= 80.0 {
                    Color::Rgb(0, 230, 118) // emerald green
                } else if score >= 50.0 {
                    Color::Rgb(255, 196, 0) // golden amber
                } else {
                    Color::Rgb(255, 59, 48) // alarm red
                }
            },
            Theme::HighContrast => {
                if score >= 80.0 {
                    Color::LightGreen
                } else if score >= 50.0 {
                    Color::LightYellow
                } else {
                    Color::LightRed
                }
            },
            Theme::Standard => {
                if score >= 80.0 {
                    Color::Green
                } else if score >= 50.0 {
                    Color::Yellow
                } else {
                    Color::Red
                }
            },
        }
    }

    /// Colour for a context pressure percentage in 0–100.
    pub fn context_color(self, pct: f64) -> Color {
        match self {
            Theme::Cockpit => {
                if pct >= 95.0 {
                    Color::Rgb(255, 59, 48) // alarm red
                } else if pct >= 80.0 {
                    Color::Rgb(255, 140, 0) // deep amber
                } else if pct >= 50.0 {
                    Color::Rgb(255, 214, 0) // caution yellow
                } else {
                    Color::Rgb(0, 210, 130) // teal-green
                }
            },
            Theme::HighContrast => {
                if pct >= 95.0 {
                    Color::LightRed
                } else if pct >= 80.0 {
                    Color::LightYellow
                } else {
                    Color::LightGreen
                }
            },
            Theme::Standard => {
                if pct >= 80.0 {
                    Color::Red
                } else if pct >= 60.0 {
                    Color::Yellow
                } else {
                    Color::Green
                }
            },
        }
    }

    /// Colour for a cache hit-rate percentage in 0–100.
    pub fn cache_color(self, rate: f64) -> Color {
        match self {
            Theme::Cockpit => {
                if rate >= 70.0 {
                    Color::Rgb(0, 230, 118)
                } else if rate >= 40.0 {
                    Color::Rgb(255, 196, 0)
                } else {
                    Color::Rgb(255, 59, 48)
                }
            },
            Theme::HighContrast => {
                if rate >= 70.0 {
                    Color::LightGreen
                } else if rate >= 40.0 {
                    Color::LightYellow
                } else {
                    Color::LightRed
                }
            },
            Theme::Standard => {
                if rate >= 70.0 {
                    Color::Green
                } else if rate >= 40.0 {
                    Color::Yellow
                } else {
                    Color::Red
                }
            },
        }
    }

    /// Render a filled progress bar string of the given `width` (in terminal columns).
    ///
    /// - `Standard` / `HighContrast`: classic `█░` binary fill.
    /// - `Cockpit`: smooth 8-step sub-cell fill using Unicode block elements
    ///   (`▏▎▍▌▋▊▉█`) for sub-character precision — creates a smoother, more
    ///   instrumentation-grade look at any fill level.
    pub fn progress_bar(self, ratio: f64, width: usize) -> String {
        let ratio = ratio.clamp(0.0, 1.0);
        match self {
            Theme::Cockpit => cockpit_bar(ratio, width),
            _ => standard_bar(ratio, width),
        }
    }

    /// Border style to apply to ratatui `Block` widgets, escalated by context pressure.
    ///
    /// In `Cockpit` mode: borders pulse red at ≥ 95% (SLOW_BLINK + alarm red),
    /// and turn amber at ≥ 80%. Other themes return the default empty style.
    pub fn crisis_border_style(self, context_pct: f64) -> Style {
        match self {
            Theme::Cockpit if context_pct >= 95.0 => Style::default()
                .fg(Color::Rgb(255, 59, 48))
                .add_modifier(Modifier::SLOW_BLINK),
            Theme::Cockpit if context_pct >= 80.0 => Style::default().fg(Color::Rgb(255, 140, 0)),
            _ => Style::default(),
        }
    }

    /// Accent colour for the tab bar logo and active-tab highlight.
    pub fn accent_color(self) -> Color {
        match self {
            Theme::Cockpit => Color::Rgb(0, 200, 255), // cool cyan
            Theme::HighContrast => Color::LightCyan,
            Theme::Standard => Color::Cyan,
        }
    }

    /// Dimmed accent — used for secondary highlights and unfocused borders.
    pub fn accent_dim(self) -> Color {
        match self {
            Theme::Cockpit => Color::Rgb(0, 120, 160),
            Theme::HighContrast => Color::Cyan,
            Theme::Standard => Color::DarkGray,
        }
    }

    /// Primary text colour.
    pub fn text_primary(self) -> Color {
        match self {
            Theme::Cockpit | Theme::Standard => Color::White,
            Theme::HighContrast => Color::White,
        }
    }

    /// Secondary / label text colour.
    pub fn text_secondary(self) -> Color {
        match self {
            Theme::Cockpit => Color::Rgb(160, 170, 180),
            Theme::HighContrast => Color::LightCyan,
            Theme::Standard => Color::Gray,
        }
    }

    /// Muted / placeholder text colour.
    pub fn muted_color(self) -> Color {
        match self {
            Theme::Cockpit => Color::Rgb(80, 90, 100),
            Theme::HighContrast => Color::DarkGray,
            Theme::Standard => Color::DarkGray,
        }
    }

    /// Section / column heading colour.
    pub fn heading_color(self) -> Color {
        match self {
            Theme::Cockpit => Color::Rgb(255, 196, 0), // golden amber
            Theme::HighContrast => Color::LightYellow,
            Theme::Standard => Color::Yellow,
        }
    }

    /// Success / positive value colour.
    pub fn success_color(self) -> Color {
        match self {
            Theme::Cockpit => Color::Rgb(0, 230, 118),
            Theme::HighContrast => Color::LightGreen,
            Theme::Standard => Color::Green,
        }
    }

    /// Warning colour.
    pub fn warning_color(self) -> Color {
        match self {
            Theme::Cockpit => Color::Rgb(255, 196, 0),
            Theme::HighContrast => Color::LightYellow,
            Theme::Standard => Color::Yellow,
        }
    }

    /// Error / critical colour.
    pub fn error_color(self) -> Color {
        match self {
            Theme::Cockpit => Color::Rgb(255, 59, 48),
            Theme::HighContrast => Color::LightRed,
            Theme::Standard => Color::Red,
        }
    }

    /// Cost / monetary value colour.
    pub fn cost_color(self) -> Color {
        match self {
            Theme::Cockpit => Color::Rgb(220, 140, 255), // soft violet
            Theme::HighContrast => Color::LightMagenta,
            Theme::Standard => Color::Magenta,
        }
    }

    /// Model / provider name colour.
    pub fn model_color(self) -> Color {
        match self {
            Theme::Cockpit => Color::Rgb(80, 220, 120),
            Theme::HighContrast => Color::LightGreen,
            Theme::Standard => Color::Green,
        }
    }

    /// Border type appropriate for this theme.
    pub fn border_type(self) -> BorderType {
        match self {
            Theme::Cockpit => BorderType::Rounded,
            Theme::HighContrast => BorderType::Thick,
            Theme::Standard => BorderType::Plain,
        }
    }

    /// Border style for an *active* / focused pane.
    pub fn active_border_style(self) -> Style {
        Style::default()
            .fg(self.accent_color())
            .add_modifier(Modifier::BOLD)
    }

    /// Border style for an *inactive* / unfocused pane.
    pub fn inactive_border_style(self) -> Style {
        Style::default().fg(self.muted_color())
    }
}

// ── Progress bar renderers ─────────────────────────────────────────────────────

/// Binary progress bar: `█` fill + `░` empty.
fn standard_bar(ratio: f64, width: usize) -> String {
    let filled = (ratio * width as f64).round() as usize;
    "█".repeat(filled.min(width)) + &"░".repeat(width.saturating_sub(filled))
}

/// Smooth sub-cell progress bar using 8-step Unicode block elements.
///
/// Each character position can show one of 9 states (empty or 8 fill levels),
/// giving `width × 8` distinct positions — much finer than the binary approach.
///
/// Characters used: `░` (empty) and `▏▎▍▌▋▊▉█` (1/8 → 8/8 fill).
fn cockpit_bar(ratio: f64, width: usize) -> String {
    const PARTIALS: [char; 8] = ['▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];

    let total_eighths = (ratio * (width * 8) as f64).round() as usize;
    let full_blocks = (total_eighths / 8).min(width);
    let remainder = total_eighths % 8;
    let has_partial = remainder > 0 && full_blocks < width;
    let empty_blocks = width
        .saturating_sub(full_blocks)
        .saturating_sub(usize::from(has_partial));

    let mut s = String::with_capacity(width * 3); // Unicode chars ≤ 3 bytes each
    for _ in 0..full_blocks {
        s.push('█');
    }
    if has_partial {
        s.push(PARTIALS[remainder - 1]);
    }
    for _ in 0..empty_blocks {
        s.push('░');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cockpit_bar_full() {
        let bar = cockpit_bar(1.0, 10);
        assert_eq!(bar.chars().count(), 10);
        assert!(bar.chars().all(|c| c == '█'));
    }

    #[test]
    fn cockpit_bar_empty() {
        let bar = cockpit_bar(0.0, 10);
        assert_eq!(bar.chars().count(), 10);
        assert!(bar.chars().all(|c| c == '░'));
    }

    #[test]
    fn cockpit_bar_half() {
        let bar = cockpit_bar(0.5, 10);
        assert_eq!(bar.chars().count(), 10);
    }

    #[test]
    fn standard_bar_full() {
        let bar = standard_bar(1.0, 8);
        assert_eq!(bar, "████████");
    }
}
