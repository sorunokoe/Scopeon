use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::app::App;
use crate::theme::Theme;

pub fn render_config(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    // Split into left (provider list) and right (instructions/preset selector)
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    render_provider_list(frame, app, chunks[0], theme);

    if app.config_preset_selector_active {
        render_preset_selector(frame, app, chunks[1], theme);
    } else {
        render_instructions(frame, app, chunks[1], theme);
    }
}

fn render_provider_list(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let items: Vec<ListItem> = app
        .config_providers
        .iter()
        .enumerate()
        .map(|(idx, provider)| {
            let is_selected = idx == app.config_selected_idx;
            let detected_indicator = if provider.detected { "✓" } else { "✗" };
            let preset_display = provider
                .current_preset
                .as_ref()
                .map(|p| format!(" ({})", p))
                .unwrap_or_else(|| " (not configured)".to_string());

            let style = if is_selected {
                Style::default()
                    .fg(theme.accent_color())
                    .add_modifier(Modifier::BOLD)
                    .bg(Color::DarkGray)
            } else if !provider.detected {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default().fg(theme.text_primary())
            };

            let line = Line::from(vec![
                Span::styled(
                    format!(" {} ", detected_indicator),
                    Style::default().fg(if provider.detected {
                        Color::Green
                    } else {
                        Color::Red
                    }),
                ),
                Span::styled(&provider.display_name, style),
                Span::styled(preset_display, Style::default().fg(Color::Gray)),
            ]);

            ListItem::new(line)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" AI Providers ")
            .border_style(theme.inactive_border_style()),
    );

    frame.render_widget(list, area);
}

