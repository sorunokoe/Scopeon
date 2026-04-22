/// MCP (Model Context Protocol) server over stdio.
///
/// Implements JSON-RPC 2.0 with the MCP protocol subset needed for tool serving.
/// Claude Code connects to this via the mcpServers config in ~/.claude/settings.json.
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tracing::{debug, info};

use scopeon_core::{
    derive_hook_effects, interaction_token_total, provider_capabilities, redact_webhook_url,
    Database, UserConfig,
};
use scopeon_metrics::{compute_suggestions, MetricContext, WasteReport};

/// §4.5: Single shared stdout writer protected by an async Mutex.
/// Prevents interleaved JSON-RPC lines if background tasks ever write notifications
/// concurrently with the main request handler. `BufWriter` batches the JSON line
/// + newline into a single kernel write, avoiding partial-line races.
type SharedWriter = Arc<tokio::sync::Mutex<BufWriter<tokio::io::Stdout>>>;

// ── Pre-Computation Engine ────────────────────────────────────────────────────

/// Pre-computed MCP tool responses, refreshed by a background Tokio task.
///
/// All fields hold the JSON `Value` returned verbatim for the corresponding
/// tool call with no arguments (or default arguments). Reading from this struct
/// costs ~200 ns (RwLock read + clone) versus 1–50 ms for a live DB query.
///
/// **Adaptive state machine** — the refresh interval scales with risk:
/// - IDLE   (context fill < 50%) → 5 s
/// - ACTIVE (context fill 50–80%) → 1 s  
/// - CRISIS (context fill > 80%) → 200 ms
#[derive(Debug, Default, Clone)]
struct MetricSnapshot {
    token_usage: Option<Value>,
    session_summary: Option<Value>,
    cache_efficiency: Option<Value>,
    history_30d: Option<Value>,
    context_pressure: Option<Value>,
    budget_status: Option<Value>,
    optimization_suggestions: Option<Value>,
    interaction_history: Option<Value>,
    task_history: Option<Value>,
    provider_capabilities: Option<Value>,
    suggest_compact: Option<Value>,
    project_stats: Option<Value>,
    sessions_list: Option<Value>,
    agent_tree: Option<Value>,
    /// Unix timestamp (seconds) of last successful refresh.
    #[allow(dead_code)] // exposed via refreshed_at in MCP snapshot metadata
    refreshed_at_unix: Option<i64>,
}

