//! IS-9: `scopeon digest` — AI usage digest report.
//!
//! Outputs a Markdown weekly (or N-day) report covering:
//! - Total cost, cache savings, and token usage for the period
//! - Daily breakdown with trend indicators
//! - Top sessions by cost
//! - Cost by tag (if tags are in use)
//! - Top waste signals and actionable recommendations
//!
//! Supports posting to Slack and Discord webhooks for team discovery:
//!
//! ```text
//!   scopeon digest --post-to-slack https://hooks.slack.com/services/...
//!   scopeon digest --post-to-discord https://discord.com/api/webhooks/...
//! ```

use std::fmt::Write as _;

use anyhow::Result;
use scopeon_core::Database;

/// Run the digest command: print the Markdown report and optionally post it
/// to a Slack or Discord webhook URL.
pub fn run(
    db: &Database,
    days: i64,
    post_to_slack: Option<&str>,
    post_to_discord: Option<&str>,
) -> Result<()> {
    let report = build_report(db, days)?;
    print!("{report}");

    if let Some(url) = post_to_slack {
        post_webhook(url, &report, WebhookKind::Slack)?;
    }
    if let Some(url) = post_to_discord {
        post_webhook(url, &report, WebhookKind::Discord)?;
    }

    Ok(())
}

enum WebhookKind {
    Slack,
    Discord,
}

fn post_webhook(url: &str, markdown: &str, kind: WebhookKind) -> Result<()> {
    // Truncate to fit API limits (Slack: 40 000 chars, Discord: 2 000 chars per message).
    let limit = match kind {
        WebhookKind::Discord => 1_950,
        WebhookKind::Slack => 39_000,
    };
    let body_text = if markdown.len() > limit {
        let truncated = &markdown[..limit];
        format!("{truncated}\n\n*[Report truncated — run `scopeon digest` locally for the full version]*")
    } else {
        markdown.to_string()
    };

    let (field, label) = match kind {
        WebhookKind::Slack => ("text", "Slack"),
        WebhookKind::Discord => ("content", "Discord"),
    };

    let payload = serde_json::json!({ field: body_text }).to_string();

    let status = std::process::Command::new("curl")
        .args([
            "-sf",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "-d",
            &payload,
            url,
        ])
        .output();

    match status {
        Ok(out) => {
            let code = String::from_utf8_lossy(&out.stdout);
            let code_num: u16 = code.trim().parse().unwrap_or(0);
            if (200..300).contains(&code_num) {
                eprintln!("✓ Digest posted to {label} (HTTP {code_num})");
            } else {
                eprintln!("⚠ {label} webhook returned HTTP {code_num}. Check the webhook URL.");
            }
        },
        Err(e) => {
            eprintln!("⚠ Could not post to {label}: {e}");
            eprintln!("  Is `curl` installed? Run `which curl` to check.");
        },
    }

    Ok(())
}

