//! Codex (OpenAI CLI) provider.
//!
//! Reads session data from `~/.codex/sessions/YYYY/MM/DD/*.jsonl` files written
//! by the Codex CLI. Each file is one session containing a stream of typed events
//! (`session_meta`, `turn_context`, `event_msg`, `response_item`).
//!
//! Token data is extracted from `event_msg/token_count` events.  The *last*
//! `token_count` before each `event_msg/task_complete` is used so that
//! streaming increments do not produce duplicate or partial turns.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use serde_json::Value;
use tracing::warn;

use super::Provider;
use scopeon_core::{
    branch_to_tag, cost::calculate_turn_cost, fnv1a_64, Database, Session, ToolCall, Turn,
    COMPACTION_MIN_PREV_TOKENS,
};

pub struct CodexProvider {
    sessions_dir: PathBuf,
}

impl CodexProvider {
    pub fn new() -> Self {
        // Honour CODEX_CONFIG_DIR env var, then fall back to ~/.codex
        let dir = std::env::var("CODEX_CONFIG_DIR")
            .ok()
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|h| h.join(".codex")))
            .unwrap_or_else(|| PathBuf::from("/nonexistent"))
            .join("sessions");
        CodexProvider { sessions_dir: dir }
    }
}

impl Default for CodexProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for CodexProvider {
    fn id(&self) -> &str {
        "codex"
    }

    fn name(&self) -> &str {
        "Codex"
    }

    fn description(&self) -> &str {
        "OpenAI Codex CLI. Reads JSONL sessions from $CODEX_CONFIG_DIR/sessions/ \
         (or ~/.codex/sessions/). Captures turns with full token breakdown and tool calls."
    }

    fn is_available(&self) -> bool {
        self.sessions_dir.exists()
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        vec![self.sessions_dir.clone()]
    }

    fn scan(&self, db: &Database) -> Result<usize> {
        if !self.is_available() {
            return Ok(0);
        }
        let mut total = 0;
        for file in walk_jsonl(&self.sessions_dir) {
            total += self.scan_file(&file, db).unwrap_or(0);
        }
        Ok(total)
    }

    /// Override to release the DB mutex between files so the TUI can refresh.
    fn scan_incremental(&self, db: Arc<Mutex<Database>>) -> Result<usize> {
        if !self.is_available() {
            return Ok(0);
        }
        let mut total = 0;
        for file in walk_jsonl(&self.sessions_dir) {
            {
                let db_guard = db
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Database mutex poisoned"))?;
                total += self.scan_file(&file, &db_guard).unwrap_or(0);
            } // mutex released here — TUI can refresh before next file
        }
        Ok(total)
    }
}

impl CodexProvider {
    fn scan_file(&self, path: &Path, db: &Database) -> Result<usize> {
        let path_str = path.to_string_lossy().to_string();

        let metadata = std::fs::metadata(path)?;
        let file_size = metadata.len();
        let current_mtime = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0);

        let (_, stored_mtime) = db.get_file_offset_and_mtime(&path_str)?;

        // Skip files that have not changed since the last scan.
        if stored_mtime != 0 && current_mtime == stored_mtime {
            return Ok(0);
        }

        let result = parse_codex_file(path)?;

        if result.session.is_none() && result.turns.is_empty() {
            // Record the mtime so we don't re-scan an empty/unrecognised file.
            db.commit_parse_result(&path_str, None, &[], &[], &[], file_size, current_mtime)?;
            return Ok(0);
        }

        db.commit_parse_result(
            &path_str,
            result.session.as_ref(),
            &result.turns,
            &result.tool_calls,
            &result.interaction_events,
            file_size,
            current_mtime,
        )?;

        // Compaction detection: >50 % input-token drop from a turn with >50 k tokens.
        for i in 1..result.turns.len() {
            let prev = result.turns[i - 1].input_tokens;
            let cur = result.turns[i].input_tokens;
            if prev > COMPACTION_MIN_PREV_TOKENS && cur < (prev as f64 * 0.5) as i64 {
                let _ = db.mark_session_had_compaction(&result.turns[i].session_id);
            }
        }

