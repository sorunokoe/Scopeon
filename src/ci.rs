/// CI Cost Gate — `scopeon ci snapshot` and `scopeon ci report`
///
/// Implements TRIZ Solution 5: moves AI cost from a personal terminal metric
/// to a team-visible GitHub PR comment, enabling organizations to gate
/// on AI spending regressions the same way they gate on test failures.
///
/// # Workflow
/// 1. On the base branch (e.g. main), run `scopeon ci snapshot --output baseline.json`
/// 2. After the feature branch work, run `scopeon ci report --baseline baseline.json`
/// 3. The report prints a Markdown table comparing current vs baseline.
/// 4. Optionally, pass `--fail-on-cost-delta 50` to exit non-zero when cost
///    grew by more than 50% (useful as a CI gate).
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use scopeon_core::{Database, GlobalStats};

/// A serializable point-in-time snapshot of AI usage metrics.
/// Captured by `scopeon ci snapshot` and compared by `scopeon ci report`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CiSnapshot {
    /// Total estimated cost in USD across all captured sessions.
    pub total_cost_usd: f64,
    /// Cache hit rate (0.0–1.0). Higher is better — reduces cost.
    pub cache_hit_rate: f64,
    /// Peak context window fill percentage observed (0.0–100.0).
    pub context_peak_pct: f64,
    /// Average input tokens per turn across all sessions.
    pub avg_tokens_per_turn: f64,
    /// Total number of sessions captured.
    pub sessions: i64,
    /// Total number of AI turns captured.
    pub turns: i64,
    /// Total input tokens (excluding cache reads/writes).
    pub total_input_tokens: i64,
    /// ISO 8601 timestamp of when this snapshot was captured.
    pub captured_at: String,
}

