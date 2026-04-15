//! First-run onboarding wizard.
//!
//! Runs once when the database is empty.  Shows detected providers,
//! key shortcuts, and shell integration — then seeds realistic demo
//! data so the TUI opens to a populated dashboard on first launch.
//! After completion the wizard permanently disables itself via a flag
//! file at `~/.scopeon/onboarded`.

use anyhow::Result;
use scopeon_core::{Database, Session, Turn};
use std::io::{self, Write as _};

// ── Flag file ────────────────────────────────────────────────────────────────

fn flag_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".scopeon").join("onboarded"))
}

pub fn is_onboarded() -> bool {
    flag_path().map(|p| p.exists()).unwrap_or(true)
}

fn mark_onboarded() -> Result<()> {
    if let Some(path) = flag_path() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, b"1")?;
    }
    Ok(())
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Run the wizard the first time the database is empty, then seed demo data.
/// Subsequent runs are no-ops.
pub fn run_wizard_if_needed(db: &Database) -> Result<()> {
    if is_onboarded() {
        return Ok(());
    }
    // Already has real data — skip wizard but mark as onboarded.
    let global = db.get_global_stats()?;
    if global.total_sessions > 0 {
        mark_onboarded()?;
        return Ok(());
    }
    // Seed demo data BEFORE the wizard so the TUI looks great on first open.
    seed_demo_data(db)?;
    run_wizard()?;
    mark_onboarded()?;
    Ok(())
}

// ── Wizard renderer ──────────────────────────────────────────────────────────

/// Ratatui-based paginated wizard (alternate screen, Cockpit theme).
fn run_wizard() -> Result<()> {
    let providers = detect_providers();
    scopeon_tui::wizard::run_wizard_tui(&providers)
}

// ── Provider detection ────────────────────────────────────────────────────────

fn detect_providers() -> Vec<(String, bool, String)> {
    let home = dirs::home_dir();
    let check =
        |sub: &str| -> bool { home.as_ref().map(|h| h.join(sub).exists()).unwrap_or(false) };

    vec![
        (
            "Claude Code".to_string(),
            check(".claude/projects"),
            "~/.claude/projects/".to_string(),
        ),
        (
            "GitHub Copilot CLI".to_string(),
            check(".copilot/session-state"),
            "~/.copilot/session-state/".to_string(),
        ),
        (
            "Aider".to_string(),
            check(".aider/analytics.jsonl"),
            "run with --analytics-log".to_string(),
        ),
        (
            "Gemini CLI".to_string(),
            check(".gemini/tmp"),
            "~/.gemini/tmp/*/session-*.jsonl".to_string(),
        ),
        (
            "Ollama".to_string(),
            check("Library/Application Support/Ollama/db.sqlite"),
            "local LLM via Ollama".to_string(),
        ),
        (
            "Cursor".to_string(),
            std::path::Path::new("/Applications/Cursor.app").exists()
                || check("Library/Application Support/Cursor/User/globalStorage"),
            "AI-powered editor".to_string(),
        ),
    ]
}

// ── Demo data seeder ──────────────────────────────────────────────────────────