fn build_report(db: &Database, days: i64) -> Result<String> {
    let mut out = String::with_capacity(8 * 1024);
    let version = env!("CARGO_PKG_VERSION");
    let now = chrono::Local::now();
    let period_label = if days == 7 {
        "Weekly".to_string()
    } else if days == 30 {
        "Monthly".to_string()
    } else {
        format!("{}-Day", days)
    };

    let rollups = db.get_daily_rollups(days).unwrap_or_default();
    let sessions = db.list_sessions(200).unwrap_or_default();
    let tags = db.get_cost_by_tag_days(days as u32).unwrap_or_default();
    let model_costs = db.get_cost_by_model().unwrap_or_default();

    // Aggregate period totals.
    let total_cost: f64 = rollups.iter().map(|r| r.estimated_cost_usd).sum();
    let total_input: i64 = rollups.iter().map(|r| r.total_input_tokens).sum();
    let total_cache_read: i64 = rollups.iter().map(|r| r.total_cache_read_tokens).sum();
    let _total_cache_write: i64 = rollups.iter().map(|r| r.total_cache_write_tokens).sum();
    let total_output: i64 = rollups.iter().map(|r| r.total_output_tokens).sum();
    let total_turns: i64 = rollups.iter().map(|r| r.turn_count).sum();
    let total_sessions: i64 = rollups.iter().map(|r| r.session_count).sum();
    let total_mcp: i64 = rollups.iter().map(|r| r.total_mcp_calls).sum();

    // Cache hit rate for period.
    let total_tokens_used = total_input + total_cache_read;
    let cache_hit_rate = if total_tokens_used > 0 {
        total_cache_read as f64 / total_tokens_used as f64 * 100.0
    } else {
        0.0
    };

    // Exact cache savings computed per model using model-specific pricing.
    let cache_savings: f64 = db
        .get_cache_tokens_by_model()
        .unwrap_or_default()
        .into_iter()
        .map(|(model, read_tok, write_tok)| {
            scopeon_core::cache_savings_usd(&model, read_tok, write_tok)
        })
        .sum::<f64>()
        .max(0.0);

    // Average cost per turn.
    let avg_cost_per_turn = if total_turns > 0 {
        total_cost / total_turns as f64
    } else {
        0.0
    };

    let cutoff_ms = (now - chrono::Duration::days(days)).timestamp_millis();
    let recent_sessions: Vec<_> = sessions
        .iter()
        .filter(|s| s.last_turn_at >= cutoff_ms)
        .take(10)
        .collect();

    // ── Report ────────────────────────────────────────────────────────────────

    writeln!(out, "# Scopeon {period_label} AI Digest")?;
    writeln!(out)?;
    writeln!(
        out,
        "> Generated by Scopeon v{version} · {days} days ending {}",
        now.format("%Y-%m-%d %H:%M")
    )?;
    writeln!(out)?;

    // ── Executive Summary ─────────────────────────────────────────────────────
    writeln!(out, "## Executive Summary")?;
    writeln!(out)?;
    writeln!(out, "| Metric | Value |")?;
    writeln!(out, "|--------|-------|")?;
    writeln!(out, "| **Total Cost** | **~${total_cost:.2}** |")?;
    writeln!(out, "| Sessions | {total_sessions} |")?;
    writeln!(out, "| Turns | {total_turns} |")?;
    writeln!(out, "| Avg Cost/Turn | ~${avg_cost_per_turn:.3} |")?;
    writeln!(out, "| Cache Hit Rate | {cache_hit_rate:.1}% |")?;
    writeln!(out, "| Est. Cache Savings | ~${cache_savings:.2} |")?;
    writeln!(out, "| MCP Tool Calls | {total_mcp} |")?;
    writeln!(out, "| Input Tokens | {}K |", total_input / 1_000)?;
    writeln!(out, "| Output Tokens | {}K |", total_output / 1_000)?;
    writeln!(out)?;

    // ── Daily Breakdown ───────────────────────────────────────────────────────
    if !rollups.is_empty() {
        writeln!(out, "## Daily Breakdown")?;
        writeln!(out)?;
        writeln!(out, "| Date | Cost | Sessions | Turns | Cache Hit | MCP |")?;
        writeln!(out, "|------|------|----------|-------|-----------|-----|")?;
        for day in &rollups {
            let day_tokens = day.total_input_tokens + day.total_cache_read_tokens;
            let day_cache_pct = if day_tokens > 0 {
                day.total_cache_read_tokens as f64 / day_tokens as f64 * 100.0
            } else {
                0.0
            };
            let trend = if day.estimated_cost_usd > avg_cost_per_turn * day.turn_count as f64 * 1.2
            {
                " ↑"
            } else if day.estimated_cost_usd < avg_cost_per_turn * day.turn_count as f64 * 0.8 {
                " ↓"
            } else {
                ""
            };
            writeln!(
                out,
                "| {} | ~${:.2}{} | {} | {} | {:.0}% | {} |",
                day.date,
                day.estimated_cost_usd,
                trend,
                day.session_count,
                day.turn_count,
                day_cache_pct,
                day.total_mcp_calls,
            )?;
        }
        writeln!(out)?;
    }

    // ── Cost by Model ─────────────────────────────────────────────────────────
    if !model_costs.is_empty() {
        writeln!(out, "## Cost by Model")?;
        writeln!(out)?;
        writeln!(out, "| Model | Cost | Share |")?;
        writeln!(out, "|-------|------|-------|")?;
        for (model, cost) in &model_costs {
            let share = if total_cost > 0.0 {
                cost / total_cost * 100.0
            } else {
                0.0
            };
            writeln!(out, "| {} | ~${:.2} | {:.0}% |", model, cost, share)?;
        }
        writeln!(out)?;
    }

    // ── Cost by Tag ───────────────────────────────────────────────────────────
    if !tags.is_empty() {
        writeln!(out, "## Cost by Tag")?;
        writeln!(out)?;
        writeln!(out, "| Tag | Cost | Sessions |")?;
        writeln!(out, "|-----|------|----------|")?;
        for (tag, cost, sess_count) in &tags {
            writeln!(out, "| `{}` | ~${:.2} | {} |", tag, cost, sess_count)?;
        }
        writeln!(out)?;
    }

    // ── Recent Sessions ───────────────────────────────────────────────────────
    if !recent_sessions.is_empty() {
        writeln!(out, "## Recent Sessions")?;
        writeln!(out)?;
        writeln!(out, "| Project | Branch | Model | Turns | Last Active |")?;
        writeln!(out, "|---------|--------|-------|-------|-------------|")?;
        for s in &recent_sessions {
            let last_active = chrono::DateTime::from_timestamp_millis(s.last_turn_at)
                .map(|dt| {
                    dt.with_timezone(&chrono::Local)
                        .format("%m-%d %H:%M")
                        .to_string()
                })
                .unwrap_or_else(|| "—".to_string());
            writeln!(
                out,
                "| {} | {} | {} | {} | {} |",
                s.project_name,
                if s.git_branch.is_empty() {
                    "—"
                } else {
                    &s.git_branch
                },
                shorten_model(&s.model),
                s.total_turns,
                last_active,
            )?;
        }
        writeln!(out)?;
    }

    // ── Optimization Recommendations ──────────────────────────────────────────
    writeln!(out, "## Optimization Recommendations")?;
    writeln!(out)?;

    if cache_hit_rate < 40.0 {
        writeln!(
            out,
            "- ⚠ **Low cache efficiency ({cache_hit_rate:.0}%)** — reuse sessions instead of starting fresh. \
             Cache warmup takes ~5 turns; short sessions waste this investment."
        )?;
    } else if cache_hit_rate >= 70.0 {
        writeln!(
            out,
            "- ✓ **Cache efficiency is strong ({cache_hit_rate:.0}%)** — context reuse is working well."
        )?;
    }

    if avg_cost_per_turn > 0.05 {
        writeln!(
            out,
            "- ⚠ **High cost per turn (${avg_cost_per_turn:.3})** — consider using a smaller model for \
             exploratory tasks and reserving claude-opus for complex reasoning."
        )?;
    }

    if total_mcp > 0 && total_turns > 0 {
        let mcp_per_turn = total_mcp as f64 / total_turns as f64;
        if mcp_per_turn > 5.0 {
            writeln!(
                out,
                "- 💡 **High MCP call density ({mcp_per_turn:.1} calls/turn)** — each MCP call adds \
                 token overhead. Batch tool calls where possible."
            )?;
        }
    }

    if cache_savings > 1.0 {
        writeln!(
            out,
            "- ✓ **Cache saved an estimated ${cache_savings:.2}** this period — context reuse is paying off."
        )?;
    }

    if total_cost == 0.0 {
        writeln!(out, "- ℹ No activity recorded in this period. Check `scopeon doctor` if sessions are missing.")?;
    }

    writeln!(out)?;
    writeln!(out, "---")?;
    writeln!(out)?;
    writeln!(
        out,
        "*Scopeon v{version} — AI Context Observability · <https://github.com/scopeon/scopeon>*"
    )?;

    Ok(out)
}

fn shorten_model(model: &str) -> &str {
    if model.len() > 20 {
        &model[..20]
    } else {
        model
    }
}
