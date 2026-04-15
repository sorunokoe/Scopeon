/// Tab 6: Multi-agent subagent tree
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use scopeon_core::AgentNode;

use crate::app::App;
use crate::views::components::themed_block;

pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    draw_header(f, app, chunks[0]);
    draw_tree(f, app, chunks[1]);
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let root_count = app.agent_roots.len();
    let total_agents: usize = app.agent_roots.iter().map(count_nodes).sum();
    let total_cost: f64 = app.agent_roots.iter().map(subtree_cost).sum();

    let line = if root_count == 0 {
        Line::from(Span::styled(
            "  No multi-agent sessions detected. Subagent trees appear when one session spawns children.",
            Style::default().fg(app.theme.muted_color()),
        ))
    } else {
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                format!("{} root session(s)", root_count),
                Style::default()
                    .fg(app.theme.accent_color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ·  ", Style::default().fg(app.theme.muted_color())),
            Span::styled(
                format!("{} total agents", total_agents),
                Style::default().fg(app.theme.text_primary()),
            ),
            Span::styled(
                "  ·  total cost: ",
                Style::default().fg(app.theme.muted_color()),
            ),
            Span::styled(
                format!("${:.4}", total_cost),
                Style::default()
                    .fg(app.theme.cost_color())
                    .add_modifier(Modifier::BOLD),
            ),
        ])
    };

    let para = Paragraph::new(line).block(themed_block(app.theme, "Agent Trees", false));
    f.render_widget(para, area);
}

fn draw_tree(f: &mut Frame, app: &App, area: Rect) {
    if app.agent_roots.is_empty() {
        let msg = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  How subagent trees work:",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from("  When an AI agent (e.g. Claude Code) spawns sub-tasks using Task() or"),
            Line::from("  similar constructs, each sub-task creates a new session with a"),
            Line::from("  parent_session_id pointing back to the root session."),
            Line::from(""),
            Line::from("  Scopeon detects these relationships automatically and renders the"),
            Line::from("  full cost tree here — so you can see which sub-agents cost the most."),
            Line::from(""),
            Line::from(Span::styled(
                "  Run a multi-agent Claude Code task to see trees appear here.",
                Style::default().fg(Color::DarkGray),
            )),
        ])
        .block(themed_block(app.theme, "Tree View", false));
        f.render_widget(msg, area);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    for root in &app.agent_roots {
        render_node(root, "", true, &mut lines);
    }

    let para = Paragraph::new(lines).block(themed_block(app.theme, "Tree View", false));
    f.render_widget(para, area);
}

fn render_node<'a>(node: &'a AgentNode, prefix: &str, is_last: bool, lines: &mut Vec<Line<'a>>) {
    let connector = if is_last { "└─ " } else { "├─ " };
    let cost_color = if node.total_cost_usd > 0.10 {
        Color::Red
    } else if node.total_cost_usd > 0.01 {
        Color::Yellow
    } else {
        Color::Green
    };

    let kind_label = if node.is_subagent { "sub" } else { "root" };
    let kind_color = if node.is_subagent {
        Color::Yellow
    } else {
        Color::Cyan
    };

    let short_id = &node.session_id[..node.session_id.len().min(8)];
    let short_model = short_model(&node.model);

    lines.push(Line::from(vec![
        Span::styled(
            format!("{}{}", prefix, connector),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            format!("[{}] ", kind_label),
            Style::default().fg(kind_color),
        ),
        Span::styled(short_id, Style::default().fg(Color::Cyan)),
        Span::styled(
            format!("  {}", node.project_name),
            Style::default().fg(Color::White),
        ),
        Span::styled(
            format!("  {}", short_model),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            format!("  {} turns", node.turn_count),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            format!("  ${:.4}", node.total_cost_usd),
            Style::default().fg(cost_color).add_modifier(Modifier::BOLD),
        ),
    ]));

    let child_prefix = format!("{}{}  ", prefix, if is_last { "   " } else { "│  " });
    for (i, child) in node.children.iter().enumerate() {
        let child_is_last = i == node.children.len() - 1;
        render_node(child, &child_prefix, child_is_last, lines);
    }
}

fn count_nodes(node: &AgentNode) -> usize {
    1 + node.children.iter().map(count_nodes).sum::<usize>()
}

fn subtree_cost(node: &AgentNode) -> f64 {
    node.total_cost_usd + node.children.iter().map(subtree_cost).sum::<f64>()
}

fn short_model(model: &str) -> String {
    let m = model.to_lowercase();
    if let Some(s) = m.strip_prefix("claude-") {
        let parts: Vec<&str> = s.splitn(3, '-').collect();
        if parts.len() >= 2 {
            return format!("{}-{}", parts[0], parts[1]);
        }
    }
    if model.len() > 12 {
        model[..12].to_string()
    } else {
        model.to_string()
    }
}