fn render_instructions(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let provider = app
        .config_providers
        .get(app.config_selected_idx)
        .map(|p| p.display_name.as_str())
        .unwrap_or("None");

    let current_preset = app
        .config_providers
        .get(app.config_selected_idx)
        .and_then(|p| p.current_preset.as_ref())
        .map(|p| format!("Currently: {}", p))
        .unwrap_or_else(|| "Not configured".to_string());

    let detected = app
        .config_providers
        .get(app.config_selected_idx)
        .map(|p| p.detected)
        .unwrap_or(false);

    let text = if detected {
        vec![
            Line::from(vec![
                Span::styled("Selected: ", Style::default().fg(Color::Gray)),
                Span::styled(provider, Style::default().fg(theme.accent_color())),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                current_preset,
                Style::default().fg(Color::Cyan),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Press Enter to select optimization preset",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled("Presets:", Style::default().fg(Color::Gray))),
            Line::from("  • most-savings - Minimize cost"),
            Line::from("  • balanced - Recommended default"),
            Line::from("  • most-speed - Fastest responses"),
            Line::from("  • most-power - Best quality"),
            Line::from(""),
            Line::from(Span::styled(
                "Navigation:",
                Style::default().fg(Color::Gray),
            )),
            Line::from("  ↑/↓ - Select provider"),
            Line::from("  Enter - Open preset selector"),
            Line::from("  q - Return to main view"),
        ]
    } else {
        vec![
            Line::from(vec![
                Span::styled("Selected: ", Style::default().fg(Color::Gray)),
                Span::styled(provider, Style::default().fg(theme.accent_color())),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "⚠ Provider not detected",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from("This AI tool is not installed or not"),
            Line::from("found in standard locations."),
            Line::from(""),
            Line::from("Install it first, then return here"),
            Line::from("to configure optimization presets."),
        ]
    };

    let para = Paragraph::new(text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Instructions ")
                .border_style(theme.inactive_border_style()),
        )
        .wrap(Wrap { trim: true })
        .alignment(Alignment::Left);

    frame.render_widget(para, area);
}

fn render_preset_selector(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    // Split into selector (left) and details (right)
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    // Get provider and its presets
    let provider = app.config_providers.get(app.config_selected_idx);
    let provider_name = provider.map(|p| p.display_name.as_str()).unwrap_or("None");
    let presets = provider
        .map(|p| &p.presets)
        .map(|v| v.as_slice())
        .unwrap_or(&[]);

    // Left: Preset list
    let items: Vec<ListItem> = presets
        .iter()
        .enumerate()
        .map(|(idx, preset)| {
            let is_selected = idx == app.config_preset_selected_idx;
            let style = if is_selected {
                Style::default()
                    .fg(theme.accent_color())
                    .add_modifier(Modifier::BOLD)
                    .bg(Color::DarkGray)
            } else {
                Style::default().fg(theme.text_primary())
            };

            let line = Line::from(vec![
                Span::styled(if is_selected { " ▶ " } else { "   " }, style),
                Span::styled(&preset.title, style),
            ]);

            ListItem::new(line)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} Presets ", provider_name))
            .border_style(Style::default().fg(Color::Cyan)),
    );

    frame.render_widget(list, chunks[0]);

    // Right: Selected preset details with wrapping
    if let Some(preset) = presets.get(app.config_preset_selected_idx) {
        let mut lines = Vec::new();

        // Title
        lines.push(Line::from(Span::styled(
            &preset.title,
            Style::default()
                .fg(theme.accent_color())
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));

        // Summary
        lines.push(Line::from(Span::styled(
            &preset.summary,
            Style::default().fg(theme.text_primary()),
        )));
        lines.push(Line::from(""));

        // Trade-off
        lines.push(Line::from(Span::styled(
            "Trade-off:",
            Style::default().fg(Color::Yellow),
        )));
        lines.push(Line::from(Span::styled(
            &preset.tradeoff,
            Style::default().fg(Color::Gray),
        )));
        lines.push(Line::from(""));

        // Command preview
        lines.push(Line::from(Span::styled(
            "Command:",
            Style::default().fg(Color::Green),
        )));
        // Wrap long commands
        let cmd_words: Vec<&str> = preset.command_preview.split_whitespace().collect();
        let mut current_line = String::new();
        for word in cmd_words {
            if current_line.len() + word.len() + 1 > (chunks[1].width as usize - 4) {
                if !current_line.is_empty() {
                    lines.push(Line::from(Span::styled(
                        current_line.clone(),
                        Style::default().fg(Color::Cyan),
                    )));
                    current_line.clear();
                    current_line.push_str("  ");
                }
            }
            if !current_line.is_empty() {
                current_line.push(' ');
            }
            current_line.push_str(word);
        }
        if !current_line.is_empty() {
            lines.push(Line::from(Span::styled(
                current_line,
                Style::default().fg(Color::Cyan),
            )));
        }
        lines.push(Line::from(""));

        // Optimizations
        if !preset.optimizations.is_empty() {
            lines.push(Line::from(Span::styled(
                "What it does:",
                Style::default().fg(Color::Magenta),
            )));
            for opt in &preset.optimizations {
                lines.push(Line::from(Span::styled(
                    format!("• {}", opt),
                    Style::default().fg(Color::Gray),
                )));
            }
        }

        let text = Text::from(lines);
        let para = Paragraph::new(text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Details ")
                    .border_style(Style::default().fg(Color::Cyan)),
            )
            .wrap(Wrap { trim: false })
            .scroll((0, 0));

        frame.render_widget(para, chunks[1]);
    }

    // Show help at bottom
    let help_area = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(1),
        width: area.width,
        height: 1,
    };

    if help_area.y < frame.area().height {
        let help = Paragraph::new(Line::from(vec![
            Span::styled("↑/↓", Style::default().fg(Color::Green)),
            Span::raw(" Select  "),
            Span::styled("Enter", Style::default().fg(Color::Green)),
            Span::raw(" Apply  "),
            Span::styled("Esc", Style::default().fg(Color::Red)),
            Span::raw(" Cancel"),
        ]))
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::Gray));

        frame.render_widget(help, help_area);
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
