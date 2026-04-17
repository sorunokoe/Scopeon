use crate::metric::MetricContext;
use crate::waste::{is_file_read_tool, WasteReport};
use scopeon_core::{
    derive_hook_effects, interaction_token_total, provider_capabilities, GlobalStats,
    PRICING_VERIFIED_DATE,
};
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

    // 6b. Provider-aware interaction suggestions — only emit when the provider supports them.
    let capabilities = provider_capabilities(ctx.provider_name);
    let hook_effects = derive_hook_effects(ctx.interaction_events);

    if capability_level(&capabilities, "mcp_identity") != "unsupported" {
        let mut server_counts = std::collections::BTreeMap::<String, (usize, i64)>::new();
        for event in ctx
            .interaction_events
            .iter()
            .filter(|e| e.kind == "mcp" && matches!(e.phase.as_str(), "single" | "complete"))
        {
            let server = event
                .mcp_server
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            let entry = server_counts.entry(server).or_insert((0, 0));
            entry.0 += 1;
            entry.1 += interaction_token_total(event);
        }
        if let Some((server, (calls, tokens))) = server_counts
            .into_iter()
            .max_by_key(|(_, (calls, tokens))| (*calls as i64, *tokens))
        {
            if calls >= 3 {
                suggestions.push(Suggestion {
                    id: "heavy-mcp-server",
                    severity: Severity::Info,
                    title: "Heavy MCP Server",
                    body: format!(
                        "{} dominates MCP usage in this session ({} calls, ~{}k attributed tokens) — \
                         consider narrowing its scope or using cheaper local discovery paths first",
                        server,
                        calls,
                        tokens / 1000
                    ),
                });
            }
        }
    }

    if capability_level(&capabilities, "task_history") != "unsupported" {
        if let Some(task) = ctx
            .task_runs
            .iter()
            .max_by_key(|t| (t.total_tokens.unwrap_or(0), t.total_tool_calls.unwrap_or(0)))
        {
            let total_tokens = task.total_tokens.unwrap_or(0);
            let total_tool_calls = task.total_tool_calls.unwrap_or(0);
            if total_tokens > 100_000 || total_tool_calls > 20 {
                suggestions.push(Suggestion {
                    id: "task-fanout",
                    severity: Severity::Warning,
                    title: "Task Fan-Out",
                    body: format!(
                        "Task '{}' expanded to {} tool calls and ~{}k tokens — split it into smaller scoped tasks or tighten the task prompt/model",
                        task.name,
                        total_tool_calls,
                        total_tokens / 1000
                    ),
                });
            }
            if task.prompt_size_chars > 8_000 {
                suggestions.push(Suggestion {
                    id: "task-prompt-bloat",
                    severity: Severity::Info,
                    title: "Large Task Prompt",
                    body: format!(
                        "Task '{}' started with a large prompt payload (~{}k chars) — trimming task setup can reduce subagent fan-out and context bloat",
                        task.name,
                        task.prompt_size_chars / 1000
                    ),
                });
            }
        }
    }

    if capability_level(&capabilities, "skills") != "unsupported" {
        let skill_count = ctx
            .interaction_events
            .iter()
            .filter(|e| e.kind == "skill")
            .count();
        let research_like = ctx
            .interaction_events
            .iter()
            .filter(|e| matches!(e.phase.as_str(), "single" | "start" | "complete"))
            .filter(|e| is_search_like(&e.name))
            .count();
        if skill_count == 0 && research_like >= 8 {
            suggestions.push(Suggestion {
                id: "skill-opportunity",
                severity: Severity::Info,
                title: "Skill Opportunity",
                body: format!(
                    "{} supports skills, and this session already shows {} discovery-heavy actions without one — use a project-specific skill before repeated search/read loops",
                    ctx.provider_name,
                    research_like
                ),
            });
        }
    }

    if capability_level(&capabilities, "hooks") != "unsupported" {
        let hook_starts = ctx
            .interaction_events
            .iter()
            .filter(|e| e.kind == "hook" && e.phase == "start")
            .count();
        if hook_starts > 0 {
            let modified = hook_effects
                .values()
                .filter(|effect| *effect == "modified")
                .count();
            let blocked = hook_effects
                .values()
                .filter(|effect| *effect == "blocked")
                .count();
            suggestions.push(Suggestion {
                id: "hook-activity",
                severity: if blocked > 0 {
                    Severity::Warning
                } else {
                    Severity::Info
                },
                title: "Hook Activity",
                body: format!(
                    "{} hook interventions observed: {} modified, {} blocked, {} passed through — hooks are a real control point in this workflow, so keep them intentional and project-specific",
                    hook_starts,
                    modified,
                    blocked,
                    hook_starts.saturating_sub(modified + blocked)
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

    // ── TRIZ S4: Semantic Turn Typing — FileHeavySession ─────────────────────
    // When the waste engine detected a file-read-dominated tool-call pattern,
    // surface an actionable caching suggestion here in the Insights tab.
    let file_heavy = waste.signals.iter().find_map(|s| {
        if let crate::waste::WasteKind::FileHeavySession {
            file_read_calls,
            total_tool_calls,
            read_pct,
        } = &s.kind
        {
            Some((*file_read_calls, *total_tool_calls, *read_pct))
        } else {
            None
        }
    });
    if let Some((reads, total, pct)) = file_heavy {
        suggestions.push(Suggestion {
            id: "file-heavy-session",
            severity: Severity::Info,
            title: "File-Read Dominated Session",
            body: format!(
                "{reads} of {total} tool calls ({pct:.0}%) are file reads. \
                 Pin the most-accessed files to the system prompt so Anthropic's \
                 prompt cache can serve them at cache-read price (~10× cheaper). \
                 Add files that stay stable across turns to a <cache_control> block.",
            ),
        });
    }

    // ── TRIZ S6: Conversation Phase Detection ────────────────────────────────
    // Classify the session's recent activity into a conversation phase and surface
    // the implication for context burn rate.  Uses tool-call patterns from the
    // last 5 turns; falls back to input-token trend when no tool data is present.
    if let Some((phase, implication)) = detect_conversation_phase(ctx) {
        suggestions.push(Suggestion {
            id: "conversation-phase",
            severity: Severity::Info,
            title: phase,
            body: implication.to_string(),
        });
    }

    // ── TRIZ S3: Pricing Staleness Warning ───────────────────────────────────
    // Alert the user when the compiled pricing table may be more than 30 days
    // out of date so they know cost estimates could drift.
    if let Some(stale_days) = pricing_staleness_days() {
        if stale_days >= 30 {
            suggestions.push(Suggestion {
                id: "pricing-stale",
                severity: if stale_days >= 90 {
                    Severity::Warning
                } else {
                    Severity::Info
                },
                title: "Pricing Table May Be Stale",
                body: format!(
                    "Built-in model pricing was last verified {stale_days} days ago \
                     ({PRICING_VERIFIED_DATE}). Run `scopeon reprice` after updating \
                     your binary, or add overrides in config.toml \
                     ([pricing.overrides.\"model-prefix\"]) to correct cost estimates.",
                ),
            });
        }
    }

    suggestions
}

fn capability_level<'a>(
    capabilities: &'a [scopeon_core::ProviderCapability],
    capability: &str,
) -> &'a str {
    capabilities
        .iter()
        .find(|c| c.capability == capability)
        .map(|c| c.level.as_str())
        .unwrap_or("unsupported")
}

fn is_search_like(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("search")
        || n.contains("rg")
        || n.contains("grep")
        || n.contains("view")
        || n.contains("read")
        || n.contains("fetch")
        || n.contains("web")
}

// ── TRIZ S6: Conversation Phase Detection ────────────────────────────────────

/// Detect the current conversation phase and return (phase_name, implication).
///
/// Phase classification is based on the tool-call composition of the last 5 turns.
/// Returns `None` when there is insufficient data (< 3 turns) or the session
/// is too ambiguous to classify with confidence.
///
/// Phases and their context burn-rate implications:
/// - **Exploration** — high read/search ratio → context fills rapidly
/// - **Focus**       — high edit/write ratio  → context stable, stay in session
/// - **Compaction**  — last turn was a compaction event → reset in progress
/// - **Wind-down**   — shrinking input tokens → safe to extend session further
fn detect_conversation_phase(ctx: &MetricContext) -> Option<(&'static str, &'static str)> {
    let n = ctx.turns.len();
    if n < 3 {
        return None;
    }

    // Compaction is authoritative — check before everything else.
    if ctx.turns.last().map(|t| t.is_compaction_event).unwrap_or(false) {
        return Some((
            "Phase: Compaction",
            "Context was just compacted — burn rate will drop sharply next turn. \
             Good moment to continue without a manual /compact.",
        ));
    }

    // Gather tool calls for the last 5 turns.
    let recent_turn_ids: std::collections::HashSet<&str> = ctx
        .turns
        .iter()
        .rev()
        .take(5)
        .map(|t| t.id.as_str())
        .collect();
    let recent_tools: Vec<&scopeon_core::ToolCall> = ctx
        .tool_calls
        .iter()
        .filter(|tc| recent_turn_ids.contains(tc.turn_id.as_str()))
        .collect();

    if !recent_tools.is_empty() {
        let total = recent_tools.len() as f64;
        let read_count = recent_tools
            .iter()
            .filter(|tc| is_file_read_tool(&tc.tool_name))
            .count() as f64;
        let search_count = recent_tools
            .iter()
            .filter(|tc| is_search_tool_name(&tc.tool_name))
            .count() as f64;
        let edit_count = recent_tools
            .iter()
            .filter(|tc| is_edit_tool_name(&tc.tool_name))
            .count() as f64;

        if (read_count + search_count) / total > 0.55 {
            return Some((
                "Phase: Exploration",
                "Recent turns are read/search-heavy — context is filling fast. \
                 Consider compacting once you have a clear plan, or pin key files \
                 to the system prompt to slow the fill rate.",
            ));
        }
        if edit_count / total > 0.40 {
            return Some((
                "Phase: Focus",
                "Recent turns are edit-heavy — context fill rate is stable. \
                 Good time to stay in the session and complete the task before compacting.",
            ));
        }
    }

    // Fallback: token-trend analysis (no tool data or ambiguous tool mix).
    if n >= 6 {
        let half = n / 2;
        let early_avg =
            ctx.turns[..half].iter().map(|t| t.input_tokens as f64).sum::<f64>() / half as f64;
        let late_avg = ctx.turns[half..]
            .iter()
            .map(|t| t.input_tokens as f64)
            .sum::<f64>()
            / (n - half) as f64;

        if early_avg > 0.0 && late_avg < early_avg * 0.75 {
            return Some((
                "Phase: Wind-down",
                "Input tokens per turn are declining — context pressure is easing. \
                 Safe to extend this session further before considering a compact.",
            ));
        }
        if early_avg > 0.0 && late_avg > early_avg * 1.35 {
            return Some((
                "Phase: Exploration",
                "Input tokens per turn are growing — context is filling faster than average. \
                 Monitor fill % closely and compact proactively to avoid mid-task interruption.",
            ));
        }
    }

    None
}

fn is_search_tool_name(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("search") || n.contains("grep") || n.contains("rg") || n == "find" || n == "glob"
}

fn is_edit_tool_name(name: &str) -> bool {
    let n = name.to_lowercase();
    n == "edit"
        || n == "create"
        || n.contains("write")
        || n.contains("edit")
        || n.contains("replace")
        || n.contains("str_replace")
        || n.contains("patch")
}

// ── TRIZ S3: Pricing Staleness ────────────────────────────────────────────────

/// Return the number of days since `PRICING_VERIFIED_DATE` was set.
/// Returns `None` when the date cannot be parsed (defensive — should never happen).
fn pricing_staleness_days() -> Option<i64> {
    let verified = chrono::NaiveDate::parse_from_str(PRICING_VERIFIED_DATE, "%Y-%m-%d").ok()?;
    let today = chrono::Utc::now().date_naive();
    Some((today - verified).num_days())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metric::MetricContext;
    use crate::waste::WasteReport;
    use scopeon_core::{GlobalStats, InteractionEvent, TaskRun};

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
            interaction_events: &[],
            task_runs: &[],
        }
    }

    fn ctx_with_provenance<'a>(
        provider_name: &'a str,
        interaction_events: &'a [InteractionEvent],
        task_runs: &'a [TaskRun],
    ) -> MetricContext<'a> {
        MetricContext {
            turns: &[],
            session: None,
            daily_rollups: &[],
            provider_name,
            tool_calls: &[],
            interaction_events,
            task_runs,
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

    #[test]
    fn test_skill_opportunity_fires_for_search_heavy_copilot_session() {
        let interactions: Vec<InteractionEvent> = (0..8)
            .map(|i| InteractionEvent {
                id: format!("evt-{i}"),
                session_id: "s1".into(),
                turn_id: None,
                task_run_id: None,
                correlation_id: None,
                parent_id: None,
                provider: "copilot-cli".into(),
                timestamp: 1_700_000_000_000 + i,
                kind: "tool".into(),
                phase: "complete".into(),
                name: if i % 2 == 0 { "rg" } else { "view" }.into(),
                display_name: None,
                mcp_server: None,
                mcp_tool: None,
                hook_type: None,
                agent_type: None,
                execution_mode: None,
                model: None,
                status: None,
                success: Some(true),
                input_size_chars: 100,
                output_size_chars: 50,
                prompt_size_chars: 0,
                summary_size_chars: 0,
                total_tokens: None,
                total_tool_calls: None,
                duration_ms: None,
                estimated_input_tokens: 10,
                estimated_output_tokens: 5,
                estimated_cost_usd: 0.0,
                confidence: "estimated".into(),
            })
            .collect();

        let suggestions = compute_suggestions(
            &ctx_with_provenance("copilot-cli", &interactions, &[]),
            &empty_waste(),
            None,
        );

        assert!(suggestions.iter().any(|s| s.id == "skill-opportunity"));
    }

    #[test]
    fn test_task_fanout_and_prompt_bloat_fire() {
        let tasks = vec![TaskRun {
            id: "task-1".into(),
            session_id: "s1".into(),
            correlation_id: Some("corr-1".into()),
            name: "repo-analyze".into(),
            display_name: Some("Repository analysis".into()),
            agent_type: "task".into(),
            execution_mode: "background".into(),
            requested_model: Some("claude-sonnet-4.5".into()),
            actual_model: Some("claude-sonnet-4.5".into()),
            started_at: 1_700_000_000_000,
            completed_at: Some(1_700_000_010_000),
            duration_ms: Some(10_000),
            success: Some(true),
            total_tokens: Some(150_000),
            total_tool_calls: Some(32),
            description_size_chars: 512,
            prompt_size_chars: 12_000,
            summary_size_chars: 256,
            confidence: "exact".into(),
        }];

        let suggestions = compute_suggestions(
            &ctx_with_provenance("copilot-cli", &[], &tasks),
            &empty_waste(),
            None,
        );

        assert!(suggestions.iter().any(|s| s.id == "task-fanout"));
        assert!(suggestions.iter().any(|s| s.id == "task-prompt-bloat"));
    }
}