/// Compute the adaptive refresh interval from the current snapshot.
/// Reads `fill_pct` from the cached `context_pressure` result.
fn adaptive_interval(snap: &MetricSnapshot) -> std::time::Duration {
    let fill = snap
        .context_pressure
        .as_ref()
        .and_then(|v| v.get("fill_pct"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    if fill >= 80.0 {
        std::time::Duration::from_millis(200) // CRISIS
    } else if fill >= 50.0 {
        std::time::Duration::from_secs(1) // ACTIVE
    } else {
        std::time::Duration::from_secs(5) // IDLE
    }
}

/// Compute a fresh `MetricSnapshot` from the database.
/// The caller must hold the DB mutex.
fn refresh_snapshot(db: &Database, config: &UserConfig) -> MetricSnapshot {
    MetricSnapshot {
        token_usage: Some(handle_get_token_usage(db)),
        session_summary: Some(handle_get_session_summary(db, None)),
        cache_efficiency: Some(handle_get_cache_efficiency(db, None)),
        history_30d: Some(handle_get_history(db, 30)),
        context_pressure: Some(handle_get_context_pressure(db)),
        budget_status: Some(handle_get_budget_status(db, config)),
        optimization_suggestions: Some(handle_get_optimization_suggestions(db)),
        interaction_history: Some(handle_get_interaction_history(db, None, 100)),
        task_history: Some(handle_get_task_history(db, None, 25)),
        provider_capabilities: Some(handle_get_provider_capabilities(db, None)),
        suggest_compact: Some(handle_suggest_compact(db)),
        project_stats: Some(handle_get_project_stats(db, None)),
        sessions_list: Some(handle_list_sessions(db, 20)),
        agent_tree: Some(handle_get_agent_tree(db, None)),
        refreshed_at_unix: Some(chrono::Utc::now().timestamp()),
    }
}

/// Execute a live DB query, returning a fallback error Value if the mutex is poisoned.
fn live_query<F: FnOnce(&Database) -> Value>(db: &Arc<Mutex<Database>>, f: F) -> Value {
    match db.lock() {
        Ok(guard) => f(&guard),
        Err(_) => json!({ "error": "Database mutex poisoned — restart Scopeon" }),
    }
}

/// Return the cached value if the snapshot is populated; otherwise fall back to a live query.
fn snap_or_live<F: FnOnce(&Arc<Mutex<Database>>) -> Value>(
    cached: Option<Value>,
    db: &Arc<Mutex<Database>>,
    fallback: F,
) -> Value {
    cached.unwrap_or_else(|| fallback(db))
}

// ── Proactive Push Notification System ───────────────────────────────────────

/// Alert types emitted by the background refresh task.
///
/// Each variant maps to a JSON-RPC notification sent to the MCP client with
/// no `id` field (fire-and-forget per JSON-RPC 2.0 §4).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum AlertKind {
    ContextCrisis,  // fill_pct >= 95%
    ContextWarning, // fill_pct >= 80%
    BudgetWarning,  // daily spend > 90% of limit
    LowTurnsLeft,   // predicted_turns_remaining <= 5
    CompactionDetected,
    CompactionAdvisory, // S-7: pre-crisis optimal compaction window (55–79%, accelerating)
}

#[derive(Debug, Clone)]
struct Alert {
    payload: Value,
}

/// Deduplicated alert debounce state: tracks when each AlertKind was last sent.
struct AlertDebounce {
    last_sent: std::collections::HashMap<AlertKind, Instant>,
    cooldown: Duration,
}

impl AlertDebounce {
    fn new(cooldown_secs: u64) -> Self {
        Self {
            last_sent: std::collections::HashMap::new(),
            cooldown: Duration::from_secs(cooldown_secs),
        }
    }

    /// Returns `true` if this alert kind should fire now (not in cooldown).
    fn should_fire(&mut self, kind: &AlertKind) -> bool {
        let now = Instant::now();
        let last = self.last_sent.get(kind).copied();
        if last
            .map(|t| now.duration_since(t) < self.cooldown)
            .unwrap_or(false)
        {
            return false;
        }
        self.last_sent.insert(kind.clone(), now);
        true
    }
}

/// Inspect the freshly-computed snapshot for alert conditions and enqueue
/// any that pass the debounce gate.
fn check_alerts(
    snap: &MetricSnapshot,
    debounce: &mut AlertDebounce,
    tx: &tokio::sync::mpsc::Sender<Alert>,
) {
    // 1. Context pressure
    let fill_pct = snap
        .context_pressure
        .as_ref()
        .and_then(|v| v.get("fill_pct"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);

    if fill_pct >= 95.0 && debounce.should_fire(&AlertKind::ContextCrisis) {
        let _ = tx.try_send(Alert {
            payload: json!({
                "method": "notifications/scopeon/alert",
                "params": {
                    "type": "context_crisis",
                    "severity": "critical",
                    "message": format!("Context window {:.0}% full — run /compact immediately", fill_pct),
                    "fill_pct": fill_pct,
                    "should_compact": true,
                }
            }),
        });
    } else if (80.0..95.0).contains(&fill_pct) && debounce.should_fire(&AlertKind::ContextWarning) {
        let predicted = snap
            .context_pressure
            .as_ref()
            .and_then(|v| v.get("predicted_turns_remaining"))
            .and_then(Value::as_i64);
        let _ = tx.try_send(Alert {
            payload: json!({
                "method": "notifications/scopeon/alert",
                "params": {
                    "type": "context_warning",
                    "severity": "warning",
                    "message": format!(
                        "Context window {:.0}% full{}",
                        fill_pct,
                        predicted.map(|t| format!(" — ~{} turns remaining", t)).unwrap_or_default()
                    ),
                    "fill_pct": fill_pct,
                    "predicted_turns_remaining": predicted,
                }
            }),
        });
    }

    // 2. Low predicted turns remaining (high urgency even if < 80% fill)
    let predicted_turns = snap
        .context_pressure
        .as_ref()
        .and_then(|v| v.get("predicted_turns_remaining"))
        .and_then(Value::as_i64);
    if let Some(turns) = predicted_turns {
        if turns <= 5 && debounce.should_fire(&AlertKind::LowTurnsLeft) {
            let _ = tx.try_send(Alert {
                payload: json!({
                    "method": "notifications/scopeon/alert",
                    "params": {
                        "type": "low_turns_left",
                        "severity": "warning",
                        "message": format!("Only ~{} turns left before context is exhausted", turns),
                        "predicted_turns_remaining": turns,
                    }
                }),
            });
        }
    }

    // 3. Budget warning (over 90% of daily limit)
    if let Some(budget) = &snap.budget_status {
        let over = budget
            .get("over_budget")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let daily_spent = budget
            .get("daily_spent")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let daily_limit = budget
            .get("daily_limit")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let near_limit = daily_limit > 0.0 && daily_spent / daily_limit > 0.90;
        if (over || near_limit) && debounce.should_fire(&AlertKind::BudgetWarning) {
            let _ = tx.try_send(Alert {
                payload: json!({
                    "method": "notifications/scopeon/alert",
                    "params": {
                        "type": "budget_warning",
                        "severity": if over { "critical" } else { "warning" },
                        "message": if over {
                            format!("Daily budget exceeded: ${:.2} spent (limit ${:.2})", daily_spent, daily_limit)
                        } else {
                            format!("Near daily budget: ${:.2} of ${:.2} used ({:.0}%)", daily_spent, daily_limit, daily_spent / daily_limit * 100.0)
                        },
                        "daily_spent": daily_spent,
                        "daily_limit": daily_limit,
                        "over_budget": over,
                    }
                }),
            });
        }
    }

    // 4. Compaction detected (is_compaction_event on latest turn)
    let compaction = snap
        .suggest_compact
        .as_ref()
        .and_then(|v| v.get("last_compaction_turn"))
        .and_then(Value::as_i64)
        .map(|t| t >= 0)
        .unwrap_or(false);
    if compaction && debounce.should_fire(&AlertKind::CompactionDetected) {
        let _ = tx.try_send(Alert {
            payload: json!({
                "method": "notifications/scopeon/alert",
                "params": {
                    "type": "compaction_detected",
                    "severity": "info",
                    "message": "Auto-compaction detected — context was reset",
                }
            }),
        });
    }
}

/// S-7: Compute a compaction advisory score (0–1) from fill history.
///
/// The score combines:
/// - Current fill percentage (higher = more urgent)
/// - Fill acceleration (rate-of-change of slope — catches fast-rising sessions)
/// - Inverse cache-write fraction (compaction is cheaper when the model is NOT
///   mid-write: high cache_write_frac means a compaction right now would discard
///   an expensive cache layer, so we back off)
///
/// Returns a score in [0, 1]. Fire advisory when score > 0.65 and fill is in the
/// 55–79% pre-crisis window.
fn compaction_advisory_score(
    fill_pct: f64,
    fill_history: &std::collections::VecDeque<f64>,
    cache_write_frac: f64,
) -> f64 {
    // Need at least 3 samples for acceleration measurement.
    if fill_history.len() < 3 {
        return 0.0;
    }
    let n = fill_history.len();
    // Recent slope (last two samples).
    let s1 = fill_history[n - 1] - fill_history[n - 2];
    // Earlier slope (second-to-last pair).
    let s0 = fill_history[n - 2] - fill_history[n - 3];

    // Acceleration ratio: clamped and normalised to [0, 1].
    let accel_ratio = if s0.abs() > 0.5 {
        ((s1 - s0) / s0.abs()).clamp(0.0, 3.0) / 3.0
    } else if s1 > 1.0 {
        // s0 near zero but s1 is rising — treat as moderate acceleration.
        0.4
    } else {
        0.0
    };

    // Penalise when cache writes are active (compact later to avoid discarding cache).
    let cache_penalty = 1.0 - cache_write_frac.clamp(0.0, 1.0);

    (fill_pct / 100.0) * (0.4 + 0.6 * accel_ratio) * cache_penalty
}

/// S-7: Check whether the advisory compaction notification should fire.
fn check_compaction_advisory(
    snap: &MetricSnapshot,
    fill_history: &std::collections::VecDeque<f64>,
    debounce: &mut AlertDebounce,
    tx: &tokio::sync::mpsc::Sender<Alert>,
) {
    let fill_pct = snap
        .context_pressure
        .as_ref()
        .and_then(|v| v.get("fill_pct"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);

    // Only advise in the pre-crisis window.
    if !(55.0..80.0).contains(&fill_pct) {
        return;
    }

    // Compute cache-write fraction to avoid advising mid-cache-write.
    let cache_write_frac = snap
        .token_usage
        .as_ref()
        .and_then(|v| {
            let write = v
                .get("cache_write_tokens")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let total = v.get("total_tokens").and_then(Value::as_f64).unwrap_or(0.0);
            (total > 0.0).then_some(write / total)
        })
        .unwrap_or(0.0);

    let score = compaction_advisory_score(fill_pct, fill_history, cache_write_frac);

    if score > 0.65 && debounce.should_fire(&AlertKind::CompactionAdvisory) {
        let predicted = snap
            .context_pressure
            .as_ref()
            .and_then(|v| v.get("predicted_turns_remaining"))
            .and_then(Value::as_i64);
        let _ = tx.try_send(Alert {
            payload: json!({
                "method": "notifications/scopeon/alert",
                "params": {
                    "type": "compaction_advisory",
                    "severity": "info",
                    "message": format!(
                        "Optimal compaction window: context {:.0}% full and accelerating{}. \
                         Compact now to maximise cache savings.",
                        fill_pct,
                        predicted
                            .map(|t| format!(" (~{} turns remain)", t))
                            .unwrap_or_default()
                    ),
                    "fill_pct": fill_pct,
                    "advisory_score": score,
                    "predicted_turns_remaining": predicted,
                    "should_compact": true,
                }
            }),
        });
    }
}

/// Fire configured webhooks for an alert.
///
/// Uses a plain HTTP/1.1 POST over tokio TCP — no reqwest dependency.
/// Failures are logged as warnings but never propagated to the caller.
async fn fire_webhooks(config: &UserConfig, alert_type: &str, payload: &Value) {
    let body = match serde_json::to_string(payload) {
        Ok(body) => body,
        Err(e) => {
            tracing::warn!("Failed to serialize webhook payload: {}", e);
            return;
        },
    };

    for wh in &config.alerts.webhooks {
        // Filter by configured event types (empty list = all events)
        if !wh.events.is_empty() && !wh.events.iter().any(|e| e == alert_type) {
            continue;
        }

        let url = wh.url.clone();
        let display_url = wh.redacted_url();
        let body = body.clone();

        tokio::spawn(async move {
            if let Err(e) = do_http_post(&url, &body).await {
                tracing::warn!("Webhook POST to {} failed: {}", display_url, e);
            }
        });
    }
}

/// HTTP POST using the system `curl` binary.
///
/// This approach supports both HTTP and HTTPS endpoints (Slack, Discord, PagerDuty,
/// GitHub webhooks) without any additional Rust crate dependencies. `curl` is present
/// on all Scopeon target platforms (macOS, Linux).
///
/// TRIZ D2: #24 Mediator — OS curl acts as intermediate substance between alert
/// payload and HTTPS endpoints. Resolves NE-A (silent HTTPS webhook failures).
async fn do_http_post(url: &str, body: &str) -> Result<()> {
    let display_url = redact_webhook_url(url);
    let status = tokio::process::Command::new("curl")
        .args([
            "-s",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "--data-raw",
            body,
            "--max-time",
            "10",
            "--retry",
            "2",
            "--retry-delay",
            "1",
            url,
        ])
        .output()
        .await?;

    if !status.status.success() {
        anyhow::bail!(
            "curl exited with status {} for URL: {}",
            status.status,
            display_url
        );
    }

    // Check the HTTP response code (captured via -w %{http_code}).
    let http_code = String::from_utf8_lossy(&status.stdout);
    let code: u16 = http_code.trim().parse().with_context(|| {
        format!(
            "Invalid HTTP status code '{}' from webhook {}",
            http_code.trim(),
            display_url
        )
    })?;
    if !(200..300).contains(&code) {
        anyhow::bail!("Webhook to {} returned HTTP {}", display_url, code);
    }

    Ok(())
}

// ── JSON-RPC types ────────────────────────────────────────────────────────────

#[derive(Deserialize, Debug)]
struct JsonRpcRequest {
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

impl JsonRpcResponse {
    fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }
    fn err(id: Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

// ── Tool definitions ──────────────────────────────────────────────────────────

fn tool_list() -> Value {
    json!({
        "tools": [
            {
                "name": "get_token_usage",
                "description": "Get token usage breakdown for the current (most recent) Claude Code session. Shows input, output, cache hit/miss, thinking, and MCP tool call tokens.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "get_session_summary",
                "description": "Get a detailed summary of a specific session or the most recent one. Includes per-turn breakdown, cost estimate, and cache efficiency.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "Session ID to query. Omit for most recent session."
                        }
                    }
                }
            },
            {
                "name": "get_cache_efficiency",
                "description": "Get prompt cache hit rate, tokens saved by caching, and estimated cost savings for the current or a specified session.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "Session ID. Omit for most recent session."
                        }
                    }
                }
            },
            {
                "name": "get_history",
                "description": "Get token usage history across sessions, optionally filtered by number of recent days.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "days": {
                            "type": "integer",
                            "description": "Number of recent days to include (default: 30)."
                        }
                    }
                }
            },
            {
                "name": "compare_sessions",
                "description": "Compare two sessions side-by-side. Useful for measuring impact of optimization changes.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "session_id_a": { "type": "string" },
                        "session_id_b": { "type": "string" }
                    },
                    "required": ["session_id_a", "session_id_b"]
                }
            },
            {
                "name": "get_context_pressure",
                "description": "Get real-time context window fill % for the current session. Returns fill_pct, tokens_remaining, context_window_size, model, and should_compact recommendation.",
                "inputSchema": { "type": "object", "properties": {}, "required": [] }
            },
            {
                "name": "get_budget_status",
                "description": "Get daily/weekly/monthly spend vs configured budget limits. Returns over_budget status and projected monthly cost.",
                "inputSchema": { "type": "object", "properties": {}, "required": [] }
            },
            {
                "name": "get_optimization_suggestions",
                "description": "Get actionable suggestions to reduce token usage and cost. Analyzes the current session for waste patterns (repeated context, over-use of tools, etc.).",
                "inputSchema": { "type": "object", "properties": {}, "required": [] }
            },
            {
                "name": "get_interaction_history",
                "description": "Get normalized provenance for tool, MCP, skill, hook, and subagent interactions in a session. Includes confidence-tagged token attribution and derived hook effects.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "Session ID to query. Omit for most recent session."
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of recent interaction events to return (default: 100)."
                        }
                    }
                }
            },
            {
                "name": "get_task_history",
                "description": "Get derived task and subagent history for a session, including prompt sizes, tool fan-out, token totals, models, and related interaction summaries.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "Session ID to query. Omit for most recent session."
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of most-recent task runs to return (default: 25)."
                        }
                    }
                }
            },
            {
                "name": "get_provider_capabilities",
                "description": "Show which provenance features the current provider supports exactly, estimates, or cannot expose, so downstream suggestions do not pretend unsupported detail exists.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "session_id": {
                            "type": "string",
                            "description": "Session ID to inspect. Omit for most recent session."
                        }
                    }
                }
            },
            {
                "name": "suggest_compact",
                "description": "Returns whether you should run /compact right now, with reason, current fill%, and how many turns since the last compaction.",
                "inputSchema": { "type": "object", "properties": {}, "required": [] }
            },
            {
                "name": "get_project_stats",
                "description": "Get cost/token/session breakdown grouped by project. Optionally filtered by recency.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "top_n": { "type": "integer", "description": "Return only the top N projects by cost (default: all)." }
                    }
                }
            },
            {
                "name": "list_sessions",
                "description": "List recent sessions with cost, prompt cache hit rate, model, project, and turn count. Useful for picking a session_id to query.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "limit": { "type": "integer", "description": "Max sessions to return (default: 20)." }
                    }
                }
            },
            {
                "name": "get_agent_tree",
                "description": "Get the full multi-agent subagent tree for a root session. Returns nested JSON with per-agent cost and turn count.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string", "description": "Root session ID. Omit for most recent." }
                    }
                }
            },
            {
                "name": "set_session_tags",
                "description": "Set one or more tags on a session for cost attribution and filtering. Tags are free-text labels (e.g. 'feat-auth', 'sprint-12'). Replaces any existing tags.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string", "description": "Session ID to tag. Omit for most recent session." },
                        "tags": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "List of tags to apply."
                        }
                    },
                    "required": ["tags"]
                }
            },
            {
                "name": "get_cost_by_tag",
                "description": "Get cost breakdown grouped by session tag. Returns total cost and session count for each tag.",
                "inputSchema": { "type": "object", "properties": {}, "required": [] }
            }
        ]
    })
}