        // Authoritative total_turns from DB count (catches out-of-order rescan edge cases).
        if let Some(ref session) = result.session {
            if let Ok(count) = db.count_turns_for_session(&session.id) {
                if count > 0 && count != session.total_turns {
                    let mut updated = session.clone();
                    updated.total_turns = count;
                    let _ = db.upsert_session(&updated);
                }
            }
            // Auto-tag from git branch (only when no manual tag has been set).
            if db
                .get_session_tags(&session.id)
                .map(|t| t.is_empty())
                .unwrap_or(true)
            {
                if let Some(tag) = branch_to_tag(&session.git_branch) {
                    let _ = db.set_session_tags(&session.id, &[tag]);
                }
            }
        }

        // Refresh daily rollup for all timestamps touched by this batch.
        if !result.turns.is_empty() {
            let timestamps: Vec<i64> = result.turns.iter().map(|t| t.timestamp).collect();
            let _ = db.refresh_daily_rollup_for_timestamps(&timestamps);
        }

        Ok(result.turns.len())
    }
}

// ── Internal parse types ──────────────────────────────────────────────────────

struct ParseResult {
    session: Option<Session>,
    turns: Vec<Turn>,
    tool_calls: Vec<ToolCall>,
    interaction_events: Vec<scopeon_core::InteractionEvent>,
}

/// Per-turn token snapshot (from the last non-null `token_count` event).
#[derive(Default, Clone, Copy)]
struct TokenSnapshot {
    input_tokens: i64,
    cached_input_tokens: i64,
    output_tokens: i64,
    reasoning_tokens: i64,
}

// ── Parser ────────────────────────────────────────────────────────────────────

