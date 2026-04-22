//! Shell integration — ambient AI observability status in your terminal prompt.
//!
//! # Setup
//! ```bash
//! # bash or zsh — add to ~/.bashrc / ~/.zshrc:
//! eval "$(scopeon shell-hook)"
//!
//! # fish — add to ~/.config/fish/config.fish:
//! scopeon shell-hook --shell fish | source
//! ```
//!
//! # Result
//! After setup, `$SCOPEON_STATUS` is refreshed on every prompt.
//! Example value: `\e[92m⬡87\e[0m \e[37m73%\e[0m $2.41`
//!
//! Add it to your prompt:
//! - zsh:  `RPROMPT='${SCOPEON_STATUS}'`
//! - bash: `PS1="${SCOPEON_STATUS} ${PS1}"`
//! - fish: include `$SCOPEON_STATUS` in your `fish_prompt` function

use anyhow::Result;
use scopeon_core::{context_pressure_with_window, Config, Database};

/// Path to the atomic status file written by the TUI daemon on every refresh.
///
/// TRIZ D1: Shell hooks read this file (<1ms) instead of forking a subprocess.
pub fn status_file_path() -> std::path::PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("scopeon")
        .join("status")
}

/// Write the current shell status to `~/.cache/scopeon/status` atomically.
///
/// Called by the TUI on every `refresh()` tick so the shell hook never forks.
/// Uses write-to-tmp → rename for atomic update (no partial reads by the shell).
pub fn write_status_file(content: &str) {
    let path = status_file_path();
    if let Some(dir) = path.parent() {
        // Create dir silently — may already exist.
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, content).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Build the shell status string from a live database (used as fallback by
/// `scopeon shell-status` and for the initial status file population).
pub fn build_status_string(config: &Config) -> String {
    if !config.db_path.exists() {
        return String::new();
    }
    let db = match Database::open(&config.db_path) {
        Ok(db) => db,
        Err(_) => return String::new(),
    };
    let (health, ctx_pct, daily_cost) = match quick_metrics(&db) {
        Ok(m) => m,
        Err(_) => return String::new(),
    };
    let (h_open, h_close) = ansi_health(health);
    let (c_open, c_close) = ansi_ctx(ctx_pct);
    format!(
        "{h_open}⬡{:.0}{h_close} {c_open}{:.0}%{c_close} \x1b[35m${:.2}\x1b[0m",
        health, ctx_pct, daily_cost
    )
}

/// Print a compact ANSI-coloured one-liner for use in shell prompts.
///
/// Format: `⬡87 73% $2.41`  (health · context% · daily cost)
///
/// First tries to read from the status file (written by the TUI daemon, <1ms).
/// Falls back to querying the DB directly if the file is absent.
/// Returns silently with no output if the DB is not found or locked.
pub fn cmd_shell_status(config: &Config) -> Result<()> {
    // Fast path: read pre-written status file (no subprocess, no DB lock).
    let file_path = status_file_path();
    if file_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&file_path) {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                print!("{}", trimmed);
                return Ok(());
            }
        }
    }

    // Fallback: query DB directly (first run / TUI not active).
    let s = build_status_string(config);
    if !s.is_empty() {
        // Opportunistically populate the status file so future hook reads hit the fast path.
        write_status_file(&s);
        print!("{}", s);
    }
    Ok(())
}
pub fn cmd_shell_hook(shell: &str) -> Result<()> {
    let exe = current_exe_path();
    let shell = if shell == "auto" {
        detect_shell()
    } else {
        shell.to_string()
    };
    match shell.as_str() {
        "fish" => print!("{}", fish_hook(&exe)),
        "zsh" => print!("{}", zsh_hook(&exe)),
        _ => print!("{}", bash_hook(&exe)),
    }
    Ok(())
}

// ── Metric computation ─────────────────────────────────────────────────────────

fn quick_metrics(db: &Database) -> Result<(f64, f64, f64)> {
    // §1.1: Use targeted queries instead of full session stats.
    let (ctx_pct, cache_pct) = if let Some(sid) = db.get_latest_session_id()? {
        let agg = db.get_session_aggregates(&sid).ok();
        let last = db.get_last_turn_for_session(&sid).ok().flatten();
        let model = agg
            .as_ref()
            .and_then(|s| s.session.as_ref())
            .map(|s| s.model.clone())
            .unwrap_or_else(|| "claude-3-5-sonnet".to_string());
        let stored_window = agg
            .as_ref()
            .and_then(|s| s.session.as_ref())
            .and_then(|s| s.context_window_tokens);
        let ctx = last
            .map(|t| {
                // §8.2: use stored context window if available
                let (pct, _) = context_pressure_with_window(
                    &model,
                    t.input_tokens + t.cache_read_tokens,
                    stored_window,
                );
                pct
            })
            .unwrap_or(0.0);
        let cache = agg.map(|s| s.cache_hit_rate * 100.0).unwrap_or(0.0);
        (ctx, cache)
    } else {
        (0.0, 0.0)
    };

    // Today's cost from the daily rollup table.
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let global = db.get_global_stats()?;
    let daily_cost = global
        .daily
        .iter()
        .find(|r| r.date == today)
        .map(|r| r.estimated_cost_usd)
        .unwrap_or(0.0);

    // Health proxy: blend of cache efficiency and spare context headroom.
    let spare_ctx = (100.0_f64 - ctx_pct).clamp(0.0, 100.0);
    let health = (cache_pct * 0.5 + spare_ctx * 0.5).clamp(0.0, 100.0);

    Ok((health, ctx_pct, daily_cost))
}

