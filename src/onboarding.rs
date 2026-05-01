//! First-run onboarding wizard.
//!
//! Runs once when the database is empty. Shows detected providers,
//! key shortcuts, and shell integration without fabricating telemetry.
//! After completion the wizard permanently disables itself via a flag
//! file at `~/.scopeon/onboarded`.

use anyhow::Result;
use scopeon_core::Database;
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

/// Run the wizard the first time the database is empty.
/// Subsequent runs are no-ops.
///
/// `has_provider_data` indicates whether any provider has data files available.
/// When true and the DB is still empty, the backfill is about to populate the DB,
/// so we skip the interactive wizard and mark the user as onboarded immediately.
/// This prevents the wizard from appearing when a background backfill is running.
pub fn run_wizard_if_needed(db: &Database, has_provider_data: bool) -> Result<()> {
    if is_onboarded() {
        return Ok(());
    }
    // Already has real data — skip wizard but mark as onboarded.
    let global = db.get_global_stats()?;
    if global.total_sessions > 0 {
        mark_onboarded()?;
        return Ok(());
    }
    // Providers have data files: the backfill will populate the DB shortly.
    // Skip the wizard so it doesn't appear while data is loading.
    if has_provider_data {
        mark_onboarded()?;
        return Ok(());
    }
    // Truly first-time user with no data sources and no existing sessions.
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

    // Respect CLAUDE_CONFIG_DIR override (same priority as ClaudeCodeProvider::new).
    let claude_base = std::env::var("CLAUDE_CONFIG_DIR")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(|| home.as_ref().map(|h| h.join(".claude")))
        .unwrap_or_else(|| std::path::PathBuf::from("/nonexistent"));
    let claude_detected = claude_base.join("projects").exists();
    let claude_hint = format!("{}/projects/", claude_base.display());

    vec![
        ("Claude Code".to_string(), claude_detected, claude_hint),
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
    writeln!(
        out,
        "  Scopeon starts empty on purpose — the dashboard only fills with real observed sessions."
    )?;
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

    // ── GitHub Copilot CLI MCP configuration ─────────────────────────────────
    let has_copilot = detected.iter().any(|(n, _, _)| n == "GitHub Copilot CLI");
    if has_copilot {
        write!(
            out,
            "Configure Scopeon as an MCP server in GitHub Copilot CLI? [Y/n] "
        )?;
        out.flush()?;
        let answer = read_line()?;
        if answer.trim().is_empty() || answer.trim().eq_ignore_ascii_case("y") {
            crate::cmd_init_copilot()?;
        } else {
            writeln!(
                out,
                "  Skipped. Run `scopeon init-copilot` later to configure."
            )?;
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
        "  scopeon optimize scan  — inspect provider optimization presets"
    )?;
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
    writeln!(out, "Docs: https://github.com/sorunokoe/Scopeon")?;
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
