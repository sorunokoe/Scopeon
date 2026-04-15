//! GitHub Copilot CLI provider.
//!
//! Reads session data from `~/.copilot/session-state/` JSONL files written by
//! the GitHub Copilot CLI (the terminal agent). Each session has:
//!
//! - `session.start`       → project, branch, timestamp
//! - `assistant.turn_start/end` → turns with duration
//! - `session.truncation`  → context window token counts per turn
//! - `tool.execution_start/complete` → tool/MCP call tracking
//! - `session.compaction_complete` → actual API token counts (input/output/cached)
//!
//! Note: only `session.compaction_complete` events expose real API tokens.
//! For regular turns, `preTruncationTokensInMessages` is used as a context-size proxy.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use anyhow::Result;
use serde::Deserialize;

use super::Provider;
use scopeon_core::{Database, Session, Turn};

pub struct CopilotCliProvider;

impl CopilotCliProvider {
    pub fn new() -> Self {
        Self
    }

    fn sessions_dir() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".copilot/session-state"))
    }
}

impl Default for CopilotCliProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for CopilotCliProvider {
    fn id(&self) -> &str {
        "copilot-cli"
    }
    fn name(&self) -> &str {
        "GitHub Copilot CLI"
    }
    fn description(&self) -> &str {
        "GitHub Copilot terminal agent. Reads JSONL sessions from ~/.copilot/session-state/. \
         Provides context window pressure, turn counts, tool calls, and compaction token data."
    }

    fn is_available(&self) -> bool {
        Self::sessions_dir().map(|p| p.exists()).unwrap_or(false)
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        Self::sessions_dir().into_iter().collect()
    }

    fn scan(&self, db: &Database) -> Result<usize> {
        let Some(dir) = Self::sessions_dir() else {
            return Ok(0);
        };
        if !dir.exists() {
            return Ok(0);
        }

        let mut total_new = 0usize;

        // Scan all *.jsonl files (flat session files) and session dirs
        let entries: Vec<_> = fs::read_dir(&dir)
            .map(|d| d.filter_map(|e| e.ok()).collect())
            .unwrap_or_default();

        for entry in &entries {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                // Flat session file
                if let Ok(n) = self.scan_session_file(&path, db) {
                    total_new += n;
                }
            } else if path.is_dir() {
                // Session directory — contains events.jsonl
                let events = path.join("events.jsonl");
                if events.exists() {
                    if let Ok(n) = self.scan_session_file(&events, db) {
                        total_new += n;
                    }
                }
            }
        }

        Ok(total_new)
    }
}

