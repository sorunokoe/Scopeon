//! `scopeon git-hook` — Install / uninstall the AI-Cost commit trailer hook.
//!
//! Installs a `prepare-commit-msg` git hook in the current repository that
//! appends a compact `AI-Cost:` trailer to every commit message. The trailer
//! is visible in `git log --show-notes`, GitHub PR diffs, and GitLens blame
//! views — turning every commit into a passive advertisement for Scopeon.
//!
//! # Example commit
//!
//! ```text
//!   feat: add OAuth login
//!
//!   AI-Cost: $0.14 (8 turns, 62k tokens, 71% cache)
//!   Powered-by: Scopeon https://github.com/scopeon/scopeon
//! ```
//!
//! # Usage
//!
//! ```text
//!   scopeon git-hook install    # install hook in current repo's .git/hooks/
//!   scopeon git-hook uninstall  # remove the hook
//! ```

use anyhow::{bail, Context, Result};

const HOOK_MARKER: &str = "# scopeon-git-hook";

const HOOK_BODY: &str = r#"#!/bin/sh
# scopeon-git-hook — appends AI-Cost trailer to every commit message.
# Installed by `scopeon git-hook install`. Remove with `scopeon git-hook uninstall`.
if command -v scopeon >/dev/null 2>&1; then
    trailer=$(scopeon git-trailer 2>/dev/null)
    if [ -n "$trailer" ]; then
        # Append a blank line then the trailer if not already present.
        if ! grep -qF 'AI-Cost:' "$1"; then
            printf '\n%s\n' "$trailer" >> "$1"
        fi
    fi
fi
"#;

fn find_git_dir() -> Result<std::path::PathBuf> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .output()
        .context("git not found — is this a git repository?")?;
    if !output.status.success() {
        bail!("Not inside a git repository. Run `scopeon git-hook install` from within a repo.");
    }
    let git_dir = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(std::path::PathBuf::from(git_dir))
}

pub fn install() -> Result<()> {
    let git_dir = find_git_dir()?;
    let hooks_dir = git_dir.join("hooks");
    std::fs::create_dir_all(&hooks_dir)?;
    let hook_path = hooks_dir.join("prepare-commit-msg");

    if hook_path.exists() {
        let existing = std::fs::read_to_string(&hook_path)?;
        if existing.contains(HOOK_MARKER) {
            println!(
                "✓ Scopeon git hook is already installed at {}",
                hook_path.display()
            );
            return Ok(());
        }
        // Append to existing hook rather than replacing it.
        let combined = format!("{}\n{}", existing.trim_end(), HOOK_BODY);
        std::fs::write(&hook_path, combined)?;
        println!(
            "✓ Scopeon AI-Cost trailer appended to existing hook at {}",
            hook_path.display()
        );
    } else {
        std::fs::write(&hook_path, HOOK_BODY)?;
        println!("✓ Scopeon git hook installed at {}", hook_path.display());
    }

    // Make the hook executable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&hook_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&hook_path, perms)?;
    }

    println!();
    println!("  Every commit in this repo will now include an AI-Cost trailer:");
    println!("  AI-Cost: $0.12 (8 turns, 62k tokens, 71% cache)");
    println!("  Powered-by: Scopeon https://github.com/scopeon/scopeon");
    println!();
    println!("  Remove with: scopeon git-hook uninstall");

    Ok(())
}

pub fn uninstall() -> Result<()> {
    let git_dir = find_git_dir()?;
    let hook_path = git_dir.join("hooks").join("prepare-commit-msg");

    if !hook_path.exists() {
        println!(
            "No prepare-commit-msg hook found at {}",
            hook_path.display()
        );
        return Ok(());
    }

    let content = std::fs::read_to_string(&hook_path)?;
    if !content.contains(HOOK_MARKER) {
        println!(
            "Scopeon hook marker not found in {}. Nothing to remove.",
            hook_path.display()
        );
        return Ok(());
    }

    // Remove lines from the scopeon marker onward (trim the appended block).
    let without_scopeon: String = content
        .lines()
        .take_while(|line| !line.contains(HOOK_MARKER))
        .collect::<Vec<_>>()
        .join("\n");

    let trimmed = without_scopeon.trim_end().to_string();

    if trimmed.is_empty() || trimmed == "#!/bin/sh" {
        // The hook was exclusively ours — delete the file.
        std::fs::remove_file(&hook_path)?;
        println!("✓ Scopeon git hook removed (hook file deleted).");
    } else {
        std::fs::write(&hook_path, format!("{trimmed}\n"))?;
        println!("✓ Scopeon git hook removed from {}.", hook_path.display());
    }

    Ok(())
}

/// Output the AI-Cost trailer line for use in the hook script.
/// Called by the hook as `scopeon git-trailer`.
pub fn print_trailer(db: &scopeon_core::Database) -> Result<()> {
    use scopeon_core::cache_hit_rate;

    let global = db.get_global_stats()?;
    let rollups = db.get_daily_rollups(1)?;

    // Today's cost (last rollup is today).
    let today_cost = rollups.last().map(|r| r.estimated_cost_usd).unwrap_or(0.0);
    let today_turns: i64 = rollups.last().map(|r| r.turn_count).unwrap_or(0);
    let today_input: i64 = rollups.last().map(|r| r.total_input_tokens).unwrap_or(0);

    let cache_pct = (cache_hit_rate(
        global.total_input_tokens,
        global.total_cache_read_tokens,
        global.total_cache_write_tokens,
    ) * 100.0)
        .round() as u64;

    // Only emit if there's actual activity today.
    if today_cost <= 0.0 && today_turns == 0 {
        return Ok(());
    }

    println!(
        "AI-Cost: ~${today_cost:.2} ({today_turns} turns, {}k tokens, {cache_pct}% cache)",
        today_input / 1_000
    );
    println!("Powered-by: Scopeon https://github.com/scopeon/scopeon");

    Ok(())
}