/// Parse a single Codex JSONL session file from the beginning.
///
/// The entire file is always read so that `session_meta` (always at line 0)
/// is available regardless of how much of the file is new.
fn parse_codex_file(path: &Path) -> Result<ParseResult> {
    let file = std::fs::File::open(path)?;
    let reader = BufReader::new(file);

    let mut session: Option<Session> = None;
    let mut turns: Vec<Turn> = Vec::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    // State accumulated while processing one turn's event sequence.
    let mut current_turn_id: Option<String> = None;
    let mut turn_models: HashMap<String, String> = HashMap::new();
    let mut turn_started_at: HashMap<String, i64> = HashMap::new();
    let mut last_token: HashMap<String, TokenSnapshot> = HashMap::new();
    let mut pending_tool_calls: HashMap<String, Vec<ToolCall>> = HashMap::new();
    let mut session_id_str = String::new();
    let mut context_window: Option<i64> = None;
    let mut turn_index: i64 = 0;

    for line in reader.lines().take(500_000) {
        let Ok(line) = line else {
            continue;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let Ok(val) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };

        let event_type = val.get("type").and_then(Value::as_str).unwrap_or("");
        let ts_ms = val
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_iso_ms)
            .unwrap_or(0);

        match event_type {
            "session_meta" => {
                let Some(payload) = val.get("payload") else {
                    continue;
                };
                let id = payload.get("id").and_then(Value::as_str).unwrap_or("");
                if id.is_empty() {
                    continue;
                }

                let cwd = payload
                    .get("cwd")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let project_name = Path::new(&cwd)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                let cli_version = payload
                    .get("cli_version")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let git_branch = payload
                    .get("git")
                    .and_then(|g| g.get("branch"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let started_at = payload
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .and_then(parse_iso_ms)
                    .unwrap_or(ts_ms);

                session_id_str = format!("codex-{}", id);
                session = Some(Session {
                    id: session_id_str.clone(),
                    project: cwd,
                    project_name,
                    slug: String::new(),
                    provider: "codex".to_string(),
                    provider_version: cli_version,
                    model: String::new(),
                    git_branch,
                    started_at,
                    last_turn_at: started_at,
                    total_turns: 0,
                    is_subagent: false,
                    parent_session_id: None,
                    context_window_tokens: None,
                });
            },

            "turn_context" => {
                let Some(payload) = val.get("payload") else {
                    continue;
                };
                let turn_id = payload.get("turn_id").and_then(Value::as_str).unwrap_or("");
                if turn_id.is_empty() {
                    continue;
                }
                if let Some(model) = payload.get("model").and_then(Value::as_str) {
                    if !model.is_empty() {
                        turn_models.insert(turn_id.to_string(), model.to_string());
                    }
                }
            },

            "event_msg" => {
                let Some(payload) = val.get("payload") else {
                    continue;
                };
                let payload_type = payload.get("type").and_then(Value::as_str).unwrap_or("");

                match payload_type {
                    "task_started" => {
                        let turn_id = payload.get("turn_id").and_then(Value::as_str).unwrap_or("");
                        if turn_id.is_empty() {
                            continue;
                        }
                        current_turn_id = Some(turn_id.to_string());
                        let started_sec = payload
                            .get("started_at")
                            .and_then(Value::as_i64)
                            .unwrap_or(0);
                        turn_started_at.insert(turn_id.to_string(), started_sec * 1000);
                        if let Some(cw) =
                            payload.get("model_context_window").and_then(Value::as_i64)
                        {
                            if cw > 0 {
                                context_window = Some(cw);
                            }
                        }
                    },

                    "token_count" => {
                        // `info` may be null during the first streaming event — keep last non-null.
                        let Some(info) = payload.get("info").filter(|v| !v.is_null()) else {
                            continue;
                        };
                        let Some(usage) = info.get("last_token_usage") else {
                            continue;
                        };
                        let snap = TokenSnapshot {
                            input_tokens: usage
                                .get("input_tokens")
                                .and_then(Value::as_i64)
                                .unwrap_or(0),
                            cached_input_tokens: usage
                                .get("cached_input_tokens")
                                .and_then(Value::as_i64)
                                .unwrap_or(0),
                            output_tokens: usage
                                .get("output_tokens")
                                .and_then(Value::as_i64)
                                .unwrap_or(0),
                            reasoning_tokens: usage
                                .get("reasoning_output_tokens")
                                .and_then(Value::as_i64)
                                .unwrap_or(0),
                        };
                        if let Some(ref tid) = current_turn_id {
                            last_token.insert(tid.clone(), snap);
                        }
                    },

                    "task_complete" => {
                        let turn_id = payload.get("turn_id").and_then(Value::as_str).unwrap_or("");
                        if turn_id.is_empty() || session_id_str.is_empty() {
                            continue;
                        }

                        let completed_sec = payload
                            .get("completed_at")
                            .and_then(Value::as_i64)
                            .unwrap_or(0);
                        let completed_ms = completed_sec * 1000;
                        let duration_ms = payload.get("duration_ms").and_then(Value::as_i64);

                        let snap = last_token.get(turn_id).copied().unwrap_or_default();
                        let model = turn_models.get(turn_id).cloned().unwrap_or_default();
                        let cost = calculate_turn_cost(
                            &model,
                            snap.input_tokens,
                            snap.output_tokens,
                            0, // Codex / OpenAI has no explicit cache-write charge
                            snap.cached_input_tokens,
                        );

                        let turn_unique_id = format!("{}-{}", session_id_str, turn_id);
                        let tool_count = pending_tool_calls
                            .get(turn_id)
                            .map(|v| v.len() as i64)
                            .unwrap_or(0);

                        let turn = Turn {
                            id: turn_unique_id.clone(),
                            session_id: session_id_str.clone(),
                            turn_index,
                            timestamp: if completed_ms > 0 {
                                completed_ms
                            } else {
                                ts_ms
                            },
                            duration_ms,
                            input_tokens: snap.input_tokens,
                            cache_read_tokens: snap.cached_input_tokens,
                            cache_write_tokens: 0,
                            cache_write_5m_tokens: 0,
                            cache_write_1h_tokens: 0,
                            output_tokens: snap.output_tokens,
                            thinking_tokens: snap.reasoning_tokens,
                            mcp_call_count: 0,
                            mcp_input_token_est: 0,
                            text_output_tokens: snap
                                .output_tokens
                                .saturating_sub(snap.reasoning_tokens),
                            model: model.clone(),
                            service_tier: String::new(),
                            estimated_cost_usd: cost.total_usd,
                            is_compaction_event: false,
                        };

                        if let Some(ref mut s) = session {
                            if turn.timestamp > s.last_turn_at {
                                s.last_turn_at = turn.timestamp;
                            }
                            if !model.is_empty() {
                                s.model = model;
                            }
                        }

                        if let Some(tc_list) = pending_tool_calls.remove(turn_id) {
                            // Fix turn_id on each tool call now that we have the
                            // stable unique turn id.
                            for mut tc in tc_list {
                                tc.turn_id = turn_unique_id.clone();
                                tool_calls.push(tc);
                            }
                        }

                        let _ = tool_count; // used above; silence warning
                        turns.push(turn);
                        turn_index += 1;
                        current_turn_id = None;
                    },

                    _ => {},
                }
            },

            "response_item" => {
                let Some(payload) = val.get("payload") else {
                    continue;
                };
                if payload.get("type").and_then(Value::as_str) != Some("function_call") {
                    continue;
                }
                let Some(ref tid) = current_turn_id.clone() else {
                    continue;
                };

                let call_id = payload.get("call_id").and_then(Value::as_str).unwrap_or("");
                let tool_name = payload
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let arguments = payload
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let input_chars = arguments.len() as i64;
                let input_hash = fnv1a_64(arguments);

                // turn_id is filled in correctly when task_complete fires.
                let tc = ToolCall {
                    id: format!("{}-{}-{}", session_id_str, tid, call_id),
                    turn_id: String::new(), // placeholder; fixed on task_complete
                    session_id: session_id_str.clone(),
                    tool_name: tool_name.to_string(),
                    input_size_chars: input_chars,
                    input_hash,
                    timestamp: ts_ms,
                };
                pending_tool_calls.entry(tid.clone()).or_default().push(tc);
            },

            _ => {},
        }
    }

    if let Some(ref mut s) = session {
        s.total_turns = turn_index;
        s.context_window_tokens = context_window;
    }

    Ok(ParseResult {
        session,
        turns,
        tool_calls,
        interaction_events: vec![],
    })
}

// ── Utilities ─────────────────────────────────────────────────────────────────

/// Recursively collect all `.jsonl` files under `dir`.
fn walk_jsonl(dir: &Path) -> Vec<PathBuf> {
    walk_inner(dir, 0)
}

fn walk_inner(dir: &Path, depth: u32) -> Vec<PathBuf> {
    const MAX_DEPTH: u32 = 8;
    let mut results = Vec::new();
    if depth >= MAX_DEPTH {
        return results;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_symlink() {
                continue;
            }
            let path = entry.path();
            if ft.is_dir() {
                results.extend(walk_inner(&path, depth + 1));
            } else if path.extension().map(|e| e == "jsonl").unwrap_or(false) {
                results.push(path);
            }
        }
    }
    results
}