/// Insert realistic synthetic sessions so the TUI opens to a populated
/// dashboard.  Only runs once (controlled by the flag file).
fn seed_demo_data(db: &Database) -> Result<()> {
    use scopeon_core::cost::calculate_turn_cost;

    let now_ms = chrono::Utc::now().timestamp_millis();
    let day_ms = 86_400_000i64;

    // Five sessions across the last 5 days, showing a realistic optimisation arc:
    // day 0 = cold cache → day 4 = well-warmed, high efficiency.
    type SessionSpec = (&'static str, &'static str, i64, i64, i64, i64, i64, i64);
    let sessions_spec: &[SessionSpec] = &[
        // (project, model, days_ago, input/turn, cache_read/turn, cache_write/turn, output_pct, turns)
        (
            "~/projects/api-service",
            "claude-sonnet-4-5",
            4,
            3_800,
            200,
            500,
            130,
            18,
        ),
        (
            "~/projects/api-service",
            "claude-sonnet-4-5",
            3,
            3_500,
            1_800,
            480,
            120,
            22,
        ),
        (
            "~/projects/frontend",
            "claude-3-5-haiku-20241022",
            2,
            2_200,
            1_300,
            300,
            110,
            14,
        ),
        (
            "~/projects/api-service",
            "claude-sonnet-4-5",
            1,
            3_200,
            2_600,
            420,
            105,
            26,
        ),
        (
            "~/projects/infra-tools",
            "claude-sonnet-4-5",
            0,
            2_900,
            2_000,
            380,
            115,
            19,
        ),
    ];

    for (
        idx,
        &(proj, model, days_ago, input_per, cache_read_per, cache_write_per, out_mult_pct, turns),
    ) in sessions_spec.iter().enumerate()
    {
        let session_id = format!("demo-session-{}", idx + 1);
        let base_ts = now_ms - days_ago * day_ms;
        let output_per = input_per * out_mult_pct / 100;

        let session = Session {
            id: session_id.clone(),
            project: proj.to_string(),
            project_name: proj.split('/').next_back().unwrap_or(proj).to_string(),
            slug: session_id.clone(),
            model: model.to_string(),
            git_branch: if days_ago == 0 {
                "main".to_string()
            } else {
                format!("feat/iteration-{}", idx)
            },
            started_at: base_ts - 3_600_000,
            last_turn_at: base_ts,
            total_turns: turns,
            is_subagent: false,
            parent_session_id: None,
            context_window_tokens: None,
        };
        db.upsert_session(&session)?;

        for t in 0..turns as usize {
            // Cache read ramps up over the session (cold → warm)
            let ramp = ((t as f64 / turns as f64) * 1.4 + 0.1).min(1.8);
            let cr = (cache_read_per as f64 * ramp) as i64;

            let turn_id = format!("demo-turn-{}-{}", idx, t);
            let cost = calculate_turn_cost(model, input_per, output_per, cache_write_per, cr);
            let turn = Turn {
                id: turn_id,
                session_id: session_id.clone(),
                turn_index: t as i64,
                timestamp: base_ts - (turns as usize - t) as i64 * 180_000,
                duration_ms: Some(1_800 + (t as i64 * 120)),
                input_tokens: input_per,
                cache_read_tokens: cr,
                cache_write_tokens: cache_write_per,
                cache_write_5m_tokens: 0,
                cache_write_1h_tokens: cache_write_per,
                output_tokens: output_per,
                thinking_tokens: if model.contains("sonnet") {
                    output_per / 5
                } else {
                    0
                },
                mcp_call_count: (t % 4) as i64,
                mcp_input_token_est: ((t % 4) as i64) * 250,
                text_output_tokens: output_per,
                model: model.to_string(),
                service_tier: "standard".to_string(),
                estimated_cost_usd: cost.total_usd,
                is_compaction_event: t == turns as usize / 2,
            };
            db.upsert_turn(&turn)?;
        }
    }

    db.refresh_daily_rollup()?;

    Ok(())
}

// ── Interactive `scopeon onboard` CLI wizard ──────────────────────────────────

