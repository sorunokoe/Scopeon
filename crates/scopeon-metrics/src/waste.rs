use crate::metric::MetricContext;
use crate::thresholds::UserThresholds;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

/// Stable serialization: each variant uses `"kind"` as the tag with snake_case names.
/// Do not reorder or rename variants without a semver bump — consumers depend on these strings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WasteKind {
    RedundantToolCalls {
        tool_name: String,
        count: usize,
    },
    ThinkingWaste {
        thinking_tokens: i64,
        output_tokens: i64,
        ratio: f64,
    },
    ColdCacheSession {
        hit_rate: f64,
        turns: usize,
    },
    ContextBloatTurn {
        turn_index: i64,
        input_tokens: i64,
        session_avg: f64,
    },
    /// TRIZ S4: Semantic Turn Typing — detected when the session is dominated by
    /// file-read tool calls, indicating an exploration/audit pattern where pinning
    /// key files to the system prompt and enabling cache could reduce input tokens.
    FileHeavySession {
        file_read_calls: usize,
        total_tool_calls: usize,
        read_pct: f64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasteSignal {
    pub kind: WasteKind,
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasteReport {
    pub signals: Vec<WasteSignal>,
    pub waste_score: f64,
}

impl WasteReport {
    pub fn compute(ctx: &MetricContext) -> Self {
        Self::compute_with_thresholds(ctx, &UserThresholds::default())
    }

    pub fn compute_with_thresholds(ctx: &MetricContext, thresholds: &UserThresholds) -> Self {
        let mut signals = Vec::new();

        // 1. Redundant tool calls: same tool_name + input fingerprint called >1 times.
        // Uses input_hash (FNV-1a) when available; falls back to input_size_chars for
        // rows written before M0006 migration (hash == 0 means "unknown").
        if !ctx.tool_calls.is_empty() {
            use std::collections::BTreeMap;
            let mut counts: BTreeMap<(&str, u64), usize> = BTreeMap::new();
            for tc in ctx.tool_calls {
                let key = if tc.input_hash != 0 {
                    tc.input_hash
                } else {
                    // Legacy fallback: treat size as a proxy (same behaviour as before M0006)
                    tc.input_size_chars as u64
                };
                *counts.entry((tc.tool_name.as_str(), key)).or_insert(0) += 1;
            }
            let mut tool_redundant: BTreeMap<String, usize> = BTreeMap::new();
            for ((name, _key), count) in &counts {
                if *count > 1 {
                    *tool_redundant.entry(name.to_string()).or_insert(0) += count - 1;
                }
            }
            for (tool_name, extra) in tool_redundant {
                let count = extra + 1;
                signals.push(WasteSignal {
                    kind: WasteKind::RedundantToolCalls {
                        tool_name: tool_name.clone(),
                        count,
                    },
                    severity: Severity::Warning,
                    message: format!("\"{}\" called {}× with identical input", tool_name, count),
                });
            }
        }

        // 2. Thinking waste: ratio above user's adaptive threshold
        if !ctx.turns.is_empty() {
            let thinking: i64 = ctx.turns.iter().map(|t| t.thinking_tokens).sum();
            let output: i64 = ctx.turns.iter().map(|t| t.output_tokens).sum();
            if output > 0 {
                let ratio = thinking as f64 / output as f64;
                if ratio > thresholds.thinking_ratio_warn {
                    signals.push(WasteSignal {
                        kind: WasteKind::ThinkingWaste {
                            thinking_tokens: thinking,
                            output_tokens: output,
                            ratio,
                        },
                        severity: if ratio > thresholds.thinking_ratio_critical {
                            Severity::Critical
                        } else {
                            Severity::Warning
                        },
                        message: format!("Avg {:.1}× more thinking than output tokens", ratio),
                    });
                }
            }
        }

        // 3. Cold cache: hit rate below user's adaptive threshold after turn 3
        if ctx.turns.len() > 3 {
            let later_turns = &ctx.turns[3..];
            let cache_read: i64 = later_turns.iter().map(|t| t.cache_read_tokens).sum();
            let input: i64 = later_turns.iter().map(|t| t.input_tokens).sum();
            let cache_write: i64 = later_turns.iter().map(|t| t.cache_write_tokens).sum();
            let total = cache_read + input + cache_write;
            if total > 0 {
                let hit_rate = cache_read as f64 / total as f64 * 100.0;
                if hit_rate < thresholds.cold_cache_pct {
                    signals.push(WasteSignal {
                        kind: WasteKind::ColdCacheSession {
                            hit_rate,
                            turns: ctx.turns.len(),
                        },
                        severity: Severity::Warning,
                        message: format!(
                            "Cache hit rate only {:.1}% after turn 3 — cold cache",
                            hit_rate
                        ),
                    });
                }
            }
        }

        // 4. Context bloat: report the worst-case turn (highest ratio vs session avg).
        if !ctx.turns.is_empty() {
            let avg_input = ctx.turns.iter().map(|t| t.input_tokens as f64).sum::<f64>()
                / ctx.turns.len() as f64;
            let multiplier = thresholds.context_bloat_multiplier;
            if avg_input > 0.0 {
                let worst = ctx
                    .turns
                    .iter()
                    .filter(|t| t.input_tokens as f64 > avg_input * multiplier)
                    .max_by(|a, b| a.input_tokens.cmp(&b.input_tokens));
                if let Some(turn) = worst {
                    signals.push(WasteSignal {
                        kind: WasteKind::ContextBloatTurn {
                            turn_index: turn.turn_index,
                            input_tokens: turn.input_tokens,
                            session_avg: avg_input,
                        },
                        severity: Severity::Info,
                        message: format!(
                            "Turn {} input {}k > {:.0}× session avg {:.0}k",
                            turn.turn_index,
                            turn.input_tokens / 1000,
                            multiplier,
                            avg_input / 1000.0
                        ),
                    });
                }
            }
        }

        // 5. File-heavy session (TRIZ S4 — Semantic Turn Typing).
        // Fires when ≥ 40 % of all tool calls are file-read operations AND the
        // session has accumulated ≥ 10 such calls across ≥ 5 turns.  This pattern
        // indicates an exploration or audit workflow where many files are read
        // per turn — a prime candidate for system-prompt caching of key files.
        if ctx.turns.len() >= 5 && !ctx.tool_calls.is_empty() {
            let file_read_calls = ctx
                .tool_calls
                .iter()
                .filter(|tc| is_file_read_tool(&tc.tool_name))
                .count();
            let total = ctx.tool_calls.len();
            let read_pct = file_read_calls as f64 / total as f64 * 100.0;
            if file_read_calls >= 10 && read_pct >= 40.0 {
                signals.push(WasteSignal {
                    kind: WasteKind::FileHeavySession {
                        file_read_calls,
                        total_tool_calls: total,
                        read_pct,
                    },
                    severity: Severity::Info,
                    message: format!(
                        "{} of {} tool calls ({:.0}%) are file reads — pin key files to system prompt",
                        file_read_calls, total, read_pct
                    ),
                });
            }
        }

        // Sort for stable display order: Critical → Warning → Info, then by message
        signals.sort_by(|a, b| {
            let sev_ord = |s: &Severity| match s {
                Severity::Critical => 0,
                Severity::Warning => 1,
                Severity::Info => 2,
            };
            sev_ord(&a.severity)
                .cmp(&sev_ord(&b.severity))
                .then(a.message.cmp(&b.message))
        });

        // Severity-weighted waste score: Critical=40, Warning=20, Info=5, capped at 100.
        // This prevents low-severity signals from scoring the same as a Critical.
        let waste_score: f64 = signals
            .iter()
            .map(|s| match s.severity {
                Severity::Critical => 40.0,
                Severity::Warning => 20.0,
                Severity::Info => 5.0,
            })
            .sum::<f64>()
            .min(100.0);
        WasteReport {
            signals,
            waste_score,
        }
    }
}

/// Returns true when a tool name indicates a file-reading operation.
///
/// Used by S4 Semantic Turn Typing (FileHeavySession detection). The list covers
/// the most common file-read tool names across Claude Code, Copilot CLI, and Aider.
/// Intentionally conservative — bash/shell are excluded because they could be
/// anything, and false negatives are safer than false positives here.
pub(crate) fn is_file_read_tool(name: &str) -> bool {
    let n = name.to_lowercase();
    // Exact names
    matches!(n.as_str(), "view" | "cat" | "head" | "tail" | "glob" | "read_file")
        // Prefix/infix patterns
        || n.starts_with("read")
        || n.contains("read_file")
        || n.contains("view_file")
        // Filesystem listing that loads paths into context
        || n == "find"
        || n == "ls"
}