impl CopilotCliProvider {
    fn scan_session_file(&self, path: &PathBuf, db: &Database) -> Result<usize> {
        // Guard against unexpectedly large files (e.g. corrupted/malicious JSONL).
        // 20 MB is well above any real Copilot CLI session.
        const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
        if let Ok(meta) = fs::metadata(path) {
            if meta.len() > MAX_FILE_BYTES {
                return Ok(0);
            }
        }

        let file = fs::File::open(path)?;
        let reader = BufReader::new(file);
        let mut new_turns = 0usize;

        // Parse all events
        let mut session: Option<CopilotSession> = None;
        let mut pending_turns: HashMap<String, PendingTurn> = HashMap::new();
        let mut active_turn_id: Option<String> = None;
        let mut turn_tool_counts: HashMap<String, i64> = HashMap::new();

        for line in reader.lines().take(100_000) {
            let Ok(line) = line else { continue };
            let Ok(evt) = serde_json::from_str::<CopilotEvent>(&line) else {
                continue;
            };

            match evt.event_type.as_str() {
                "session.start" => {
                    let data = evt.data;
                    let sid = data
                        .get("sessionId")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if sid.is_empty() {
                        continue;
                    }

                    let start_time = data
                        .get("startTime")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let ts_ms = parse_iso_ms(start_time.as_str());
                    let ts_ms = if ts_ms == 0 {
                        parse_iso_ms(&evt.timestamp.unwrap_or_default())
                    } else {
                        ts_ms
                    };

                    let ctx = data.get("context");
                    let cwd = ctx
                        .and_then(|c| c.get("cwd"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let branch = ctx
                        .and_then(|c| c.get("branch"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let project_name = cwd
                        .rsplit('/')
                        .next()
                        .unwrap_or("copilot-session")
                        .to_string();

                    session = Some(CopilotSession {
                        id: format!("copilot-{}", sid),
                        project_path: cwd.to_string(),
                        project_name,
                        branch,
                        started_at: ts_ms,
                    });
                },

                "assistant.turn_start" => {
                    let turn_id = evt
                        .data
                        .get("turnId")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let ts_ms = parse_iso_ms(&evt.timestamp.unwrap_or_default());
                    active_turn_id = Some(turn_id.clone());
                    pending_turns.insert(
                        turn_id.clone(),
                        PendingTurn {
                            id: turn_id,
                            start_ms: ts_ms,
                            end_ms: 0,
                            context_tokens: 0,
                            token_limit: 200_000,
                            compaction_input: 0,
                            compaction_output: 0,
                            compaction_cached: 0,
                            is_compaction: false,
                        },
                    );
                },

                "session.truncation" => {
                    let pre = evt
                        .data
                        .get("preTruncationTokensInMessages")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let limit = evt
                        .data
                        .get("tokenLimit")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(200_000);

                    if let Some(tid) = &active_turn_id {
                        if let Some(turn) = pending_turns.get_mut(tid) {
                            turn.context_tokens = pre;
                            turn.token_limit = limit;
                        }
                    }
                },

                "tool.execution_start" => {
                    if let Some(tid) = &active_turn_id {
                        *turn_tool_counts.entry(tid.clone()).or_insert(0) += 1;
                    }
                },

                "assistant.turn_end" => {
                    let turn_id = evt
                        .data
                        .get("turnId")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let ts_ms = parse_iso_ms(&evt.timestamp.unwrap_or_default());
                    if let Some(turn) = pending_turns.get_mut(&turn_id) {
                        turn.end_ms = ts_ms;
                    }
                    active_turn_id = None;
                },

                "session.compaction_complete" => {
                    let used = evt.data.get("compactionTokensUsed");
                    if let Some(used) = used {
                        let input = used.get("input").and_then(|v| v.as_i64()).unwrap_or(0);
                        let output = used.get("output").and_then(|v| v.as_i64()).unwrap_or(0);
                        let cached = used
                            .get("cachedInput")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);

                        // Create a synthetic compaction turn
                        let ts_ms = parse_iso_ms(&evt.timestamp.clone().unwrap_or_default());
                        let comp_id = format!("compaction-{}", evt.id.as_deref().unwrap_or("0"));
                        pending_turns.insert(
                            comp_id.clone(),
                            PendingTurn {
                                id: comp_id,
                                start_ms: ts_ms,
                                end_ms: ts_ms,
                                context_tokens: input,
                                token_limit: 200_000,
                                compaction_input: input,
                                compaction_output: output,
                                compaction_cached: cached,
                                is_compaction: true,
                            },
                        );
                    }
                },

                _ => {},
            }
        }

        // Flush all completed turns into DB
        let Some(sess) = session else {
            return Ok(0);
        };

        let last_turn_at = pending_turns
            .values()
            .map(|t| t.end_ms.max(t.start_ms))
            .max()
            .unwrap_or(sess.started_at);

        // Model: Copilot CLI uses Claude Sonnet under the hood
        let model = "copilot-claude-sonnet".to_string();

        let db_session = Session {
            id: sess.id.clone(),
            project: sess.project_path.clone(),
            project_name: sess.project_name.clone(),
            slug: sess.project_name.to_lowercase().replace(' ', "-"),
            model: model.clone(),
            git_branch: sess.branch.clone(),
            started_at: sess.started_at,
            last_turn_at,
            total_turns: pending_turns.len() as i64,
            is_subagent: false,
            parent_session_id: None,
            context_window_tokens: None,
        };

        let _ = db.upsert_session(&db_session);

        let mut sorted_turns: Vec<_> = pending_turns.values().collect();
        sorted_turns.sort_by_key(|t| t.start_ms);

        for (idx, t) in sorted_turns.iter().enumerate() {
            let tool_count = turn_tool_counts.get(&t.id).copied().unwrap_or(0);
            let duration_ms = if t.end_ms > t.start_ms {
                Some(t.end_ms - t.start_ms)
            } else {
                None
            };

            // For compaction turns use actual API tokens; for regular turns use context proxy
            let (input_tokens, cache_read, cache_write, output_tokens) = if t.is_compaction {
                (
                    t.compaction_input,
                    t.compaction_cached,
                    0,
                    t.compaction_output,
                )
            } else {
                // context_tokens is the total context size at turn start
                // Approximate: treat as input (cache_read portion is invisible here)
                (t.context_tokens, 0, 0, 500i64) // 500 output token estimate
            };

            let turn_id = format!("{}-turn-{}", sess.id, idx);
            let cost = scopeon_core::cost::calculate_turn_cost(
                &model,
                input_tokens,
                output_tokens,
                cache_write,
                cache_read,
            )
            .total_usd;

            let turn = Turn {
                id: turn_id,
                session_id: sess.id.clone(),
                turn_index: idx as i64,
                timestamp: t.start_ms,
                duration_ms,
                input_tokens,
                cache_read_tokens: cache_read,
                cache_write_tokens: cache_write,
                cache_write_5m_tokens: 0,
                cache_write_1h_tokens: 0,
                output_tokens,
                thinking_tokens: 0,
                mcp_call_count: tool_count,
                mcp_input_token_est: 0,
                text_output_tokens: output_tokens,
                model: model.clone(),
                service_tier: "default".to_string(),
                estimated_cost_usd: cost,
                is_compaction_event: t.is_compaction,
            };

            if db.upsert_turn(&turn).is_ok() {
                new_turns += 1;
            }
        }

        Ok(new_turns)
    }
}

// ── Event schema ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CopilotEvent {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    data: serde_json::Value,
    id: Option<String>,
    timestamp: Option<String>,
}

struct CopilotSession {
    id: String,
    project_path: String,
    project_name: String,
    branch: String,
    started_at: i64,
}

struct PendingTurn {
    id: String,
    start_ms: i64,
    end_ms: i64,
    context_tokens: i64,
    token_limit: i64,
    compaction_input: i64,
    compaction_output: i64,
    compaction_cached: i64,
    is_compaction: bool,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn parse_iso_ms(s: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(0)
}