/// Run the interactive onboard wizard from the CLI.
///
/// Detects installed AI tools, offers to configure each, and shows
/// quick-start tips for shell integration and digest reports.
pub fn cmd_onboard() -> Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();

    writeln!(
        out,
        "╔══════════════════════════════════════════════════════╗"
    )?;
    writeln!(
        out,
        "║        Scopeon — AI Context Observability Setup      ║"
    )?;
    writeln!(
        out,
        "╚══════════════════════════════════════════════════════╝"
    )?;
    writeln!(out)?;
    writeln!(out, "Detecting installed AI coding tools…")?;
    writeln!(out)?;

    let providers = detect_providers();
    let detected: Vec<_> = providers.iter().filter(|(_, found, _)| *found).collect();
    let missing: Vec<_> = providers.iter().filter(|(_, found, _)| !*found).collect();

    if detected.is_empty() {
        writeln!(out, "  ⚠  No supported AI tools detected yet.")?;
        writeln!(
            out,
            "     Install Claude Code, Aider, or Gemini CLI and re-run."
        )?;
    } else {
        writeln!(out, "  ✓ Detected:")?;
        for (name, _, path) in &detected {
            writeln!(out, "      • {name}  ({path})")?;
        }
    }

    if !missing.is_empty() {
        writeln!(out)?;
        writeln!(out, "  — Not detected (install to enable tracking):")?;
        for (name, _, path) in &missing {
            writeln!(out, "      • {name}  ({path})")?;
        }
    }
    writeln!(out)?;

    // ── Claude Code MCP configuration ────────────────────────────────────────
    let has_claude = detected.iter().any(|(n, _, _)| n == "Claude Code");
    if has_claude {
        write!(
            out,
            "Configure Scopeon as an MCP server in Claude Code? [Y/n] "
        )?;
        out.flush()?;
        let answer = read_line()?;
        if answer.trim().is_empty() || answer.trim().eq_ignore_ascii_case("y") {
            crate::cmd_init()?;
        } else {
            writeln!(out, "  Skipped. Run `scopeon init` later to configure.")?;
        }
        writeln!(out)?;
    }

    // ── Shell integration ─────────────────────────────────────────────────────
    write!(out, "Add Scopeon status to your shell prompt? [Y/n] ")?;
    out.flush()?;
    let shell_answer = read_line()?;
    if shell_answer.trim().is_empty() || shell_answer.trim().eq_ignore_ascii_case("y") {
        let shell = detect_shell();
        writeln!(out)?;
        writeln!(out, "  Add this to your shell rc file:")?;
        writeln!(
            out,
            "  ┌─────────────────────────────────────────────────────┐"
        )?;
        match shell.as_str() {
            "fish" => {
                writeln!(
                    out,
                    "  │  # ~/.config/fish/config.fish                       │"
                )?;
                writeln!(
                    out,
                    "  │  scopeon shell-hook --shell fish | source            │"
                )?;
            },
            _ => {
                writeln!(
                    out,
                    "  │  # ~/.zshrc  or  ~/.bashrc                           │"
                )?;
                writeln!(
                    out,
                    "  │  eval \"$(scopeon shell-hook)\"                         │"
                )?;
            },
        }
        writeln!(
            out,
            "  └─────────────────────────────────────────────────────┘"
        )?;
    }
    writeln!(out)?;

    // ── Git hook ──────────────────────────────────────────────────────────────
    let in_git = std::path::Path::new(".git").exists();
    if in_git {
        write!(
            out,
            "Install AI-Cost git commit trailer in this repo? [y/N] "
        )?;
        out.flush()?;
        let hook_answer = read_line()?;
        if hook_answer.trim().eq_ignore_ascii_case("y") {
            crate::git_hook::install()?;
        }
        writeln!(out)?;
    }

    // ── Quick-start tips ─────────────────────────────────────────────────────
    writeln!(
        out,
        "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    )?;
    writeln!(out, "You're all set! Quick commands:")?;
    writeln!(out)?;
    writeln!(out, "  scopeon start          — open TUI dashboard")?;
    writeln!(
        out,
        "  scopeon serve          — start API server for team sharing"
    )?;
    writeln!(out, "  scopeon digest         — weekly AI usage report")?;
    writeln!(
        out,
        "  scopeon badge          — README badges for your repo"
    )?;
    writeln!(out, "  scopeon ci snapshot    — capture baseline for PRs")?;
    writeln!(out)?;
    writeln!(out, "Docs: https://github.com/scopeon/scopeon")?;
    writeln!(
        out,
        "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    )?;

    Ok(())
}

fn read_line() -> Result<String> {
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf)
}

fn detect_shell() -> String {
    std::env::var("SHELL")
        .unwrap_or_default()
        .split('/')
        .next_back()
        .unwrap_or("bash")
        .to_string()
}
