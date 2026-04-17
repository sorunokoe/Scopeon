/// `scopeon team` — Git-Native AI Cost Ledger (TRIZ S-2)
///
/// Aggregates `AI-Cost:` trailers written into git commits by `scopeon git-hook`
/// and renders a Markdown table of per-author AI spend — no cloud, no data sharing,
/// all computation local to the git repository.
///
/// # How it works
///
/// `scopeon git-hook` appends a trailer like:
/// ```
///   AI-Cost: $0.42 (8 turns, 42k tokens, 68% cache)
/// ```
/// This command reads those trailers from `git log`, parses costs, and groups them
/// by author email. Output is a Markdown table suitable for team cost attribution.
///
/// # Usage
///
/// ```sh
/// scopeon team              # last 30 days
/// scopeon team --days 7     # last 7 days
/// scopeon team --days 90    # last quarter
/// ```
use std::collections::HashMap;

use anyhow::Result;

/// Per-author aggregated AI usage extracted from git trailers.
#[derive(Debug, Default)]
struct AuthorStats {
    commits: usize,
    total_cost_usd: f64,
    total_turns: u64,
    total_tokens_k: f64,
}

/// Parse `AI-Cost: $0.42 (8 turns, 42k tokens, 68% cache)` → cost in USD.
fn parse_ai_cost_trailer(value: &str) -> Option<(f64, u64, f64)> {
    let v = value.trim();

    // Extract dollar amount — required.
    let cost = v
        .split_whitespace()
        .find(|w| w.starts_with('$'))
        .and_then(|w| w.trim_start_matches('$').trim_end_matches(',').parse::<f64>().ok())?;

    // Extract turns (optional, default 0).
    let turns: u64 = v
        .split_whitespace()
        .enumerate()
        .find(|(_, w)| *w == "turns,")
        .and_then(|(i, _)| v.split_whitespace().nth(i.wrapping_sub(1)))
        .and_then(|w| w.trim_start_matches('(').parse().ok())
        .unwrap_or(0);

    // Extract tokens in thousands (optional, default 0).
    let tokens_k: f64 = v
        .split_whitespace()
        .find(|w| w.ends_with('k'))
        .and_then(|w| w.trim_end_matches('k').parse().ok())
        .unwrap_or(0.0);

    Some((cost, turns, tokens_k))
}

/// Run `git log` over the last `days` days and return lines of `email|trailer_value`.
fn git_log_trailers(days: i64) -> Result<String> {
    let since = format!("{}.days.ago", days);
    let output = std::process::Command::new("git")
        .args([
            "log",
            "--format=%ae|%(trailers:key=AI-Cost,valueonly,unfold)",
            &format!("--since={}", since),
        ])
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git log failed: {}", stderr.trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Print a Markdown table of per-author AI spend from git trailers.
pub fn cmd_team_report(days: i64) -> Result<()> {
    // Verify we're inside a git repository.
    let repo_check = std::process::Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .output();
    match repo_check {
        Ok(o) if !o.status.success() => {
            anyhow::bail!("Not inside a git repository. `scopeon team` must be run from a git repo.");
        },
        Err(_) => {
            anyhow::bail!("`git` not found in PATH. Install git to use `scopeon team`.");
        },
        _ => {},
    }

    let raw = git_log_trailers(days)?;

    let mut authors: HashMap<String, AuthorStats> = HashMap::new();
    let mut commits_seen: usize = 0;
    let mut commits_with_trailer: usize = 0;

    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let mut parts = line.splitn(2, '|');
        let email = match parts.next() {
            Some(e) if !e.is_empty() => e.trim().to_lowercase(),
            _ => continue,
        };
        let trailer = parts.next().unwrap_or("").trim();

        commits_seen += 1;

        if trailer.is_empty() {
            // Commit exists but has no AI-Cost trailer — still count the commit.
            authors.entry(email.clone()).or_default().commits += 1;
            continue;
        }

        commits_with_trailer += 1;
        let stats = authors.entry(email.clone()).or_default();
        stats.commits += 1;

        if let Some((cost, turns, tokens_k)) = parse_ai_cost_trailer(trailer) {
            stats.total_cost_usd += cost;
            stats.total_turns += turns;
            stats.total_tokens_k += tokens_k;
        }
    }

    if authors.is_empty() {
        eprintln!(
            "No git commits found in the last {} day(s). Nothing to report.",
            days
        );
        return Ok(());
    }

    // Sort by cost descending.
    let mut rows: Vec<(&String, &AuthorStats)> = authors.iter().collect();
    rows.sort_by(|a, b| {
        b.1.total_cost_usd
            .partial_cmp(&a.1.total_cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let total_cost: f64 = rows.iter().map(|(_, s)| s.total_cost_usd).sum();
    let total_commits: usize = rows.iter().map(|(_, s)| s.commits).sum();

    println!("## AI Cost by Author — last {} days\n", days);
    println!(
        "| Author | Commits | AI Commits | Total Cost | Avg / Commit | Tokens |"
    );
    println!("|--------|---------|------------|------------|--------------|--------|");

    for (email, stats) in &rows {
        let ai_commits = if stats.total_cost_usd > 0.0 || stats.total_turns > 0 {
            // Rough count: assume each trailer-carrying commit was an AI commit.
            // (We don't have a separate per-commit flag, so we use cost > 0 as proxy.)
            format!("{}", stats.commits.min(commits_with_trailer))
        } else {
            "0".to_string()
        };

        let avg = if stats.commits > 0 {
            stats.total_cost_usd / stats.commits as f64
        } else {
            0.0
        };

        let tokens_display = if stats.total_tokens_k > 0.0 {
            format!("{:.0}k", stats.total_tokens_k)
        } else {
            "—".to_string()
        };

        println!(
            "| {} | {} | {} | ${:.2} | ${:.2} | {} |",
            email,
            stats.commits,
            ai_commits,
            stats.total_cost_usd,
            avg,
            tokens_display,
        );
    }

    println!("\n**Total**: {} commits, ${:.2} AI spend", total_commits, total_cost);
    if commits_seen > 0 {
        let pct = commits_with_trailer as f64 / commits_seen as f64 * 100.0;
        println!(
            "_AI trailers found on {}/{} commits ({:.0}%)._",
            commits_with_trailer, commits_seen, pct
        );
    }

    if commits_with_trailer == 0 {
        println!();
        println!("> **Tip**: No AI-Cost trailers found. Run `scopeon init` to set up the git");
        println!("> commit hook, then trailer data will accumulate automatically.");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_trailer() {
        let (cost, turns, tokens_k) = parse_ai_cost_trailer("$0.42 (8 turns, 42k tokens, 68% cache)").unwrap();
        assert!((cost - 0.42).abs() < 1e-9);
        assert_eq!(turns, 8);
        assert!((tokens_k - 42.0).abs() < 1e-9);
    }

    #[test]
    fn parse_cost_only() {
        let (cost, turns, tokens_k) = parse_ai_cost_trailer("$1.00").unwrap();
        assert!((cost - 1.0).abs() < 1e-9);
        assert_eq!(turns, 0);
        assert!((tokens_k - 0.0).abs() < 1e-9);
    }

    #[test]
    fn parse_invalid_returns_none() {
        assert!(parse_ai_cost_trailer("no dollar sign here").is_none());
    }
}