fn parse_iso_ms(s: &str) -> Option<i64> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp_millis());
    }
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Some(dt.and_utc().timestamp_millis());
    }
    warn!("Codex: unparseable timestamp {:?}", s);
    None
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_jsonl(lines: &[serde_json::Value]) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        for line in lines {
            writeln!(f, "{}", line).unwrap();
        }
        f
    }

    fn session_meta(id: &str, cwd: &str, branch: &str) -> serde_json::Value {
        serde_json::json!({
            "timestamp": "2026-04-24T12:49:47.000Z",
            "type": "session_meta",
            "payload": {
                "id": id,
                "timestamp": "2026-04-24T12:49:47.000Z",
                "cwd": cwd,
                "cli_version": "0.124.0",
                "model_provider": "openai",
                "git": { "branch": branch }
            }
        })
    }

    fn task_started(turn_id: &str, started: i64, cw: i64) -> serde_json::Value {
        serde_json::json!({
            "timestamp": "2026-04-24T12:50:05.000Z",
            "type": "event_msg",
            "payload": {
                "type": "task_started",
                "turn_id": turn_id,
                "started_at": started,
                "model_context_window": cw
            }
        })
    }

    fn turn_context(turn_id: &str, model: &str) -> serde_json::Value {
        serde_json::json!({
            "timestamp": "2026-04-24T12:50:05.100Z",
            "type": "turn_context",
            "payload": { "turn_id": turn_id, "model": model }
        })
    }

    fn token_count(input: i64, cached: i64, output: i64, reasoning: i64) -> serde_json::Value {
        serde_json::json!({
            "timestamp": "2026-04-24T12:50:06.000Z",
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": {
                    "last_token_usage": {
                        "input_tokens": input,
                        "cached_input_tokens": cached,
                        "output_tokens": output,
                        "reasoning_output_tokens": reasoning,
                        "total_tokens": input + output
                    },
                    "model_context_window": 258400
                }
            }
        })
    }

    fn token_count_null() -> serde_json::Value {
        serde_json::json!({
            "timestamp": "2026-04-24T12:50:05.500Z",
            "type": "event_msg",
            "payload": { "type": "token_count", "info": null }
        })
    }

    fn task_complete(turn_id: &str, completed: i64, duration: i64) -> serde_json::Value {
        serde_json::json!({
            "timestamp": "2026-04-24T12:50:07.000Z",
            "type": "event_msg",
            "payload": {
                "type": "task_complete",
                "turn_id": turn_id,
                "completed_at": completed,
                "duration_ms": duration
            }
        })
    }

    fn function_call(call_id: &str, name: &str, args: &str) -> serde_json::Value {
        serde_json::json!({
            "timestamp": "2026-04-24T12:50:06.500Z",
            "type": "response_item",
            "payload": {
                "type": "function_call",
                "call_id": call_id,
                "name": name,
                "arguments": args
            }
        })
    }

    #[test]
    fn test_single_turn() {
        let turn_id = "aaaa-1111";
        let file = write_jsonl(&[
            session_meta("sess-1", "/home/user/project", "main"),
            task_started(turn_id, 1_745_500_000, 258_400),
            turn_context(turn_id, "gpt-5.4-mini"),
            token_count(1000, 500, 200, 50),
            task_complete(turn_id, 1_745_500_010, 5_000),
        ]);
        let result = parse_codex_file(file.path()).unwrap();

        let session = result.session.unwrap();
        assert_eq!(session.id, "codex-sess-1");
        assert_eq!(session.provider, "codex");
        assert_eq!(session.git_branch, "main");
        assert_eq!(session.total_turns, 1);

        assert_eq!(result.turns.len(), 1);
        let turn = &result.turns[0];
        assert_eq!(turn.session_id, "codex-sess-1");
        assert_eq!(turn.input_tokens, 1000);
        assert_eq!(turn.cache_read_tokens, 500);
        assert_eq!(turn.output_tokens, 200);
        assert_eq!(turn.thinking_tokens, 50);
        assert_eq!(turn.model, "gpt-5.4-mini");
        assert_eq!(turn.duration_ms, Some(5_000));
        assert_eq!(turn.timestamp, 1_745_500_010 * 1000);
    }

    #[test]
    fn test_multiple_turns() {
        let t1 = "turn-0001";
        let t2 = "turn-0002";
        let file = write_jsonl(&[
            session_meta("sess-2", "/home/user/project", "dev"),
            task_started(t1, 1_745_500_000, 258_400),
            turn_context(t1, "gpt-5.4-mini"),
            token_count(1000, 500, 200, 0),
            task_complete(t1, 1_745_500_010, 3_000),
            task_started(t2, 1_745_500_020, 258_400),
            turn_context(t2, "gpt-5.4-mini"),
            token_count(2000, 1000, 400, 100),
            task_complete(t2, 1_745_500_035, 7_000),
        ]);
        let result = parse_codex_file(file.path()).unwrap();

        assert_eq!(result.turns.len(), 2);
        assert_eq!(result.turns[0].turn_index, 0);
        assert_eq!(result.turns[1].turn_index, 1);
        assert_eq!(result.turns[0].input_tokens, 1000);
        assert_eq!(result.turns[1].input_tokens, 2000);
    }

    #[test]
    fn test_null_token_count_uses_last_non_null() {
        let tid = "turn-null";
        let file = write_jsonl(&[
            session_meta("sess-3", "/home/user/project", "main"),
            task_started(tid, 1_745_500_000, 258_400),
            turn_context(tid, "gpt-5.4-mini"),
            token_count_null(), // must be ignored
            token_count(800, 200, 150, 0),
            task_complete(tid, 1_745_500_010, 2_000),
        ]);
        let result = parse_codex_file(file.path()).unwrap();

        assert_eq!(result.turns.len(), 1);
        assert_eq!(result.turns[0].input_tokens, 800);
        assert_eq!(result.turns[0].output_tokens, 150);
    }

    #[test]
    fn test_incomplete_turn_not_emitted() {
        let tid = "turn-incomplete";
        let file = write_jsonl(&[
            session_meta("sess-4", "/home/user/project", "main"),
            task_started(tid, 1_745_500_000, 258_400),
            turn_context(tid, "gpt-5.4-mini"),
            token_count(1000, 500, 200, 0),
            // No task_complete
        ]);
        let result = parse_codex_file(file.path()).unwrap();

        assert_eq!(result.turns.len(), 0);
        // Session was still parsed
        assert!(result.session.is_some());
    }

    #[test]
    fn test_tool_call_attributed_to_correct_turn() {
        let t1 = "turn-tool-1";
        let t2 = "turn-tool-2";
        let file = write_jsonl(&[
            session_meta("sess-5", "/home/user/project", "main"),
            task_started(t1, 1_745_500_000, 258_400),
            turn_context(t1, "gpt-5.4-mini"),
            function_call("call-a", "read_file", r#"{"path": "src/main.rs"}"#),
            token_count(1000, 500, 200, 0),
            task_complete(t1, 1_745_500_010, 3_000),
            task_started(t2, 1_745_500_020, 258_400),
            turn_context(t2, "gpt-5.4-mini"),
            function_call("call-b", "write_file", r#"{"path": "out.txt"}"#),
            token_count(2000, 1000, 400, 0),
            task_complete(t2, 1_745_500_035, 5_000),
        ]);
        let result = parse_codex_file(file.path()).unwrap();

        assert_eq!(result.tool_calls.len(), 2);
        let turn_id_0 = &result.turns[0].id;
        let turn_id_1 = &result.turns[1].id;
        assert_eq!(result.tool_calls[0].turn_id, *turn_id_0);
        assert_eq!(result.tool_calls[1].turn_id, *turn_id_1);
        assert_eq!(result.tool_calls[0].tool_name, "read_file");
        assert_eq!(result.tool_calls[1].tool_name, "write_file");
    }

    #[test]
    fn test_multiple_token_counts_uses_last() {
        let tid = "turn-multi-tok";
        let file = write_jsonl(&[
            session_meta("sess-6", "/home/user/project", "main"),
            task_started(tid, 1_745_500_000, 258_400),
            turn_context(tid, "gpt-5.4-mini"),
            token_count(100, 50, 20, 0), // streaming partial — must be overwritten
            token_count(500, 200, 80, 10), // final — must be used
            task_complete(tid, 1_745_500_010, 1_500),
        ]);
        let result = parse_codex_file(file.path()).unwrap();

        assert_eq!(result.turns.len(), 1);
        assert_eq!(result.turns[0].input_tokens, 500);
        assert_eq!(result.turns[0].output_tokens, 80);
    }

    #[test]
    fn test_env_var_override() {
        let _guard = {
            // Serialize env-mutating tests to avoid races.
            static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
            LOCK.lock().unwrap()
        };
        let custom = std::env::temp_dir().join("scopeon_test_codex");
        std::env::set_var("CODEX_CONFIG_DIR", custom.to_str().unwrap());
        let provider = CodexProvider::new();
        assert_eq!(provider.sessions_dir, custom.join("sessions"));
        std::env::remove_var("CODEX_CONFIG_DIR");
    }
}
