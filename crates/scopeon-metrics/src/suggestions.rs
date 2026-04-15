use crate::metric::MetricContext;
use crate::waste::WasteReport;
use scopeon_core::GlobalStats;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suggestion {
    pub id: &'static str,
    pub severity: Severity,
    pub title: &'static str,
    pub body: String,
}

pub fn compute_suggestions(
    ctx: &MetricContext,
    waste: &WasteReport,
    global_stats: Option<&GlobalStats>,
) -> Vec<Suggestion> {
    let mut suggestions = Vec::new();

    // 1. Cache warmup: turns until first cache hit — threshold proportional to session length
    if !ctx.turns.is_empty() {
        let warmup = ctx.turns.iter().position(|t| t.cache_read_tokens > 0);
        if let Some(idx) = warmup {
            // For short sessions (≤50 turns), absolute threshold of 5.
            // For longer sessions, also flag if >10% of turns had no cache.
            let abs_threshold = 5;
            let proportional_threshold = (ctx.turns.len() / 10).max(abs_threshold);
            if idx > proportional_threshold {
                suggestions.push(Suggestion {
                    id: "cache-warmup",
                    severity: Severity::Info,
                    title: "Cache Warmup",
                    body: format!(
                        "Cache warmup takes ~{} turns ({:.0}% of session) — consider reusing sessions",
                        idx + 1,
                        (idx + 1) as f64 / ctx.turns.len() as f64 * 100.0,
                    ),
                });
            }
        }
    }

    // 2. Compaction frequency from turns
    let compaction_count = ctx.turns.iter().filter(|t| t.is_compaction_event).count();
    if compaction_count > 2 {
        suggestions.push(Suggestion {
            id: "compaction-freq",
            severity: Severity::Warning,
            title: "Auto-Compaction",
            body: format!(
                "{} auto-compactions detected — try /compact manually earlier",
                compaction_count
            ),
        });
    }

    // 3. Thinking ratio anomaly
    if !ctx.turns.is_empty() {
        let thinking: i64 = ctx.turns.iter().map(|t| t.thinking_tokens).sum();
        let output: i64 = ctx.turns.iter().map(|t| t.output_tokens).sum();
        if output > 0 {
            let ratio = thinking as f64 / output as f64 * 100.0;
            if ratio > 60.0 {
                suggestions.push(Suggestion {
                    id: "thinking-ratio",
                    severity: Severity::Info,
                    title: "High Thinking Ratio",
                    body: format!(
                        "Thinking is {:.0}% of output — model is reasoning heavily",
                        ratio
                    ),
                });
            }
        }
    }

    // 4. Redundant tools from waste signals
    let has_redundant = waste
        .signals
        .iter()
        .any(|s| matches!(s.kind, crate::waste::WasteKind::RedundantToolCalls { .. }));
    if has_redundant {
        suggestions.push(Suggestion {
            id: "redundant-tools",
            severity: Severity::Warning,
            title: "Redundant Tool Calls",
            body: "Same tool called with identical inputs — check for loops or retry patterns"
                .to_string(),
        });
    }

    // 5. Cold cache sessions
    let has_cold_cache = waste
        .signals
        .iter()
        .any(|s| matches!(s.kind, crate::waste::WasteKind::ColdCacheSession { .. }));
    if has_cold_cache {
        suggestions.push(Suggestion {
            id: "cold-cache",
            severity: Severity::Warning,
            title: "Cold Cache",
            body: "Session not benefiting from cache — avoid restarting sessions unnecessarily"
                .to_string(),
        });
    }

    // 6. High MCP density
    if !ctx.turns.is_empty() {
        let mcp: i64 = ctx.turns.iter().map(|t| t.mcp_call_count).sum();
        let density = mcp as f64 / ctx.turns.len() as f64;
        if density > 5.0 {
            suggestions.push(Suggestion {
                id: "high-mcp",
                severity: Severity::Info,
                title: "High MCP Density",
                body: format!(
                    "{:.1} MCP calls/turn — consider limiting enabled MCP servers",
                    density
                ),
            });
        }
    }

    // 7–9. Cross-session intelligence: compare this session against historical averages.
    // These suggestions only fire when global_stats covers enough history (≥ 10 turns).
    if let Some(global) = global_stats {
        if global.total_turns >= 10 {
            // 7. Below-average cache hit rate vs. user's own historical mean.
            let session_cache_hit = if !ctx.turns.is_empty() {
                let read: i64 = ctx.turns.iter().map(|t| t.cache_read_tokens).sum();
                let inp: i64 = ctx.turns.iter().map(|t| t.input_tokens).sum();
                let write: i64 = ctx.turns.iter().map(|t| t.cache_write_tokens).sum();
                let total = inp + read + write;
                if total > 0 {
                    read as f64 / total as f64
                } else {
                    0.0
                }
            } else {
                0.0
            };
            // Only fire if the user historically achieves meaningful caching (>10%) but
            // this session is significantly worse (< 70% of their average).
            if global.cache_hit_rate > 0.10
                && session_cache_hit < global.cache_hit_rate * 0.70
                && !ctx.turns.is_empty()
            {
                suggestions.push(Suggestion {
                    id: "below-avg-cache",
                    severity: Severity::Warning,
                    title: "Below-Average Cache Efficiency",
                    body: format!(
                        "This session's cache hit rate ({:.0}%) is well below your historical average ({:.0}%) — \
                         consider reusing or extending the current session instead of starting fresh",
                        session_cache_hit * 100.0,
                        global.cache_hit_rate * 100.0,
                    ),
                });
            }

            // 8. Above-average tokens per turn vs. user's historical mean.
            // Fires only when the session has ≥ 5 turns to avoid early-session noise.
            if ctx.turns.len() >= 5 {
                let session_total_input: i64 = ctx.turns.iter().map(|t| t.input_tokens).sum();
                let session_avg_input = session_total_input as f64 / ctx.turns.len() as f64;
                let global_avg_input = if global.total_turns > 0 {
                    global.total_input_tokens as f64 / global.total_turns as f64
                } else {
                    0.0
                };
                if global_avg_input > 0.0 && session_avg_input > global_avg_input * 1.75 {
                    suggestions.push(Suggestion {
                        id: "above-avg-input",
                        severity: Severity::Info,
                        title: "Unusually Large Context Per Turn",
                        body: format!(
                            "Avg input this session ({:.0}k tokens/turn) is {:.0}× your historical average ({:.0}k) — \
                             large system prompts or tool results may be bloating context",
                            session_avg_input / 1000.0,
                            session_avg_input / global_avg_input,
                            global_avg_input / 1000.0,
                        ),
                    });
                }
            }

            // 9. High cost per turn vs. historical mean — surfaces expensive sessions early.
            if ctx.turns.len() >= 3 {
                let session_cost: f64 = ctx.turns.iter().map(|t| t.estimated_cost_usd).sum();
                let session_cost_per_turn = session_cost / ctx.turns.len() as f64;
                let global_cost_per_turn = if global.total_turns > 0 {
                    global.estimated_cost_usd / global.total_turns as f64
                } else {
                    0.0
                };
                if global_cost_per_turn > 0.0
                    && session_cost_per_turn > global_cost_per_turn * 2.0
                    && session_cost_per_turn > 0.01
                {
                    suggestions.push(Suggestion {
                        id: "high-cost-per-turn",
                        severity: Severity::Warning,
                        title: "High Cost Per Turn",
                        body: format!(
                            "This session costs ${:.3}/turn on average — {:.0}× your historical mean (${:.3}). \
                             Check model selection, thinking budget, or context size",
                            session_cost_per_turn,
                            session_cost_per_turn / global_cost_per_turn,
                            global_cost_per_turn,
                        ),
                    });
                }
            }
        }
    }

    // IS-F: Z-score personal outlier detection — 3-tier progressive personalization.
    // <7d data → keep existing static checks (already above).
    // 7-90d → P10/P90 comparison.
    // >90d daily entries → full Z-score outlier detection on daily cost.
    //
    // MINOR-5 fix: Only use *completed* days for the distribution and comparison.
    // The last daily entry is today (partial day), so we exclude it from the
    // cost distribution and use it only as the "today" value being compared.
    if let Some(global) = global_stats {
        let daily = &global.daily;
        // Exclude the last entry (today — partial) from the reference distribution.
        let completed = if daily.len() > 1 {
            &daily[..daily.len() - 1]
        } else {
            &[] as &[_]
        };
        let costs: Vec<f64> = completed
            .iter()
            .map(|d| d.estimated_cost_usd)
            .filter(|&c| c > 0.0)
            .collect();
        if costs.len() >= 7 {
            let n = costs.len() as f64;
            let mean = costs.iter().sum::<f64>() / n;
            let variance = costs.iter().map(|c| (c - mean).powi(2)).sum::<f64>() / n;
            let std_dev = variance.sqrt();

            // Today's cost from the most-recent daily entry (may be partial).
            let today_cost = daily.last().map(|d| d.estimated_cost_usd).unwrap_or(0.0);

            if std_dev > 0.0 {
                let z = (today_cost - mean) / std_dev;
                if z > 2.0 && today_cost > 0.01 {
                    // Use "full Z-score" label for >90 days of data.
                    let label = if costs.len() >= 90 { "Z-score" } else { "P90" };
                    suggestions.push(Suggestion {
                        id: "zscore-high-cost-day",
                        severity: Severity::Warning,
                        title: "Unusually Expensive Day",
                        body: format!(
                            "Today's cost so far (${:.2}) is {:.1}σ above your daily average (${:.2}) — \
                             {} outlier. Check for large sessions, missing cache, or model upgrade \
                             that skipped pricing.",
                            today_cost, z, mean, label,
                        ),
                    });
                } else if z < -2.0 && mean > 0.05 {
                    suggestions.push(Suggestion {
                        id: "zscore-low-cost-day",
                        severity: Severity::Info,
                        title: "Low-Cost Day",
                        body: format!(
                            "Today's cost so far (${:.2}) is {:.1}σ below your average (${:.2}). \
                             Great caching, lighter workload, or successful optimization.",
                            today_cost,
                            z.abs(),
                            mean,
                        ),
                    });
                }
            }
        }
    }

    suggestions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metric::MetricContext;
    use crate::waste::WasteReport;
    use scopeon_core::GlobalStats;

    fn make_turn(input: i64, cache_read: i64, cost: f64) -> scopeon_core::Turn {
        scopeon_core::Turn {
            id: "t1".into(),
            session_id: "s1".into(),
            turn_index: 0,
            timestamp: 1_700_000_000_000,
            duration_ms: None,
            input_tokens: input,
            cache_read_tokens: cache_read,
            cache_write_tokens: 0,
            cache_write_5m_tokens: 0,
            cache_write_1h_tokens: 0,
            output_tokens: 100,
            thinking_tokens: 0,
            mcp_call_count: 0,
            mcp_input_token_est: 0,
            text_output_tokens: 100,
            model: "claude-sonnet".into(),
            service_tier: "standard".into(),
            estimated_cost_usd: cost,
            is_compaction_event: false,
        }
    }

    fn empty_ctx<'a>(turns: &'a [scopeon_core::Turn]) -> MetricContext<'a> {
        MetricContext {
            turns,
            session: None,
            daily_rollups: &[],
            provider_name: "test",
            tool_calls: &[],
        }
    }

    fn empty_waste() -> WasteReport {
        WasteReport {
            signals: vec![],
            waste_score: 0.0,
        }
    }

    #[test]
    fn test_no_suggestions_empty() {
        let suggestions = compute_suggestions(&empty_ctx(&[]), &empty_waste(), None);
        assert!(suggestions.is_empty());
    }

    #[test]
    fn test_global_stats_below_avg_cache_fires() {
        // Session has 0% cache hit, historical average is 60% → should fire
        let turn = make_turn(1000, 0, 0.001);
        let turns = vec![turn];
        let global = GlobalStats {
            cache_hit_rate: 0.60,
            total_turns: 100,
            total_input_tokens: 50_000,
            estimated_cost_usd: 5.0,
            ..Default::default()
        };
        let suggestions = compute_suggestions(&empty_ctx(&turns), &empty_waste(), Some(&global));
        assert!(
            suggestions.iter().any(|s| s.id == "below-avg-cache"),
            "should fire below-avg-cache when session rate is far below historical"
        );
    }

    #[test]
    fn test_global_stats_no_fire_when_good_cache() {
        // Session has 65% cache hit, historical average is 60% → should NOT fire
        let turn = make_turn(400, 600, 0.001); // 600/(400+600) = 60%
        let turns = vec![turn];
        let global = GlobalStats {
            cache_hit_rate: 0.60,
            total_turns: 100,
            ..Default::default()
        };
        let suggestions = compute_suggestions(&empty_ctx(&turns), &empty_waste(), Some(&global));
        assert!(!suggestions.iter().any(|s| s.id == "below-avg-cache"));
    }

    #[test]
    fn test_global_stats_no_fire_when_insufficient_history() {
        let global = GlobalStats {
            cache_hit_rate: 0.60,
            total_turns: 5, // below threshold of 10
            ..Default::default()
        };
        let suggestions = compute_suggestions(&empty_ctx(&[]), &empty_waste(), Some(&global));
        assert!(!suggestions.iter().any(|s| s.id == "below-avg-cache"));
        assert!(!suggestions.iter().any(|s| s.id == "above-avg-input"));
        assert!(!suggestions.iter().any(|s| s.id == "high-cost-per-turn"));
    }

    #[test]
    fn test_high_cost_per_turn_fires() {
        // 5 turns at $0.10/turn, global average is $0.02/turn → fires
        let turns: Vec<_> = (0..5).map(|_| make_turn(1000, 0, 0.10)).collect();
        let global = GlobalStats {
            total_turns: 100,
            estimated_cost_usd: 2.0, // $0.02/turn
            ..Default::default()
        };
        let suggestions = compute_suggestions(&empty_ctx(&turns), &empty_waste(), Some(&global));
        assert!(
            suggestions.iter().any(|s| s.id == "high-cost-per-turn"),
            "should fire when cost per turn is 5× historical mean"
        );
    }
}