// ── Tool handlers ─────────────────────────────────────────────────────────────

fn handle_get_token_usage(db: &Database) -> Value {
    let session_id = match db.get_latest_session_id() {
        Ok(Some(id)) => id,
        _ => return json!({"error": "No sessions found. Run Claude Code first."}),
    };
    match db.get_session_aggregates(&session_id) {
        Ok(stats) => {
            let total_context = stats.total_input_tokens + stats.total_cache_read_tokens;
            json!({
                "session_id": session_id,
                "session_slug": stats.session.as_ref().map(|s| &s.slug),
                "model": stats.session.as_ref().map(|s| &s.model),
                "total_turns": stats.total_turns,
                "token_breakdown": {
                    "prompt_input_tokens": stats.total_input_tokens,
                    "cache_hit_tokens": stats.total_cache_read_tokens,
                    "cache_write_tokens": stats.total_cache_write_tokens,
                    "output_tokens": stats.total_output_tokens,
                    "thinking_tokens": stats.total_thinking_tokens,
                    "mcp_calls": stats.total_mcp_calls,
                    "total_context_tokens": total_context,
                },
                "cost": {
                    "estimated_usd": stats.estimated_cost_usd,
                    "cache_savings_usd": stats.cache_savings_usd,
                },
                "cache_hit_rate_pct": stats.cache_hit_rate * 100.0,
            })
        },
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn handle_get_session_summary(db: &Database, session_id: Option<&str>) -> Value {
    let sid = match resolve_session_id(db, session_id) {
        Ok(id) => id,
        Err(err) => return err,
    };
    match db.get_session_stats(&sid) {
        Ok(stats) => {
            let turns_summary: Vec<Value> = stats
                .turns
                .iter()
                .map(|t| {
                    json!({
                        "turn": t.turn_index,
                        "input": t.input_tokens,
                        "cache_hit": t.cache_read_tokens,
                        "cache_write": t.cache_write_tokens,
                        "output": t.output_tokens,
                        "thinking": t.thinking_tokens,
                        "mcp_calls": t.mcp_call_count,
                        "duration_ms": t.duration_ms,
                        "cost_usd": t.estimated_cost_usd,
                    })
                })
                .collect();
            let session = stats.session.as_ref();
            json!({
                "session_id": sid,
                "session": {
                    "slug": session.map(|s| s.slug.clone()),
                    "project": session.map(|s| s.project_name.clone()),
                    "branch": session.map(|s| s.git_branch.clone()),
                    "provider": session.map(|s| s.provider.clone()),
                    "provider_version": session.map(|s| s.provider_version.clone()),
                    "model": session.map(|s| s.model.clone()),
                    "context_window_tokens": session.and_then(|s| s.context_window_tokens),
                    "started_at": session.map(|s| s.started_at),
                    "last_turn_at": session.map(|s| s.last_turn_at),
                },
                "turns": turns_summary,
                "totals": {
                    "input_tokens": stats.total_input_tokens,
                    "cache_hit_tokens": stats.total_cache_read_tokens,
                    "cache_write_tokens": stats.total_cache_write_tokens,
                    "output_tokens": stats.total_output_tokens,
                    "thinking_tokens": stats.total_thinking_tokens,
                    "mcp_calls": stats.total_mcp_calls,
                    "estimated_cost_usd": stats.estimated_cost_usd,
                    "cache_hit_rate_pct": stats.cache_hit_rate * 100.0,
                }
            })
        },
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn handle_get_cache_efficiency(db: &Database, session_id: Option<&str>) -> Value {
    let sid = match session_id {
        Some(id) => id.to_string(),
        None => match db.get_latest_session_id() {
            Ok(Some(id)) => id,
            _ => return json!({"error": "No sessions found."}),
        },
    };
    match db.get_session_aggregates(&sid) {
        Ok(stats) => {
            let total_billable = stats.total_input_tokens
                + stats.total_cache_read_tokens
                + stats.total_cache_write_tokens;
            json!({
                "session_id": sid,
                "cache_hit_tokens": stats.total_cache_read_tokens,
                "cache_write_tokens": stats.total_cache_write_tokens,
                "total_billable_input_tokens": total_billable,
                "cache_hit_rate_pct": stats.cache_hit_rate * 100.0,
                "tokens_saved": stats.total_cache_read_tokens,
                "cost_savings_usd": stats.cache_savings_usd,
            })
        },
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn handle_get_history(db: &Database, days: i64) -> Value {
    match db.get_daily_rollups(days) {
        Ok(rollups) => {
            let data: Vec<Value> = rollups
                .iter()
                .map(|r| {
                    json!({
                        "date": r.date,
                        "sessions": r.session_count,
                        "turns": r.turn_count,
                        "input_tokens": r.total_input_tokens,
                        "cache_hit_tokens": r.total_cache_read_tokens,
                        "output_tokens": r.total_output_tokens,
                        "thinking_tokens": r.total_thinking_tokens,
                        "mcp_calls": r.total_mcp_calls,
                        "estimated_cost_usd": r.estimated_cost_usd,
                    })
                })
                .collect();
            json!({"history": data, "days": days})
        },
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn resolve_session_id(
    db: &Database,
    session_id: Option<&str>,
) -> std::result::Result<String, Value> {
    match session_id {
        Some(id) => Ok(id.to_string()),
        None => match db.get_latest_session_id() {
            Ok(Some(id)) => Ok(id),
            _ => Err(json!({"error": "No sessions found."})),
        },
    }
}

fn handle_compare_sessions(db: &Database, id_a: &str, id_b: &str) -> Value {
    let a = db.get_session_aggregates(id_a);
    let b = db.get_session_aggregates(id_b);
    // Determine which session is missing before consuming the results.
    let a_missing = a.is_err();
    let b_missing = b.is_err();
    match (a, b) {
        (Ok(a), Ok(b)) => {
            let cost_delta = b.estimated_cost_usd - a.estimated_cost_usd;
            let cache_delta = b.cache_hit_rate - a.cache_hit_rate;
            json!({
                "session_a": {
                    "id": id_a,
                    "turns": a.total_turns,
                    "input_tokens": a.total_input_tokens,
                    "cache_hit_tokens": a.total_cache_read_tokens,
                    "cache_hit_rate_pct": a.cache_hit_rate * 100.0,
                    "cost_usd": a.estimated_cost_usd,
                },
                "session_b": {
                    "id": id_b,
                    "turns": b.total_turns,
                    "input_tokens": b.total_input_tokens,
                    "cache_hit_tokens": b.total_cache_read_tokens,
                    "cache_hit_rate_pct": b.cache_hit_rate * 100.0,
                    "cost_usd": b.estimated_cost_usd,
                },
                "delta": {
                    "cost_usd": cost_delta,
                    "cache_hit_rate_pct": cache_delta * 100.0,
                    "cost_change": if cost_delta < 0.0 { "reduced (better)" } else { "increased" },
                }
            })
        },
        _ => {
            let missing = match (a_missing, b_missing) {
                (true, true) => format!("'{}' and '{}'", id_a, id_b),
                (true, false) => format!("'{}'", id_a),
                _ => format!("'{}'", id_b),
            };
            json!({"error": format!("Session {} not found.", missing)})
        },
    }
}

// ── New tool handlers (Phase 1–4) ─────────────────────────────────────────────

/// Fit a least-squares linear slope to the last ≤10 turns' token sizes and
/// return how many more turns are predicted before `tokens_remaining` runs out.
///
/// Returns `None` when there are fewer than 3 data points (not enough signal)
/// or when the trend is flat / decreasing (no meaningful countdown to show).
fn predict_turns_remaining(turns: &[scopeon_core::Turn], tokens_remaining: i64) -> Option<i64> {
    // Use the last 10 turns; fewer is fine but we need ≥3 for a meaningful slope.
    let window: Vec<_> = turns.iter().rev().take(10).rev().collect();
    if window.len() < 3 {
        return None;
    }
    let n = window.len() as f64;
    // x = turn index within the window (0, 1, …), y = tokens used that turn.
    let xs: Vec<f64> = (0..window.len()).map(|i| i as f64).collect();
    let ys: Vec<f64> = window
        .iter()
        .map(|t| (t.input_tokens + t.cache_read_tokens) as f64)
        .collect();
    let x_mean = xs.iter().sum::<f64>() / n;
    let y_mean = ys.iter().sum::<f64>() / n;
    let numerator: f64 = xs
        .iter()
        .zip(ys.iter())
        .map(|(&x, &y)| (x - x_mean) * (y - y_mean))
        .sum();
    let denominator: f64 = xs.iter().map(|&x| (x - x_mean).powi(2)).sum();
    if denominator < 1.0 {
        return None; // all turns at the same x — degenerate case
    }
    let slope = numerator / denominator;
    // Only show a countdown when the context is growing (positive slope).
    if slope <= 0.0 {
        return None;
    }
    // Clamp to a sane upper bound: a flat slope near 0 produces astronomically
    // large predictions that are meaningless to the user.
    const MAX_PREDICTED_TURNS: i64 = 10_000;
    let predicted = (tokens_remaining as f64 / slope).round() as i64;
    Some(predicted.clamp(0, MAX_PREDICTED_TURNS))
}

fn handle_get_context_pressure(db: &Database) -> Value {
    let sid = match db.get_latest_session_id() {
        Ok(Some(id)) => id,
        _ => return json!({"error": "No sessions found."}),
    };
    let session = match db.get_session(&sid) {
        Ok(Some(session)) => session,
        Ok(None) => return json!({"error": "Session metadata unavailable."}),
        Err(e) => return json!({"error": e.to_string()}),
    };
    let model = session.model.as_str();
    let stored_window = session.context_window_tokens;
    let last_input = match db.get_last_turn_for_session(&sid) {
        Ok(Some(turn)) => turn.input_tokens + turn.cache_read_tokens,
        Ok(None) => 0,
        Err(e) => return json!({"error": e.to_string()}),
    };
    let recent_turns = db
        .list_recent_turns_for_session(&sid, 10)
        .unwrap_or_default();

    // §8.2: prefer stored context window from JSONL if available.
    let window = stored_window.unwrap_or_else(|| scopeon_core::context_window_for_model(model));
    let (fill_pct, tokens_remaining) =
        scopeon_core::context_pressure_with_window(model, last_input, stored_window);

    // Predictive countdown: fit a linear trend to the last ≤10 turns'
    // total input size and extrapolate how many more turns remain before
    // the context window is exhausted.
    let predicted_turns_remaining = predict_turns_remaining(&recent_turns, tokens_remaining);

    json!({
        "session_id": sid,
        "model": model,
        "context_window_tokens": window,
        "last_turn_input_tokens": last_input,
        "fill_pct": fill_pct,
        "tokens_remaining": tokens_remaining,
        "should_compact": fill_pct >= 80.0,
        "pressure_level": if fill_pct >= 95.0 { "critical" } else if fill_pct >= 80.0 { "high" } else { "normal" },
        "predicted_turns_remaining": predicted_turns_remaining,
    })
}

fn handle_get_budget_status(db: &Database, config: &UserConfig) -> Value {
    use chrono::Datelike;
    let global = match db.get_global_stats() {
        Ok(g) => g,
        Err(e) => return json!({"error": e.to_string()}),
    };
    let today = chrono::Local::now().date_naive();
    let week_start = today - chrono::Duration::days(today.weekday().num_days_from_monday() as i64);
    let mut daily = 0.0f64;
    let mut weekly = 0.0f64;
    let mut monthly = 0.0f64;
    for r in &global.daily {
        if let Ok(d) = chrono::NaiveDate::parse_from_str(&r.date, "%Y-%m-%d") {
            if d == today {
                daily += r.estimated_cost_usd;
            }
            if d >= week_start {
                weekly += r.estimated_cost_usd;
            }
            if d.month() == today.month() && d.year() == today.year() {
                monthly += r.estimated_cost_usd;
            }
        }
    }
    // Use elapsed calendar days (not activity days) for projection to avoid
    // overstating costs for users who aren't active every day.
    let avg_daily = {
        let dates: Vec<chrono::NaiveDate> = global
            .daily
            .iter()
            .filter_map(|r| chrono::NaiveDate::parse_from_str(&r.date, "%Y-%m-%d").ok())
            .collect();
        if dates.is_empty() {
            0.0
        } else {
            let earliest = dates.iter().min().copied().unwrap_or(today);
            let calendar_days = (today - earliest).num_days() + 1; // inclusive
            global.estimated_cost_usd / calendar_days.max(1) as f64
        }
    };
    let projected_monthly = avg_daily * 30.0;
    json!({
        "daily":   { "spent": daily,   "limit": config.budget.daily_usd,   "pct": if config.budget.daily_usd > 0.0 { daily/config.budget.daily_usd*100.0 } else { 0.0 } },
        "weekly":  { "spent": weekly,  "limit": config.budget.weekly_usd,  "pct": if config.budget.weekly_usd > 0.0 { weekly/config.budget.weekly_usd*100.0 } else { 0.0 } },
        "monthly": { "spent": monthly, "limit": config.budget.monthly_usd, "pct": if config.budget.monthly_usd > 0.0 { monthly/config.budget.monthly_usd*100.0 } else { 0.0 } },
        "projected_monthly": projected_monthly,
        "over_budget":
            (config.budget.daily_usd > 0.0 && daily > config.budget.daily_usd)
            || (config.budget.weekly_usd > 0.0 && weekly > config.budget.weekly_usd)
            || (config.budget.monthly_usd > 0.0 && monthly > config.budget.monthly_usd),
    })
}

fn handle_get_provider_capabilities(db: &Database, session_id: Option<&str>) -> Value {
    let sid = match resolve_session_id(db, session_id) {
        Ok(id) => id,
        Err(err) => return err,
    };
    let session = match db.get_session(&sid) {
        Ok(Some(session)) => session,
        Ok(None) => return json!({"error": "Session metadata unavailable."}),
        Err(e) => return json!({"error": e.to_string()}),
    };
    let capabilities = provider_capabilities(&session.provider);
    let exact = capabilities.iter().filter(|c| c.level == "exact").count();
    let estimated = capabilities
        .iter()
        .filter(|c| c.level == "estimated")
        .count();
    let unsupported = capabilities
        .iter()
        .filter(|c| c.level == "unsupported")
        .count();

    json!({
        "session_id": sid,
        "provider": {
            "name": session.provider,
            "version": session.provider_version,
            "model": session.model,
        },
        "support_summary": {
            "exact": exact,
            "estimated": estimated,
            "unsupported": unsupported,
        },
        "capabilities": capabilities,
    })
}

fn handle_get_interaction_history(db: &Database, session_id: Option<&str>, limit: usize) -> Value {
    let sid = match resolve_session_id(db, session_id) {
        Ok(id) => id,
        Err(err) => return err,
    };
    let session = match db.get_session(&sid) {
        Ok(Some(session)) => session,
        Ok(None) => return json!({"error": "Session metadata unavailable."}),
        Err(e) => return json!({"error": e.to_string()}),
    };
    let capabilities = provider_capabilities(&session.provider);
    let events = match db.list_interaction_events_for_session(&sid, limit) {
        Ok(events) => events,
        Err(e) => return json!({"error": e.to_string()}),
    };
    let hook_effects = derive_hook_effects(&events);
    let hook_effect_summary = {
        let modified = hook_effects
            .values()
            .filter(|effect| effect.as_str() == "modified")
            .count();
        let blocked = hook_effects
            .values()
            .filter(|effect| effect.as_str() == "blocked")
            .count();
        let pass_through = hook_effects
            .values()
            .filter(|effect| effect.as_str() == "pass_through")
            .count();
        json!({
            "modified": modified,
            "blocked": blocked,
            "pass_through": pass_through,
            "observed": hook_effects.len().saturating_sub(modified + blocked + pass_through),
        })
    };
    let data: Vec<Value> = events
        .iter()
        .map(|event| {
            json!({
                "id": event.id,
                "timestamp": event.timestamp,
                "turn_id": event.turn_id,
                "task_run_id": event.task_run_id,
                "correlation_id": event.correlation_id,
                "parent_id": event.parent_id,
                "kind": event.kind,
                "phase": event.phase,
                "name": event.name,
                "display_name": event.display_name,
                "provider": event.provider,
                "status": event.status,
                "success": event.success,
                "hook_type": event.hook_type,
                "agent_type": event.agent_type,
                "execution_mode": event.execution_mode,
                "model": event.model,
                "mcp": {
                    "server": event.mcp_server,
                    "tool": event.mcp_tool,
                },
                "sizes": {
                    "input_chars": event.input_size_chars,
                    "output_chars": event.output_size_chars,
                    "prompt_chars": event.prompt_size_chars,
                    "summary_chars": event.summary_size_chars,
                },
                "tokens": {
                    "total": event.total_tokens,
                    "estimated_input": event.estimated_input_tokens,
                    "estimated_output": event.estimated_output_tokens,
                    "attributed_total": interaction_token_total(event),
                    "confidence": event.confidence,
                },
                "duration_ms": event.duration_ms,
                "tool_calls": event.total_tool_calls,
                "hook_effect": hook_effects.get(&event.id),
            })
        })
        .collect();

    json!({
        "session_id": sid,
        "provider": {
            "name": session.provider,
            "version": session.provider_version,
            "model": session.model,
        },
        "capabilities": capabilities,
        "hook_effect_summary": hook_effect_summary,
        "count": data.len(),
        "events": data,
    })
}

fn handle_get_task_history(db: &Database, session_id: Option<&str>, limit: usize) -> Value {
    let sid = match resolve_session_id(db, session_id) {
        Ok(id) => id,
        Err(err) => return err,
    };
    let session = match db.get_session(&sid) {
        Ok(Some(session)) => session,
        Ok(None) => return json!({"error": "Session metadata unavailable."}),
        Err(e) => return json!({"error": e.to_string()}),
    };
    let capabilities = provider_capabilities(&session.provider);
    let tasks = match db.list_recent_task_runs_for_session(&sid, limit) {
        Ok(tasks) => tasks,
        Err(e) => return json!({"error": e.to_string()}),
    };
    let events = db
        .list_interaction_events_for_session(&sid, 10_000)
        .unwrap_or_default();

    let task_data: Vec<Value> = tasks
        .iter()
        .map(|task| {
            let task_events: Vec<_> = events
                .iter()
                .filter(|event| event.task_run_id.as_deref() == Some(task.id.as_str()))
                .collect();
            let mut kinds = std::collections::BTreeMap::<String, usize>::new();
            let mut tools = std::collections::BTreeSet::<String>::new();
            let mut mcp_tools = std::collections::BTreeSet::<String>::new();

            for event in &task_events {
                *kinds.entry(event.kind.clone()).or_default() += 1;
                if matches!(event.kind.as_str(), "tool" | "task" | "skill") {
                    tools.insert(event.name.clone());
                }
                if event.kind == "mcp" {
                    let label = match (&event.mcp_server, &event.mcp_tool) {
                        (Some(server), Some(tool)) => format!("{server}.{tool}"),
                        _ => event.name.clone(),
                    };
                    mcp_tools.insert(label);
                }
            }

            json!({
                "id": task.id,
                "correlation_id": task.correlation_id,
                "name": task.name,
                "display_name": task.display_name,
                "agent_type": task.agent_type,
                "execution_mode": task.execution_mode,
                "requested_model": task.requested_model,
                "actual_model": task.actual_model,
                "started_at": task.started_at,
                "completed_at": task.completed_at,
                "duration_ms": task.duration_ms,
                "success": task.success,
                "tokens": {
                    "total": task.total_tokens,
                    "confidence": task.confidence,
                },
                "tool_calls": task.total_tool_calls,
                "payload_sizes": {
                    "description_chars": task.description_size_chars,
                    "prompt_chars": task.prompt_size_chars,
                    "summary_chars": task.summary_size_chars,
                },
                "interaction_summary": {
                    "count": task_events.len(),
                    "by_kind": kinds,
                    "tools_used": tools.into_iter().collect::<Vec<_>>(),
                    "mcp_tools_used": mcp_tools.into_iter().collect::<Vec<_>>(),
                },
            })
        })
        .collect();

    json!({
        "session_id": sid,
        "provider": {
            "name": session.provider,
            "version": session.provider_version,
            "model": session.model,
        },
        "capabilities": capabilities,
        "count": task_data.len(),
        "tasks": task_data,
    })
}

fn handle_get_optimization_suggestions(db: &Database) -> Value {
    let sid = match db.get_latest_session_id() {
        Ok(Some(id)) => id,
        _ => return json!({"error": "No sessions found."}),
    };
    let stats = match db.get_session_stats(&sid) {
        Ok(s) => s,
        Err(e) => return json!({"error": e.to_string()}),
    };
    let tool_calls = db.list_tool_calls_for_session(&sid).unwrap_or_default();
    let interaction_events = db
        .list_interaction_events_for_session(&sid, 10_000)
        .unwrap_or_default();
    let task_runs = db.list_task_runs_for_session(&sid).unwrap_or_default();
    let global = db.get_global_stats().ok();
    let provider_name = stats
        .session
        .as_ref()
        .map(|s| s.provider.as_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("unknown");
    let ctx = MetricContext {
        turns: &stats.turns,
        session: stats.session.as_ref(),
        daily_rollups: global.as_ref().map(|g| g.daily.as_slice()).unwrap_or(&[]),
        provider_name,
        tool_calls: &tool_calls,
        interaction_events: &interaction_events,
        task_runs: &task_runs,
    };
    let thresholds = db
        .get_threshold_data()
        .map(|d| scopeon_metrics::UserThresholds::from_daily_data(&d))
        .unwrap_or_default();
    let waste = WasteReport::compute_with_thresholds(&ctx, &thresholds);
    let suggestions = compute_suggestions(&ctx, &waste, global.as_ref());

    let waste_signals: Vec<Value> = waste
        .signals
        .iter()
        .map(|s| {
            let mut obj = serde_json::to_value(&s.kind).unwrap_or(Value::Null);
            if let Some(m) = obj.as_object_mut() {
                m.insert(
                    "severity".into(),
                    serde_json::to_value(&s.severity).unwrap_or(Value::Null),
                );
                m.insert("message".into(), Value::String(s.message.clone()));
            }
            obj
        })
        .collect();

    let suggestion_list: Vec<Value> = suggestions
        .iter()
        .map(|s| {
            json!({
                "id": s.id,
                "severity": serde_json::to_value(&s.severity).unwrap_or(Value::Null),
                "title": s.title,
                "body": s.body,
            })
        })
        .collect();

    let capabilities = provider_capabilities(provider_name);
    let hook_effects = derive_hook_effects(&interaction_events);
    let hook_effect_summary = json!({
        "modified": hook_effects.values().filter(|effect| effect.as_str() == "modified").count(),
        "blocked": hook_effects.values().filter(|effect| effect.as_str() == "blocked").count(),
        "pass_through": hook_effects.values().filter(|effect| effect.as_str() == "pass_through").count(),
    });

    json!({
        "session_id": sid,
        "provider": {
            "name": provider_name,
            "version": stats.session.as_ref().map(|s| s.provider_version.clone()),
            "model": stats.session.as_ref().map(|s| s.model.clone()),
            "capabilities": capabilities,
        },
        "provenance_summary": {
            "interaction_events": interaction_events.len(),
            "task_runs": task_runs.len(),
            "hook_effects": hook_effect_summary,
        },
        "waste_signals": waste_signals,
        "suggestions": suggestion_list,
        "waste_score": waste.waste_score,
    })
}

fn handle_suggest_compact(db: &Database) -> Value {
    let sid = match db.get_latest_session_id() {
        Ok(Some(id)) => id,
        _ => return json!({"error": "No sessions found."}),
    };
    let session = match db.get_session(&sid) {
        Ok(Some(session)) => session,
        Ok(None) => return json!({"error": "Session metadata unavailable."}),
        Err(e) => return json!({"error": e.to_string()}),
    };
    let model = session.model.as_str();
    let stored_window = session.context_window_tokens;
    let last_input = match db.get_last_turn_for_session(&sid) {
        Ok(Some(turn)) => turn.input_tokens + turn.cache_read_tokens,
        Ok(None) => 0,
        Err(e) => return json!({"error": e.to_string()}),
    };
    // §8.2: prefer stored context window from JSONL if available
    let (fill_pct, _) =
        scopeon_core::context_pressure_with_window(model, last_input, stored_window);

    // Count turns since last compaction event
    let turns_since_compact = db.count_turns_since_last_compaction(&sid).unwrap_or(0);
    let last_compact_was_recent = turns_since_compact < 10;

    let should_compact = fill_pct >= 80.0 && !last_compact_was_recent;
    let reason = if fill_pct >= 95.0 {
        "Context is critically full (≥95%) — compact immediately to avoid context loss"
    } else if fill_pct >= 80.0 && !last_compact_was_recent {
        "Context is high (≥80%) and no recent compaction — good time to compact"
    } else if fill_pct >= 80.0 {
        "Context is high but compaction was recent — monitor before compacting again"
    } else {
        "Context pressure is normal — no compaction needed"
    };

    json!({
        "should_compact": should_compact,
        "fill_pct": fill_pct,
        "turns_since_last_compaction": turns_since_compact,
        "reason": reason,
    })
}

fn handle_get_project_stats(db: &Database, top_n: Option<usize>) -> Value {
    match db.get_project_stats() {
        Ok(mut projects) => {
            if let Some(n) = top_n {
                projects.truncate(n);
            }
            let data: Vec<Value> = projects
                .iter()
                .map(|p| {
                    json!({
                        "project": p.project_name,
                        "branch": p.git_branch,
                        "sessions": p.session_count,
                        "turns": p.total_turns,
                        "cost_usd": p.total_cost_usd,
                        "cache_hit_rate_pct": p.avg_cache_hit_rate,
                        "compactions": p.compaction_count,
                    })
                })
                .collect();
            json!({"projects": data})
        },
        Err(e) => json!({"error": e.to_string()}),
    }
}

fn handle_list_sessions(db: &Database, limit: usize) -> Value {
    let sessions = db.list_sessions(limit).unwrap_or_default();
    let summaries: std::collections::HashMap<String, scopeon_core::SessionSummary> = db
        .list_session_summaries(limit)
        .unwrap_or_default()
        .into_iter()
        .map(|s| (s.session_id.clone(), s))
        .collect();

    let data: Vec<Value> = sessions
        .iter()
        .map(|s| {
            let cost = summaries
                .get(&s.id)
                .map(|sm| sm.estimated_cost_usd)
                .unwrap_or(0.0);
            let cache = summaries
                .get(&s.id)
                .map(|sm| sm.cache_hit_rate * 100.0)
                .unwrap_or(0.0);
            json!({
                "session_id": s.id,
                "project": s.project_name,
                "provider": s.provider.clone(),
                "provider_version": s.provider_version.clone(),
                "model": s.model.clone(),
                "branch": s.git_branch,
                "is_subagent": s.is_subagent,
                "turn_count": s.total_turns,
                "cost_usd": cost,
                "cache_hit_rate_pct": cache,
                "started_at": s.started_at,
                "last_turn_at": s.last_turn_at,
            })
        })
        .collect();
    json!({"sessions": data, "count": data.len()})
}

fn handle_get_agent_tree(db: &Database, session_id: Option<&str>) -> Value {
    let root_id = match session_id {
        Some(id) => id.to_string(),
        None => match db.get_latest_session_id() {
            Ok(Some(id)) => id,
            _ => return json!({"error": "No sessions found."}),
        },
    };
    match db.get_agent_tree(&root_id) {
        Ok(tree) => serde_json::to_value(&tree).unwrap_or_else(|e| json!({"error": e.to_string()})),
        Err(e) => json!({"error": e.to_string()}),
    }
}

// ── Main server loop ──────────────────────────────────────────────────────────

pub async fn run_mcp_server(db: Arc<Mutex<Database>>) -> Result<()> {
    let stdin = tokio::io::stdin();
    // §4.5: Single shared writer — all stdout paths (responses + push notifications)
    // lock this mutex to prevent interleaved JSON-RPC output lines.
    let stdout: SharedWriter =
        Arc::new(tokio::sync::Mutex::new(BufWriter::new(tokio::io::stdout())));
    let config = UserConfig::load();
    let snapshot_config = config.clone();
    // Bound individual stdin lines to 4 MiB *before* allocating, preventing OOM
    // from a sender that writes gigabytes without a newline. We use take() so the
    // kernel-level buffer is capped before read_line allocates its String.
    const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;
    let mut reader = BufReader::new(stdin);
    let mut line = String::with_capacity(4096);

    info!("Scopeon MCP server started (JSON-RPC 2.0 over stdio)");

    // ── Pre-Computation Engine (TRIZ Rank 1) ─────────────────────────────────
    // All MCP tool responses are pre-computed in a background task and stored in
    // MetricSnapshot. The dispatch() function reads from this snapshot — zero DB
    // queries at call time. The refresh interval adapts to context risk level.
    let snapshot: Arc<RwLock<MetricSnapshot>> = Arc::new(RwLock::new(MetricSnapshot::default()));
    let snapshot_writer = snapshot.clone();

    // ── Read-Connection Pool (complement WAL, TRIZ S3) ────────────────────────
    // With WAL mode already enabled on the database, multiple readers can coexist
    // with the writer at the SQLite level. We open a dedicated read-only connection
    // for the pre-computation task so it never contends with the watcher's write
    // connection. Falls back to the shared mutex if the path is unavailable
    // (in-memory DB used in tests, or first boot before path is known).
    let db_path_for_reader = db.lock().ok().and_then(|g| g.path().map(|p| p.to_owned()));
    let db_snap = db.clone();

    // ── Proactive Push Notification Channel ──────────────────────────────────
    let (alert_tx, mut alert_rx) = tokio::sync::mpsc::channel::<Alert>(32);

    tokio::spawn(async move {
        // 60-second cooldown per alert kind to prevent notification spam.
        let mut debounce = AlertDebounce::new(60);
        let mut first_refresh = true;

        // S-7: Track fill_pct history (last 5 samples) for acceleration detection.
        let mut fill_history: std::collections::VecDeque<f64> =
            std::collections::VecDeque::with_capacity(5);

        // S-3: Ambient status push every 30 s when not in crisis (no token cost).
        let mut last_ambient = std::time::Instant::now();
        const AMBIENT_INTERVAL_SECS: u64 = 30;

        // Open a dedicated read-only connection to avoid holding the write mutex
        // during the (potentially slow) full-snapshot computation.
        let read_db = db_path_for_reader
            .as_deref()
            .and_then(|p| scopeon_core::Database::open_readonly(p).ok());

        // Do an immediate first refresh so the snapshot is populated before the
        // first MCP call arrives.
        loop {
            if !first_refresh {
                let interval = {
                    let guard = snapshot_writer.read().unwrap_or_else(|p| p.into_inner());
                    adaptive_interval(&guard)
                };
                tokio::time::sleep(interval).await;
            }
            first_refresh = false;

            // Prefer the dedicated read connection; fall back to shared mutex.
            let new_snap = if let Some(rdb) = &read_db {
                // Zero-contention path: reuse one read-only handle instead of reopening
                // SQLite connections inside the refresh loop.
                refresh_snapshot(rdb, &snapshot_config)
            } else {
                match db_snap.lock() {
                    Ok(guard) => {
                        let s = refresh_snapshot(&guard, &snapshot_config);
                        drop(guard);
                        s
                    },
                    Err(_) => {
                        tracing::warn!(
                            "Pre-computation task: DB mutex poisoned — skipping refresh"
                        );
                        continue;
                    },
                }
            };

            // S-7: Maintain rolling fill_pct history for acceleration scoring.
            let fill_pct = new_snap
                .context_pressure
                .as_ref()
                .and_then(|v| v.get("fill_pct"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            if fill_history.len() == 5 {
                fill_history.pop_front();
            }
            fill_history.push_back(fill_pct);

            // Check alert conditions before updating the shared snapshot.
            check_alerts(&new_snap, &mut debounce, &alert_tx);

            // S-7: Compaction advisory — pre-crisis optimal compact window.
            check_compaction_advisory(&new_snap, &fill_history, &mut debounce, &alert_tx);

            if let Ok(mut w) = snapshot_writer.write() {
                *w = new_snap;
            }

            // S-3: Ambient status push — zero-token periodic notification at 30s.
            // Only fires outside the crisis band (≥ 80 % fill triggers proper alerts).
            if fill_pct < 80.0 && last_ambient.elapsed().as_secs() >= AMBIENT_INTERVAL_SECS {
                last_ambient = std::time::Instant::now();
                let guard = snapshot_writer.read().unwrap_or_else(|p| p.into_inner());
                let snap = &*guard;

                let daily_cost = snap
                    .budget_status
                    .as_ref()
                    .and_then(|v| v.get("daily_spent"))
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                let cache_hit_rate = snap
                    .cache_efficiency
                    .as_ref()
                    .and_then(|v| v.get("cache_hit_rate"))
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                let predicted_turns = snap
                    .context_pressure
                    .as_ref()
                    .and_then(|v| v.get("predicted_turns_remaining"))
                    .and_then(Value::as_i64);
                let should_compact = snap
                    .suggest_compact
                    .as_ref()
                    .and_then(|v| v.get("should_compact"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                // Lightweight health proxy (no full MetricContext needed here).
                let health_proxy = (cache_hit_rate * 35.0
                    + (100.0 - fill_pct).max(0.0) * 0.35
                    + (100.0
                        - snap
                            .optimization_suggestions
                            .as_ref()
                            .and_then(|v| v.get("waste_score"))
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0))
                        * 0.30)
                    .clamp(0.0, 100.0);

                let _ = alert_tx.try_send(Alert {
                    payload: json!({
                        "method": "notifications/scopeon/status",
                        "params": {
                            "type": "ambient_status",
                            "fill_pct": fill_pct,
                            "predicted_turns_remaining": predicted_turns,
                            "daily_cost_usd": daily_cost,
                            "cache_hit_rate_pct": cache_hit_rate,
                            "should_compact": should_compact,
                            "health_score_proxy": (health_proxy * 10.0).round() / 10.0,
                        }
                    }),
                });
            }
        }
    });

    loop {
        line.clear();
        // take(MAX+1) reads at most MAX_LINE_BYTES+1 bytes or until '\n'.
        // If the line is longer, read_line stops at the byte limit and the
        // string will not end with '\n' — we detect and drain the remainder.
        let bytes_read = tokio::select! {
            // Priority 1: outgoing push notifications (non-blocking send)
            alert = alert_rx.recv() => {
                if let Some(alert) = alert {
                    // JSON-RPC notification: no "id" field (§4 of spec)
                    let notif = match serde_json::to_string(&alert.payload) {
                        Ok(notif) => notif,
                        Err(e) => {
                            tracing::warn!("Failed to serialize push notification: {}", e);
                            continue;
                        },
                    };
                    debug!("→ [push] {}", notif);
                    {
                        let mut w = stdout.lock().await;
                        w.write_all(notif.as_bytes()).await?;
                        w.write_all(b"\n").await?;
                        w.flush().await?;
                    }

                    // Fire configured webhooks (non-blocking; failures are logged only)
                    if !config.alerts.webhooks.is_empty() {
                        let alert_type = alert.payload
                            .pointer("/params/type")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown");
                        fire_webhooks(&config, alert_type, &alert.payload).await;
                    }
                }
                continue;
            }
            // Priority 2: incoming JSON-RPC requests from the MCP client
            result = async {
                let mut limited = (&mut reader).take(MAX_LINE_BYTES as u64 + 1);
                limited.read_line(&mut line).await
            } => result?,
        };

        if bytes_read == 0 {
            break; // EOF
        }

        if line.len() > MAX_LINE_BYTES {
            // Drain the remainder of this oversized line so the next read
            // starts at a clean message boundary.
            let mut discard = Vec::new();
            let _ = reader.read_until(b'\n', &mut discard).await;
            let resp = JsonRpcResponse::err(
                Value::Null,
                -32600,
                format!("Request too large (max {} bytes)", MAX_LINE_BYTES),
            );
            send_response(&stdout, &resp).await?;
            continue;
        }

        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        debug!("← {}", line);

        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = JsonRpcResponse::err(Value::Null, -32700, format!("Parse error: {}", e));
                send_response(&stdout, &resp).await?;
                continue;
            },
        };

        let id = request.id.clone().unwrap_or(Value::Null);
        let response = dispatch(id, request, &db, &snapshot, &config);
        send_response(&stdout, &response).await?;
    }

    Ok(())
}

fn dispatch(
    id: Value,
    request: JsonRpcRequest,
    db: &Arc<Mutex<Database>>,
    snapshot: &Arc<RwLock<MetricSnapshot>>,
    config: &UserConfig,
) -> JsonRpcResponse {
    // Snapshot read guard — used for zero-cost tool calls.
    // If the RwLock is somehow poisoned (extremely unlikely), fall back to live queries.
    let snap = snapshot.read().ok().map(|g| g.clone());

    match request.method.as_str() {
        "initialize" => JsonRpcResponse::ok(
            id,
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "scopeon", "version": env!("CARGO_PKG_VERSION") }
            }),
        ),
        "tools/list" => JsonRpcResponse::ok(id, tool_list()),
        "tools/call" => {
            let params = request.params.unwrap_or_default();
            let tool_name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or_default();

            let result = match tool_name {
                // ── Snapshot-served tools (zero DB query when cache is warm) ──────
                "get_token_usage" => snap_or_live(
                    snap.as_ref().and_then(|s| s.token_usage.clone()),
                    db,
                    |db| live_query(db, handle_get_token_usage),
                ),
                "get_context_pressure" => snap_or_live(
                    snap.as_ref().and_then(|s| s.context_pressure.clone()),
                    db,
                    |db| live_query(db, handle_get_context_pressure),
                ),
                "get_budget_status" => snap_or_live(
                    snap.as_ref().and_then(|s| s.budget_status.clone()),
                    db,
                    |db| live_query(db, |d| handle_get_budget_status(d, config)),
                ),
                "get_optimization_suggestions" => snap_or_live(
                    snap.as_ref()
                        .and_then(|s| s.optimization_suggestions.clone()),
                    db,
                    |db| live_query(db, handle_get_optimization_suggestions),
                ),
                "get_provider_capabilities" => {
                    let sid = args.get("session_id").and_then(Value::as_str);
                    if sid.is_none() {
                        snap_or_live(
                            snap.as_ref().and_then(|s| s.provider_capabilities.clone()),
                            db,
                            |db| live_query(db, |d| handle_get_provider_capabilities(d, None)),
                        )
                    } else {
                        live_query(db, |d| handle_get_provider_capabilities(d, sid))
                    }
                },
                "get_interaction_history" => {
                    let sid = args.get("session_id").and_then(Value::as_str);
                    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(100) as usize;
                    if sid.is_none() && limit == 100 {
                        snap_or_live(
                            snap.as_ref().and_then(|s| s.interaction_history.clone()),
                            db,
                            |db| live_query(db, |d| handle_get_interaction_history(d, None, 100)),
                        )
                    } else {
                        live_query(db, |d| handle_get_interaction_history(d, sid, limit))
                    }
                },
                "get_task_history" => {
                    let sid = args.get("session_id").and_then(Value::as_str);
                    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(25) as usize;
                    if sid.is_none() && limit == 25 {
                        snap_or_live(
                            snap.as_ref().and_then(|s| s.task_history.clone()),
                            db,
                            |db| live_query(db, |d| handle_get_task_history(d, None, 25)),
                        )
                    } else {
                        live_query(db, |d| handle_get_task_history(d, sid, limit))
                    }
                },
                "suggest_compact" => snap_or_live(
                    snap.as_ref().and_then(|s| s.suggest_compact.clone()),
                    db,
                    |db| live_query(db, handle_suggest_compact),
                ),

                // ── Conditionally snapshot-served (only when no specific args) ────
                "get_session_summary" => {
                    let sid = args.get("session_id").and_then(Value::as_str);
                    if sid.is_none() {
                        snap_or_live(
                            snap.as_ref().and_then(|s| s.session_summary.clone()),
                            db,
                            |db| live_query(db, |d| handle_get_session_summary(d, None)),
                        )
                    } else {
                        live_query(db, |d| handle_get_session_summary(d, sid))
                    }
                },
                "get_cache_efficiency" => {
                    let sid = args.get("session_id").and_then(Value::as_str);
                    if sid.is_none() {
                        snap_or_live(
                            snap.as_ref().and_then(|s| s.cache_efficiency.clone()),
                            db,
                            |db| live_query(db, |d| handle_get_cache_efficiency(d, None)),
                        )
                    } else {
                        live_query(db, |d| handle_get_cache_efficiency(d, sid))
                    }
                },
                "get_history" => {
                    let days = args.get("days").and_then(Value::as_i64).unwrap_or(30);
                    if days == 30 {
                        snap_or_live(
                            snap.as_ref().and_then(|s| s.history_30d.clone()),
                            db,
                            |db| live_query(db, |d| handle_get_history(d, 30)),
                        )
                    } else {
                        live_query(db, |d| handle_get_history(d, days))
                    }
                },
                "get_project_stats" => {
                    let top_n = args
                        .get("top_n")
                        .and_then(Value::as_u64)
                        .map(|n| n as usize);
                    if top_n.is_none() {
                        snap_or_live(
                            snap.as_ref().and_then(|s| s.project_stats.clone()),
                            db,
                            |db| live_query(db, |d| handle_get_project_stats(d, None)),
                        )
                    } else {
                        live_query(db, |d| handle_get_project_stats(d, top_n))
                    }
                },
                "list_sessions" => {
                    let limit = args
                        .get("limit")
                        .and_then(Value::as_u64)
                        .map(|n| n as usize);
                    if limit.is_none() || limit == Some(20) {
                        snap_or_live(
                            snap.as_ref().and_then(|s| s.sessions_list.clone()),
                            db,
                            |db| live_query(db, |d| handle_list_sessions(d, 20)),
                        )
                    } else {
                        live_query(db, |d| handle_list_sessions(d, limit.unwrap_or(20)))
                    }
                },
                "get_agent_tree" => {
                    let sid = args.get("session_id").and_then(Value::as_str);
                    if sid.is_none() {
                        snap_or_live(snap.as_ref().and_then(|s| s.agent_tree.clone()), db, |db| {
                            live_query(db, |d| handle_get_agent_tree(d, None))
                        })
                    } else {
                        live_query(db, |d| handle_get_agent_tree(d, sid))
                    }
                },

                // ── Always live (requires two specific IDs, not cacheable) ─────────
                "compare_sessions" => {
                    let id_a = args
                        .get("session_id_a")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let id_b = args
                        .get("session_id_b")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    live_query(db, |d| handle_compare_sessions(d, id_a, id_b))
                },

                // ── Tag operations (always live — mutations) ─────────────────────
                "set_session_tags" => {
                    let sid = args.get("session_id").and_then(Value::as_str);
                    let tags: Vec<String> = args
                        .get("tags")
                        .and_then(Value::as_array)
                        .map(|arr| {
                            arr.iter()
                                .filter_map(Value::as_str)
                                .map(String::from)
                                .collect()
                        })
                        .unwrap_or_default();
                    live_query(db, |d| {
                        let sid_resolved = match sid {
                            Some(id) => id.to_string(),
                            None => match d.get_latest_session_id() {
                                Ok(Some(id)) => id,
                                _ => return json!({"error": "No sessions found."}),
                            },
                        };
                        let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();
                        match d.set_session_tags(&sid_resolved, &tag_refs) {
                            Ok(()) => json!({"session_id": sid_resolved, "tags": tags, "ok": true}),
                            Err(e) => json!({"error": e.to_string()}),
                        }
                    })
                },
                "get_cost_by_tag" => live_query(db, |d| match d.get_cost_by_tag() {
                    Ok(rows) => {
                        let items: Vec<Value> = rows
                                .into_iter()
                                .map(|(tag, cost, count)| {
                                    json!({"tag": tag, "cost_usd": cost, "session_count": count})
                                })
                                .collect();
                        json!({"tags": items})
                    },
                    Err(e) => json!({"error": e.to_string()}),
                }),

                other => return JsonRpcResponse::err(id, -32601, format!("Unknown tool: {other}")),
            };

            let rendered_result = match serde_json::to_string_pretty(&result) {
                Ok(text) => text,
                Err(e) => {
                    json!({"error": format!("Failed to serialize tool result: {}", e)}).to_string()
                },
            };
            JsonRpcResponse::ok(
                id,
                json!({
                    "content": [{ "type": "text", "text": rendered_result }]
                }),
            )
        },
        "notifications/initialized" | "ping" => JsonRpcResponse::ok(id, json!({})),
        other => JsonRpcResponse::err(id, -32601, format!("Method not found: {other}")),
    }
}

async fn send_response(stdout: &SharedWriter, resp: &JsonRpcResponse) -> Result<()> {
    let json = serde_json::to_string(resp)?;
    debug!("→ {}", json);
    let mut w = stdout.lock().await;
    w.write_all(json.as_bytes()).await?;
    w.write_all(b"\n").await?;
    w.flush().await?;
    Ok(())
}