impl CiSnapshot {
    /// Build a snapshot from current database global stats.
    pub fn from_db(db: &Database) -> Result<Self> {
        let stats: GlobalStats = db
            .get_global_stats()
            .context("Reading global stats from DB")?;
        let context_peak_pct = peak_context_pct(&stats);
        let avg_tokens_per_turn = if stats.total_turns > 0 {
            stats.total_input_tokens as f64 / stats.total_turns as f64
        } else {
            0.0
        };
        Ok(Self {
            total_cost_usd: stats.estimated_cost_usd,
            cache_hit_rate: stats.cache_hit_rate,
            context_peak_pct,
            avg_tokens_per_turn,
            sessions: stats.total_sessions,
            turns: stats.total_turns,
            total_input_tokens: stats.total_input_tokens,
            captured_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    /// Save this snapshot to a JSON file.
    pub fn save(&self, path: &PathBuf) -> Result<()> {
        let json = serde_json::to_string_pretty(self).context("Serializing CI snapshot")?;
        std::fs::write(path, json)
            .with_context(|| format!("Writing snapshot to {}", path.display()))?;
        Ok(())
    }

    /// Load a snapshot from a JSON file.
    pub fn load(path: &PathBuf) -> Result<Self> {
        let json = std::fs::read_to_string(path)
            .with_context(|| format!("Reading snapshot from {}", path.display()))?;
        serde_json::from_str(&json).context("Parsing CI snapshot JSON")
    }
}

/// Estimate the peak context fill % from global stats daily rollups.
/// Falls back to 0.0 if no context pressure data is available.
fn peak_context_pct(stats: &GlobalStats) -> f64 {
    // Use max avg_input_per_turn / context_window as a CI context-fill signal.
    // Claude Sonnet/Opus have 200k window; use that conservative default.
    // TODO: store dominant_model in GlobalStats for per-model accuracy.
    let window = scopeon_core::context_window_for_model("claude") as f64;
    let max_avg_input = stats
        .daily
        .iter()
        .map(|d| {
            if d.turn_count > 0 {
                d.total_input_tokens as f64 / d.turn_count as f64
            } else {
                0.0
            }
        })
        .fold(0.0_f64, f64::max);
    (max_avg_input / window * 100.0).min(100.0)
}

/// Run `scopeon ci snapshot --output path`.
pub fn cmd_snapshot(db: &Database, output: &PathBuf) -> Result<()> {
    let snap = CiSnapshot::from_db(db)?;
    snap.save(output)?;
    eprintln!(
        "✅ Snapshot saved to {} (cost: ${:.4}, sessions: {}, turns: {})",
        output.display(),
        snap.total_cost_usd,
        snap.sessions,
        snap.turns,
    );
    Ok(())
}

/// Run `scopeon ci report [--baseline path] [--fail-on-cost-delta pct]`.
///
/// Prints a Markdown table to stdout (suitable for posting as a GitHub PR comment).
/// Returns an error if `fail_threshold_pct` is set and the cost delta exceeds it.
pub fn cmd_report(
    db: &Database,
    baseline_path: Option<&PathBuf>,
    fail_threshold_pct: Option<f64>,
) -> Result<()> {
    let current = CiSnapshot::from_db(db)?;
    let baseline = baseline_path.map(CiSnapshot::load).transpose()?;

    // Print the Markdown report to stdout (for use in `gh pr comment --body-file -`)
    println!("{}", render_report(&current, baseline.as_ref()));

    // CI gate: exit non-zero if cost grew beyond the threshold
    if let (Some(baseline), Some(threshold)) = (&baseline, fail_threshold_pct) {
        if baseline.total_cost_usd > 0.0 {
            let delta_pct = (current.total_cost_usd - baseline.total_cost_usd)
                / baseline.total_cost_usd
                * 100.0;
            if delta_pct > threshold {
                anyhow::bail!(
                    "CI gate: AI cost grew by {:.1}% (threshold: {:.1}%). \
                     Run `scopeon ci report --baseline baseline.json` locally to investigate.",
                    delta_pct,
                    threshold,
                );
            }
        }
    }

    Ok(())
}

/// Render the Markdown report table.
fn render_report(current: &CiSnapshot, baseline: Option<&CiSnapshot>) -> String {
    let mut out = String::new();
    out.push_str("## 🔬 AI Cost Analysis\n\n");

    if let Some(base) = baseline {
        out.push_str(&format!(
            "Comparing current snapshot ({}) to baseline ({}).\n\n",
            &current.captured_at[..10],
            &base.captured_at[..10],
        ));
        out.push_str("| Metric | Baseline | Current | Delta |\n");
        out.push_str("|--------|----------|---------|-------|\n");

        out.push_str(&metric_row(
            "Total cost",
            &format!("~${:.4}", base.total_cost_usd),
            &format!("~${:.4}", current.total_cost_usd),
            pct_delta(base.total_cost_usd, current.total_cost_usd),
            true, // lower is better
        ));
        out.push_str(&metric_row(
            "Cache hit rate",
            &format!("{:.1}%", base.cache_hit_rate * 100.0),
            &format!("{:.1}%", current.cache_hit_rate * 100.0),
            pct_delta(base.cache_hit_rate, current.cache_hit_rate),
            false, // higher is better
        ));
        out.push_str(&metric_row(
            "Context peak",
            &format!("{:.1}%", base.context_peak_pct),
            &format!("{:.1}%", current.context_peak_pct),
            pct_delta(base.context_peak_pct, current.context_peak_pct),
            true, // lower is better
        ));
        out.push_str(&metric_row(
            "Avg tokens/turn",
            &format!("{:.0}", base.avg_tokens_per_turn),
            &format!("{:.0}", current.avg_tokens_per_turn),
            pct_delta(base.avg_tokens_per_turn, current.avg_tokens_per_turn),
            true, // lower is better
        ));
        out.push_str(&metric_row(
            "Sessions",
            &base.sessions.to_string(),
            &current.sessions.to_string(),
            None,
            false,
        ));
        out.push_str(&metric_row(
            "Turns",
            &base.turns.to_string(),
            &current.turns.to_string(),
            None,
            false,
        ));
    } else {
        // No baseline — just show current stats
        out.push_str(&format!(
            "_No baseline provided. Showing current snapshot from {}_\n\n",
            &current.captured_at[..10],
        ));
        out.push_str("| Metric | Value |\n");
        out.push_str("|--------|-------|\n");
        out.push_str(&format!(
            "| Total cost | ~${:.4} |\n",
            current.total_cost_usd
        ));
        out.push_str(&format!(
            "| Cache hit rate | {:.1}% |\n",
            current.cache_hit_rate * 100.0
        ));
        out.push_str(&format!(
            "| Context peak | {:.1}% |\n",
            current.context_peak_pct
        ));
        out.push_str(&format!(
            "| Avg tokens/turn | {:.0} |\n",
            current.avg_tokens_per_turn
        ));
        out.push_str(&format!("| Sessions | {} |\n", current.sessions));
        out.push_str(&format!("| Turns | {} |\n", current.turns));
    }

    // Suggestions
    out.push('\n');
    if current.cache_hit_rate < 0.5 {
        out.push_str(
            "💡 **Cold cache detected.** Consider restructuring prompts to keep system \
             instructions stable and warm the prompt cache. Potential savings: up to 60–90% \
             on input token costs.\n",
        );
    }
    if current.context_peak_pct > 80.0 {
        out.push_str(
            "⚠️ **High context fill detected.** Sessions are approaching the context window \
             limit. Consider using `/compact` more frequently to reduce compaction costs.\n",
        );
    }

    out.push_str(&format!(
        "\n<sub>Generated by [Scopeon](https://github.com/yersonargote/scopeon) v{} at {}</sub>\n",
        env!("CARGO_PKG_VERSION"),
        &current.captured_at[..19],
    ));

    out
}

/// Compute the percentage delta between two values, returning None if base is zero.
fn pct_delta(base: f64, current: f64) -> Option<f64> {
    if base == 0.0 {
        None
    } else {
        Some((current - base) / base * 100.0)
    }
}

/// Format a table row with optional delta and emoji indicator.
///
/// `lower_is_better`: if true, a positive delta (increase) is bad (red/warning emoji).
fn metric_row(
    name: &str,
    baseline_str: &str,
    current_str: &str,
    delta: Option<f64>,
    lower_is_better: bool,
) -> String {
    let delta_str = match delta {
        None => "—".to_string(),
        Some(d) => {
            let emoji = if d.abs() < 5.0 {
                "⬜" // negligible change
            } else if (d > 0.0) == lower_is_better {
                // Positive delta on a "lower is better" metric = bad
                if d.abs() > 50.0 {
                    "🔴"
                } else {
                    "🟡"
                }
            } else {
                "🟢" // improvement
            };
            format!("{:+.1}% {}", d, emoji)
        },
    };
    format!(
        "| {} | {} | {} | {} |\n",
        name, baseline_str, current_str, delta_str
    )
}
