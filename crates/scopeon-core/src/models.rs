//! Core data models for Scopeon.
//!
//! All structs derive [`serde::Serialize`] and [`serde::Deserialize`] so they
//! can be used in both the SQLite layer and the JSON export / MCP responses.

use serde::{Deserialize, Serialize};

/// FNV-1a 64-bit hash — deterministic, zero-dependency, suitable for short strings.
/// Used to fingerprint tool call inputs for redundancy detection.
pub fn fnv1a_64(data: &str) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x00000100000001b3;
    data.bytes()
        .fold(OFFSET, |h, b| (h ^ b as u64).wrapping_mul(PRIME))
}

/// A Claude Code session, corresponding to one `.jsonl` file under
/// `~/.claude/projects/<project-hash>/`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub project: String,
    pub project_name: String,
    pub slug: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub provider_version: String,
    pub model: String,
    pub git_branch: String,
    pub started_at: i64,
    pub last_turn_at: i64,
    pub total_turns: i64,
    pub is_subagent: bool,
    pub parent_session_id: Option<String>,
    /// §8.2: Actual context window size reported by the API (max_tokens from request).
    /// When `Some`, used in place of the model-lookup table value in `context_pressure`.
    /// `None` means unknown — fall back to the built-in table.
    #[serde(default)]
    pub context_window_tokens: Option<i64>,
}

/// A single assistant turn with full token breakdown
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    pub id: String,
    pub session_id: String,
    pub turn_index: i64,
    pub timestamp: i64,
    pub duration_ms: Option<i64>,
    // Raw usage from Claude API
    pub input_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub cache_write_5m_tokens: i64,
    pub cache_write_1h_tokens: i64,
    pub output_tokens: i64,
    // Computed breakdown
    pub thinking_tokens: i64,
    pub mcp_call_count: i64,
    pub mcp_input_token_est: i64,
    pub text_output_tokens: i64,
    pub model: String,
    pub service_tier: String,
    // Computed cost
    pub estimated_cost_usd: f64,
    #[serde(default)]
    pub is_compaction_event: bool,
}

/// A tool call within a turn
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub turn_id: String,
    pub session_id: String,
    pub tool_name: String,
    pub input_size_chars: i64,
    /// FNV-1a 64-bit hash of the serialized input JSON. Used for accurate redundancy
    /// detection in waste analysis. `0` means "unknown" (rows before M0006 migration).
    pub input_hash: u64,
    pub timestamp: i64,
}

/// A normalized interaction event emitted by a provider log.
///
/// Interaction events capture higher-level provenance that cannot be represented by
/// turn totals alone: skills, hooks, MCP/tool start/end, tasks, subagents, plan changes,
/// notifications, and similar lifecycle events.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InteractionEvent {
    pub id: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub task_run_id: Option<String>,
    pub correlation_id: Option<String>,
    pub parent_id: Option<String>,
    pub provider: String,
    pub timestamp: i64,
    pub kind: String,
    pub phase: String,
    pub name: String,
    pub display_name: Option<String>,
    pub mcp_server: Option<String>,
    pub mcp_tool: Option<String>,
    pub hook_type: Option<String>,
    pub agent_type: Option<String>,
    pub execution_mode: Option<String>,
    pub model: Option<String>,
    pub status: Option<String>,
    pub success: Option<bool>,
    pub input_size_chars: i64,
    pub output_size_chars: i64,
    pub prompt_size_chars: i64,
    pub summary_size_chars: i64,
    pub total_tokens: Option<i64>,
    pub total_tool_calls: Option<i64>,
    pub duration_ms: Option<i64>,
    pub estimated_input_tokens: i64,
    pub estimated_output_tokens: i64,
    pub estimated_cost_usd: f64,
    pub confidence: String,
}

/// A task/subagent run derived from tool lifecycle and session completion events.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskRun {
    pub id: String,
    pub session_id: String,
    pub correlation_id: Option<String>,
    pub name: String,
    pub display_name: Option<String>,
    pub agent_type: String,
    pub execution_mode: String,
    pub requested_model: Option<String>,
    pub actual_model: Option<String>,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub duration_ms: Option<i64>,
    pub success: Option<bool>,
    pub total_tokens: Option<i64>,
    pub total_tool_calls: Option<i64>,
    pub description_size_chars: i64,
    pub prompt_size_chars: i64,
    pub summary_size_chars: i64,
    pub confidence: String,
}