// ── ANSI colour helpers ────────────────────────────────────────────────────────

fn ansi_health(score: f64) -> (&'static str, &'static str) {
    if score >= 80.0 {
        ("\x1b[92m", "\x1b[0m") // bright green
    } else if score >= 50.0 {
        ("\x1b[93m", "\x1b[0m") // bright yellow
    } else {
        ("\x1b[91m", "\x1b[0m") // bright red
    }
}

fn ansi_ctx(pct: f64) -> (&'static str, &'static str) {
    if pct >= 95.0 {
        ("\x1b[91m", "\x1b[0m") // bright red — critical
    } else if pct >= 80.0 {
        ("\x1b[93m", "\x1b[0m") // yellow — warning
    } else {
        ("\x1b[37m", "\x1b[0m") // white — normal
    }
}

// ── Shell detection ────────────────────────────────────────────────────────────

/// Wrap a filesystem path in POSIX single-quotes so it is safe to embed in
/// generated shell code for sh, bash, and zsh regardless of spaces, `$`,
/// backticks, or other metacharacters.  The only character that cannot appear
/// inside single-quotes is the single-quote itself; we escape it as `'\''`.
fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Wrap a filesystem path in fish double-quotes.
/// Fish single-quotes do not support backslash escapes, so we use double
/// quotes and escape the three characters that are special inside them:
/// `\` (escape char), `"` (quote terminator), and `$` (variable expansion).
fn shell_quote_fish(s: &str) -> String {
    let escaped = s
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$");
    format!("\"{}\"", escaped)
}

fn detect_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .and_then(|s| {
            std::path::Path::new(&s)
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.to_string())
        })
        .unwrap_or_else(|| "bash".to_string())
}

fn current_exe_path() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "scopeon".to_string())
}

// ── Shell integration snippets ─────────────────────────────────────────────────

fn bash_hook(exe: &str) -> String {
    // TRIZ D1: read pre-written status file (<1ms) instead of forking scopeon each prompt.
    // Falls back to scopeon shell-status on first run (file not yet written by TUI).
    let status_path = status_file_path();
    let exe_q = shell_single_quote(exe);
    let status_q = shell_single_quote(&status_path.to_string_lossy());
    format!(
        r#"# Scopeon ambient status — refresh $SCOPEON_STATUS on every prompt.
# Added to ~/.bashrc via:  eval "$(scopeon shell-hook)"
# Uses pre-written status file for zero-fork, zero-latency prompt updates.
_scopeon_refresh() {{
  SCOPEON_STATUS="$(cat {status_q} 2>/dev/null || {exe_q} shell-status 2>/dev/null || true)"
}}
if [[ -n "$PROMPT_COMMAND" ]]; then
  PROMPT_COMMAND="_scopeon_refresh; $PROMPT_COMMAND"
else
  PROMPT_COMMAND="_scopeon_refresh"
fi
# To add to your prompt:
#   PS1="${{SCOPEON_STATUS}} ${{PS1}}"
"#
    )
}

fn zsh_hook(exe: &str) -> String {
    let status_path = status_file_path();
    let exe_q = shell_single_quote(exe);
    let status_q = shell_single_quote(&status_path.to_string_lossy());
    format!(
        r#"# Scopeon ambient status — refresh $SCOPEON_STATUS on every prompt.
# Added to ~/.zshrc via:  eval "$(scopeon shell-hook)"
# Uses pre-written status file for zero-fork, zero-latency prompt updates.
autoload -Uz add-zsh-hook
_scopeon_refresh() {{
  SCOPEON_STATUS="$(cat {status_q} 2>/dev/null || {exe_q} shell-status 2>/dev/null || true)"
}}
add-zsh-hook precmd _scopeon_refresh
# To show in right prompt:
#   RPROMPT="${{SCOPEON_STATUS}}"
"#
    )
}

fn fish_hook(exe: &str) -> String {
    let status_path = status_file_path();
    let exe_q = shell_quote_fish(exe);
    let status_q = shell_quote_fish(&status_path.to_string_lossy());
    format!(
        r#"# Scopeon ambient status — refresh $SCOPEON_STATUS on every prompt.
# Source in config.fish via:  {exe} shell-hook --shell fish | source
# Uses pre-written status file for zero-fork, zero-latency prompt updates.
function _scopeon_refresh --on-event fish_prompt
  set -gx SCOPEON_STATUS (cat {status_q} 2>/dev/null; or {exe_q} shell-status 2>/dev/null; or echo "")
end
# To show in prompt, add to fish_prompt:
#   echo -n $SCOPEON_STATUS" "
"#
    )
}