/// Describes what level of provenance is actually available for a provider.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderCapability {
    pub provider: String,
    pub capability: String,
    pub level: String,
    pub note: String,
}

/// Lightweight per-session cost+cache summary for the sessions list view.
/// Computed by a single GROUP BY query, so it's fast even for hundreds of sessions.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionSummary {
    pub session_id: String,
    pub estimated_cost_usd: f64,
    pub cache_hit_rate: f64,
}

/// Pre-aggregated daily stats
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyRollup {
    pub date: String,
    pub total_input_tokens: i64,
    pub total_cache_read_tokens: i64,
    pub total_cache_write_tokens: i64,
    pub total_output_tokens: i64,
    pub total_thinking_tokens: i64,
    pub total_mcp_calls: i64,
    pub session_count: i64,
    pub turn_count: i64,
    pub estimated_cost_usd: f64,
    /// Optional daily health-rollup value (0.0–100.0).
    /// Current read paths compute health live and must not mutate this field.
    pub health_score_avg: f64,
}

/// Aggregated stats for a full session
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionStats {
    pub session: Option<Session>,
    pub total_input_tokens: i64,
    pub total_cache_read_tokens: i64,
    pub total_cache_write_tokens: i64,
    pub total_output_tokens: i64,
    pub total_thinking_tokens: i64,
    pub total_mcp_calls: i64,
    pub total_turns: i64,
    pub estimated_cost_usd: f64,
    pub cache_savings_usd: f64,
    pub cache_hit_rate: f64,
    pub turns: Vec<Turn>,
}

/// Summary across all sessions or a date range
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GlobalStats {
    pub total_sessions: i64,
    pub total_turns: i64,
    pub total_input_tokens: i64,
    pub total_cache_read_tokens: i64,
    pub total_cache_write_tokens: i64,
    pub total_output_tokens: i64,
    pub total_thinking_tokens: i64,
    pub total_mcp_calls: i64,
    pub estimated_cost_usd: f64,
    pub cache_savings_usd: f64,
    pub cache_hit_rate: f64,
    pub daily: Vec<DailyRollup>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectStats {
    pub project_name: String,
    pub git_branch: String,
    pub session_count: i64,
    pub total_cost_usd: f64,
    pub avg_cache_hit_rate: f64,
    pub total_turns: i64,
    pub compaction_count: i64,
    pub most_recent_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolStat {
    pub tool_name: String,
    pub call_count: i64,
    pub avg_input_chars: f64,
    pub est_tokens: i64,
    pub est_cost_usd: f64,
    /// Pricing model used for cost estimation (e.g., "sonnet-class-estimate").
    /// Callers should surface this label so users know the figure is approximate.
    pub cost_model: String,
}

/// Anomaly classification for a session, computed by comparing against the median.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionAnomaly {
    pub session_id: String,
    pub is_expensive: bool,
    pub is_tool_heavy: bool,
    pub is_cache_cold: bool,
    pub had_compaction: bool,
}

/// One row of the per-session tool/MCP/skill breakdown.
/// Aggregated from `interaction_events` grouped by kind + server + name.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolBreakdownItem {
    /// Event kind: "mcp" | "tool" | "skill" | "hook" | "subagent" | "compaction" | other
    pub kind: String,
    /// MCP server name for kind="mcp", empty string otherwise.
    pub server: String,
    /// MCP tool name (kind="mcp"), tool/skill/hook name (other kinds).
    pub name: String,
    pub count: i64,
}

/// A node in the multi-agent subagent tree. Each node represents one session;
/// children are sessions spawned as subagents from this session.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentNode {
    pub session_id: String,
    pub project_name: String,
    pub model: String,
    pub turn_count: i64,
    pub total_cost_usd: f64,
    pub total_tokens: i64,
    pub is_subagent: bool,
    pub started_at: i64,
    pub children: Vec<AgentNode>,
}
