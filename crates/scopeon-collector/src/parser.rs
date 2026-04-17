use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

use anyhow::Result;
use serde::Deserialize;
use serde_json::Value;
use tracing::{debug, warn};

use scopeon_core::cost::calculate_turn_cost;
use scopeon_core::models::{fnv1a_64, InteractionEvent, Session, ToolCall, Turn};

// ── Raw JSONL deserialization types ──────────────────────────────────────────

#[derive(Deserialize, Debug)]
struct RawEntry {
    #[serde(rename = "type")]
    entry_type: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    cwd: Option<String>,
    slug: Option<String>,
    version: Option<String>,
    #[serde(rename = "gitBranch")]
    git_branch: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "durationMs")]
    duration_ms: Option<i64>,
    message: Option<Value>,
    /// §8.2: Some AI coding tools log the API request max_tokens alongside the response.
    /// Claude Code may surface this as top-level `maxTokens` or in a request metadata field.
    #[serde(rename = "maxTokens")]
    max_tokens: Option<i64>,
}

#[derive(Deserialize, Debug)]
struct RawUsage {
    input_tokens: Option<i64>,
    cache_creation_input_tokens: Option<i64>,
    cache_read_input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    /// Precise thinking token count provided by the Anthropic API (extended thinking).
    /// Prefer this over the character-count heuristic in `ContentAnalysis`.
    thinking_tokens: Option<i64>,
    service_tier: Option<String>,
    cache_creation: Option<RawCacheCreation>,
}

#[derive(Deserialize, Debug)]
struct RawCacheCreation {
    ephemeral_5m_input_tokens: Option<i64>,
    ephemeral_1h_input_tokens: Option<i64>,
}

/// Named result of `analyze_content`, replacing the former 5-tuple.
struct ContentAnalysis {
    thinking_tokens: i64,
    mcp_call_count: i64,
    mcp_input_token_est: i64,
    text_output_tokens: i64,
    tool_calls: Vec<ToolCall>,
    interaction_events: Vec<InteractionEvent>,
}
pub struct ParseResult {
    pub session: Option<Session>,
    pub turns: Vec<Turn>,
    pub tool_calls: Vec<ToolCall>,
    pub interaction_events: Vec<InteractionEvent>,
    pub new_offset: u64,
}

/// Parse new lines from a JSONL file starting at `from_offset`.
/// Returns the parsed data and the new byte offset.
pub fn parse_file_incremental(
    file_path: &Path,
    from_offset: u64,
    start_turn_index: i64,
) -> Result<ParseResult> {
    let file = File::open(file_path)?;
    let mut reader = BufReader::new(file);
    reader.seek(SeekFrom::Start(from_offset))?;

    let mut session: Option<Session> = None;
    let mut turns: Vec<Turn> = Vec::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    let mut interaction_events: Vec<InteractionEvent> = Vec::new();
    let mut turn_index: i64 = start_turn_index;
    let mut current_offset = from_offset;
    // IS-A: track msg_id → index in `turns` to deduplicate streaming records
    // (Claude Code emits 2-4 records per assistant turn with increasing output_tokens)
    let mut seen_msg_ids: HashMap<String, usize> = HashMap::new();

    // Detect whether this JSONL file belongs to a subagent.
    // Claude Code stores subagent files at: <project>/<parent-session-id>/subagents/<agent-id>.jsonl
    // The `sessionId` field inside the file equals the PARENT session's UUID — not the subagent's
    // own ID — so every subagent turn would overwrite the parent session record without this fix.
    // We derive a unique session ID and record the parent link.
    let subagent_ctx: Option<(String, String)> = (|| {
        let parent_dir = file_path.parent()?;
        if parent_dir.file_name()?.to_str()? != "subagents" {
            return None;
        }
        let session_dir = parent_dir.parent()?;
        let parent_id = session_dir.file_name()?.to_str()?.to_string();
        let agent_id = file_path.file_stem()?.to_str()?.to_string();
        Some((parent_id, agent_id))
    })();

    let mut line = String::new();
    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            current_offset += bytes_read as u64;
            continue;
        }

        match serde_json::from_str::<RawEntry>(trimmed) {
            Ok(entry) => {
                // Skip non-assistant messages, snapshots, etc.
                let entry_type = entry.entry_type.as_deref().unwrap_or("");
                if matches!(entry_type, "file-history-snapshot" | "user") {
                    current_offset += bytes_read as u64;
                    continue;
                }

                if let Some(msg) = &entry.message {
                    let role = msg.get("role").and_then(Value::as_str).unwrap_or("");
                    if role != "assistant" {
                        current_offset += bytes_read as u64;
                        continue;
                    }

                    let usage = msg
                        .get("usage")
                        .and_then(|u| serde_json::from_value::<RawUsage>(u.clone()).ok());
                    if usage.is_none() {
                        current_offset += bytes_read as u64;
                        continue;
                    }
                    let usage = usage.unwrap();

                    // For subagent files the JSONL `sessionId` is the parent's ID; use the
                    // derived unique ID instead so the subagent gets its own DB record.
                    let session_id = match &subagent_ctx {
                        Some((parent_id, agent_id)) => format!("{}-{}", parent_id, agent_id),
                        None => entry.session_id.clone().unwrap_or_default(),
                    };
                    let msg_id = msg
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let model = msg
                        .get("model")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();

                    // Parse timestamp (ISO8601) → unix ms
                    let ts_ms = entry
                        .timestamp
                        .as_deref()
                        .and_then(parse_iso_to_ms)
                        .unwrap_or(0);

                    // Analyze content blocks
                    let content = msg.get("content").and_then(Value::as_array);
                    let analysis = analyze_content(content, &msg_id, &session_id, ts_ms);

                    // Raw usage
                    let input_tokens = usage.input_tokens.unwrap_or(0);
                    let cache_read = usage.cache_read_input_tokens.unwrap_or(0);
                    let cache_write = usage.cache_creation_input_tokens.unwrap_or(0);
                    let cache_5m = usage
                        .cache_creation
                        .as_ref()
                        .and_then(|c| c.ephemeral_5m_input_tokens)
                        .unwrap_or(0);
                    let cache_1h = usage
                        .cache_creation
                        .as_ref()
                        .and_then(|c| c.ephemeral_1h_input_tokens)
                        .unwrap_or(0);
                    let output_tokens = usage.output_tokens.unwrap_or(0);
                    let service_tier = usage.service_tier.unwrap_or_default();

                    let cost = calculate_turn_cost(
                        &model,
                        input_tokens,
                        output_tokens,
                        cache_write,
                        cache_read,
                    );

                    // Build session info from first turn if not yet known
                    if session.is_none() && !session_id.is_empty() {
                        let cwd = entry.cwd.clone().unwrap_or_default();
                        let project_name = Path::new(&cwd)
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        // §8.2: Extract context window size from JSONL.
                        // Check top-level maxTokens first, then message.max_tokens.
                        let context_window_tokens = entry
                            .max_tokens
                            .or_else(|| msg.get("max_tokens").and_then(Value::as_i64))
                            .filter(|&v| v > 0);
                        session = Some(Session {
                            id: session_id.clone(),
                            project: cwd,
                            project_name,
                            slug: entry.slug.clone().unwrap_or_default(),
                            provider: "claude-code".to_string(),
                            provider_version: entry.version.clone().unwrap_or_default(),
                            model: model.clone(),
                            git_branch: entry.git_branch.clone().unwrap_or_default(),
                            started_at: ts_ms,
                            last_turn_at: ts_ms,
                            total_turns: 0,
                            is_subagent: subagent_ctx.is_some(),
                            parent_session_id: subagent_ctx
                                .as_ref()
                                .map(|(parent_id, _)| parent_id.clone()),
                            context_window_tokens,
                        });
                    }
                    if let Some(ref mut s) = session {
                        if ts_ms > s.last_turn_at {
                            s.last_turn_at = ts_ms;
                        }
                        if model != s.model && !model.is_empty() {
                            s.model = model.clone();
                        }
                        if s.provider_version.is_empty() {
                            s.provider_version = entry.version.clone().unwrap_or_default();
                        }
                        s.total_turns = turn_index + 1;
                        // §8.2: Update context_window_tokens if we find it later in the file.
                        if s.context_window_tokens.is_none() {
                            s.context_window_tokens = entry
                                .max_tokens
                                .or_else(|| msg.get("max_tokens").and_then(Value::as_i64))
                                .filter(|&v| v > 0);
                        }
                    }

                    let turn_id = if msg_id.is_empty() {
                        format!("{}-{}", session_id, turn_index)
                    } else {
                        msg_id
                    };

                    // IS-A: last-write-wins for streaming records.
                    // Claude Code appends 2-4 records per assistant turn sharing the same
                    // message id, each with progressively higher output_tokens. When we
                    // encounter a duplicate id within this batch we overwrite the existing
                    // turn's token fields rather than pushing a new Turn (which would
                    // inflate turn_index and session.total_turns).
                    if let Some(&existing_idx) = seen_msg_ids.get(&turn_id) {
                        let existing = &mut turns[existing_idx];
                        existing.output_tokens = output_tokens;
                        existing.thinking_tokens =
                            usage.thinking_tokens.unwrap_or(analysis.thinking_tokens);
                        existing.cache_read_tokens = cache_read;
                        existing.cache_write_tokens = cache_write;
                        existing.cache_write_5m_tokens = cache_5m;
                        existing.cache_write_1h_tokens = cache_1h;
                        existing.estimated_cost_usd = cost.total_usd;
                        existing.text_output_tokens = analysis.text_output_tokens;
                        existing.mcp_call_count = analysis.mcp_call_count;
                        existing.mcp_input_token_est = analysis.mcp_input_token_est;
                        // Update session model if it changed mid-stream
                        if !model.is_empty() {
                            existing.model = model;
                        }
                        tool_calls.extend(analysis.tool_calls);
                        interaction_events.extend(analysis.interaction_events);
                    } else {
                        let turn = Turn {
                            id: turn_id.clone(),
                            session_id: session_id.clone(),
                            turn_index,
                            timestamp: ts_ms,
                            duration_ms: entry.duration_ms,
                            input_tokens,
                            cache_read_tokens: cache_read,
                            cache_write_tokens: cache_write,
                            cache_write_5m_tokens: cache_5m,
                            cache_write_1h_tokens: cache_1h,
                            output_tokens,
                            // Prefer the API-provided count (precise); fall back to the
                            // 4-chars-per-token heuristic parsed from content blocks.
                            thinking_tokens: usage
                                .thinking_tokens
                                .unwrap_or(analysis.thinking_tokens),
                            mcp_call_count: analysis.mcp_call_count,
                            mcp_input_token_est: analysis.mcp_input_token_est,
                            text_output_tokens: analysis.text_output_tokens,
                            model,
                            service_tier,
                            estimated_cost_usd: cost.total_usd,
                            is_compaction_event: false,
                        };

                        seen_msg_ids.insert(turn_id, turns.len());
                        turn_index += 1;
                        turns.push(turn);
                        tool_calls.extend(analysis.tool_calls);
                        interaction_events.extend(analysis.interaction_events);
                    }
                }
            },
            Err(e) => {
                debug!("Skipping unparseable line in {:?}: {}", file_path, e);
            },
        }

        current_offset += bytes_read as u64;
    }

    Ok(ParseResult {
        session,
        turns,
        tool_calls,
        interaction_events,
        new_offset: current_offset,
    })
}

/// Analyze content blocks to compute thinking tokens, MCP calls, and text output tokens.
fn analyze_content(
    content: Option<&Vec<Value>>,
    turn_id: &str,
    session_id: &str,
    timestamp: i64,
) -> ContentAnalysis {
    let Some(blocks) = content else {
        return ContentAnalysis {
            thinking_tokens: 0,
            mcp_call_count: 0,
            mcp_input_token_est: 0,
            text_output_tokens: 0,
            tool_calls: vec![],
            interaction_events: vec![],
        };
    };

    let mut thinking_chars: i64 = 0;
    let mut text_chars: i64 = 0;
    let mut mcp_call_count: i64 = 0;
    let mut mcp_input_chars: i64 = 0;
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    let mut interaction_events: Vec<InteractionEvent> = Vec::new();

    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("thinking") => {
                let chars = block
                    .get("thinking")
                    .and_then(Value::as_str)
                    .map(|s| s.len() as i64)
                    .unwrap_or(0);
                thinking_chars += chars;
            },
            Some("text") => {
                let chars = block
                    .get("text")
                    .and_then(Value::as_str)
                    .map(|s| s.len() as i64)
                    .unwrap_or(0);
                text_chars += chars;
            },
            Some("tool_use") => {
                let tool_name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string();
                // §6.5: Only count MCP server tools (mcp__<server>__<tool>).
                // Built-in tools (Read, Write, Bash, Edit, etc.) are Claude Code internals
                // and should not inflate the MCP call count.
                if tool_name.starts_with("mcp__") {
                    mcp_call_count += 1;
                }
                let input_json = block
                    .get("input")
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                let input_chars = input_json.len() as i64;
                let input_hash = fnv1a_64(&input_json);
                mcp_input_chars += input_chars;
                let tool_id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                tool_calls.push(ToolCall {
                    id: if tool_id.is_empty() {
                        format!("{}-tool-{}", turn_id, mcp_call_count)
                    } else {
                        tool_id.clone()
                    },
                    turn_id: turn_id.to_string(),
                    session_id: session_id.to_string(),
                    tool_name: tool_name.clone(),
                    input_size_chars: input_chars,
                    input_hash,
                    timestamp,
                });
                let (kind, mcp_server, mcp_tool, name) = normalize_tool_name(&tool_name);
                interaction_events.push(InteractionEvent {
                    id: if tool_id.is_empty() {
                        format!("{}-interaction-{}", turn_id, interaction_events.len())
                    } else {
                        format!("{tool_id}-interaction")
                    },
                    session_id: session_id.to_string(),
                    turn_id: Some(turn_id.to_string()),
                    task_run_id: None,
                    correlation_id: non_empty(tool_id),
                    parent_id: None,
                    provider: "claude-code".to_string(),
                    timestamp,
                    kind,
                    phase: "single".to_string(),
                    name,
                    display_name: None,
                    mcp_server,
                    mcp_tool,
                    hook_type: None,
                    agent_type: None,
                    execution_mode: None,
                    model: None,
                    status: Some("observed".to_string()),
                    success: None,
                    input_size_chars: input_chars,
                    output_size_chars: 0,
                    prompt_size_chars: 0,
                    summary_size_chars: 0,
                    total_tokens: None,
                    total_tool_calls: None,
                    duration_ms: None,
                    estimated_input_tokens: input_chars / 4,
                    estimated_output_tokens: 0,
                    estimated_cost_usd: 0.0,
                    confidence: "estimated".to_string(),
                });
            },
            _ => {},
        }
    }

    // Estimate: 1 token ≈ 4 characters
    ContentAnalysis {
        thinking_tokens: thinking_chars / 4,
        mcp_call_count,
        mcp_input_token_est: mcp_input_chars / 4,
        text_output_tokens: text_chars / 4,
        tool_calls,
        interaction_events,
    }
}

fn normalize_tool_name(tool_name: &str) -> (String, Option<String>, Option<String>, String) {
    if let Some(rest) = tool_name.strip_prefix("mcp__") {
        let mut parts = rest.splitn(3, "__");
        let server = parts.next().unwrap_or("").to_string();
        let tool = parts.next().unwrap_or("").to_string();
        let name = if !server.is_empty() && !tool.is_empty() {
            format!("{server}::{tool}")
        } else {
            tool_name.to_string()
        };
        ("mcp".to_string(), non_empty(server), non_empty(tool), name)
    } else if tool_name == "task" {
        ("task".to_string(), None, None, "task".to_string())
    } else {
        ("tool".to_string(), None, None, tool_name.to_string())
    }
}

fn non_empty(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}

fn parse_iso_to_ms(s: &str) -> Option<i64> {
    // Try RFC 3339 / ISO 8601 with timezone (most common)
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp_millis());
    }
    // Fallback: YYYY-MM-DD HH:MM:SS (no timezone) — assume UTC
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Some(dt.and_utc().timestamp_millis());
    }
    warn!("Unparseable timestamp, defaulting to 0: {:?}", s);
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    /// Build a realistic assistant JSONL entry with given parameters.
    fn make_assistant_line(
        session_id: &str,
        msg_id: &str,
        input_tokens: i64,
        cache_read: i64,
        cache_write: i64,
        output_tokens: i64,
        content_blocks: serde_json::Value,
    ) -> String {
        serde_json::json!({
            "parentUuid": "parent-uuid",
            "isSidechain": false,
            "userType": "external",
            "cwd": "/home/user/project",
            "sessionId": session_id,
            "version": "2.1.15",
            "gitBranch": "main",
            "slug": "test-slug",
            "type": "assistant",
            "timestamp": "2024-01-15T10:30:00Z",
            "durationMs": 1500,
            "message": {
                "id": msg_id,
                "model": "claude-opus-4-5-20251101",
                "type": "message",
                "role": "assistant",
                "content": content_blocks,
                "usage": {
                    "input_tokens": input_tokens,
                    "cache_read_input_tokens": cache_read,
                    "cache_creation_input_tokens": cache_write,
                    "cache_creation": {
                        "ephemeral_5m_input_tokens": cache_write,
                        "ephemeral_1h_input_tokens": 0
                    },
                    "output_tokens": output_tokens,
                    "service_tier": "standard"
                }
            }
        })
        .to_string()
    }

    fn write_lines(lines: &[String]) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        for line in lines {
            writeln!(f, "{}", line).unwrap();
        }
        f
    }

    #[test]
    fn test_parse_single_text_turn() {
        let content = serde_json::json!([{"type": "text", "text": "Hello world"}]);
        let line = make_assistant_line("sess-A", "msg-1", 100, 500, 200, 50, content);
        let file = write_lines(&[line]);

        let result = parse_file_incremental(file.path(), 0, 0).unwrap();
        assert_eq!(result.turns.len(), 1);
        let turn = &result.turns[0];
        assert_eq!(turn.session_id, "sess-A");
        assert_eq!(turn.input_tokens, 100);
        assert_eq!(turn.cache_read_tokens, 500);
        assert_eq!(turn.cache_write_tokens, 200);
        assert_eq!(turn.output_tokens, 50);
        assert_eq!(turn.mcp_call_count, 0);
        assert_eq!(turn.thinking_tokens, 0);
    }

    #[test]
    fn test_parse_thinking_block_estimates_tokens() {
        // 400 chars of thinking → 400/4 = 100 tokens
        let thinking_text = "a".repeat(400);
        let content = serde_json::json!([
            {"type": "thinking", "thinking": thinking_text},
            {"type": "text", "text": "Answer"}
        ]);
        let line = make_assistant_line("sess-B", "msg-2", 10, 0, 0, 120, content);
        let file = write_lines(&[line]);

        let result = parse_file_incremental(file.path(), 0, 0).unwrap();
        let turn = &result.turns[0];
        assert_eq!(turn.thinking_tokens, 100); // 400 chars / 4
    }

    #[test]
    fn test_parse_tool_use_blocks_counted() {
        // §6.5: Only mcp__-prefixed tools increment mcp_call_count.
        // Built-in Claude Code tools (Read, Write, Bash, Edit) should not.
        let content = serde_json::json!([
            {"type": "tool_use", "id": "tool-1", "name": "mcp__github__create_pr", "input": {}},
            {"type": "tool_use", "id": "tool-2", "name": "bash", "input": {"command": "ls"}},
            {"type": "tool_use", "id": "tool-3", "name": "read_file", "input": {"path": "/foo"}},
            {"type": "text", "text": "Done"}
        ]);
        let line = make_assistant_line("sess-C", "msg-3", 50, 1000, 300, 80, content);
        let file = write_lines(&[line]);

        let result = parse_file_incremental(file.path(), 0, 0).unwrap();
        let turn = &result.turns[0];
        // Only the mcp__-prefixed tool counts; built-in tools do not.
        assert_eq!(turn.mcp_call_count, 1);
        assert_eq!(result.tool_calls.len(), 3);
        assert_eq!(result.tool_calls[0].tool_name, "mcp__github__create_pr");
        assert_eq!(result.tool_calls[1].tool_name, "bash");
        assert_eq!(result.tool_calls[2].tool_name, "read_file");
    }

    #[test]
    fn test_skips_non_assistant_entries() {
        // user message (no usage field) — should be skipped
        let user_line = serde_json::json!({
            "type": "user",
            "sessionId": "sess-D",
            "message": {"role": "user", "content": "Hello!"}
        })
        .to_string();

        // snapshot entry — should be skipped
        let snapshot_line = serde_json::json!({
            "type": "file-history-snapshot",
            "messageId": "snap-1",
            "snapshot": {}
        })
        .to_string();

        let content = serde_json::json!([{"type": "text", "text": "Hi"}]);
        let assistant_line = make_assistant_line("sess-D", "msg-real", 5, 0, 0, 10, content);

        let file = write_lines(&[user_line, snapshot_line, assistant_line]);
        let result = parse_file_incremental(file.path(), 0, 0).unwrap();
        assert_eq!(result.turns.len(), 1);
    }

    #[test]
    fn test_incremental_offset_tracking() {
        let content = serde_json::json!([{"type": "text", "text": "Turn 1"}]);
        let line1 = make_assistant_line("sess-E", "msg-a", 10, 0, 0, 5, content.clone());
        let line2 = make_assistant_line("sess-E", "msg-b", 20, 0, 0, 8, content);

        let file = write_lines(&[line1, line2]);

        // First parse: from offset 0 — should get both turns
        let result1 = parse_file_incremental(file.path(), 0, 0).unwrap();
        assert_eq!(result1.turns.len(), 2);
        let offset_after_first = result1.new_offset;

        // Second parse: from offset after all lines — should get nothing new
        let result2 = parse_file_incremental(file.path(), offset_after_first, 0).unwrap();
        assert_eq!(result2.turns.len(), 0);
        assert_eq!(result2.new_offset, offset_after_first);
    }

    #[test]
    fn test_multiple_turns_assigned_sequential_indices() {
        let content = serde_json::json!([{"type": "text", "text": "x"}]);
        let lines: Vec<String> = (0..4)
            .map(|i| {
                make_assistant_line(
                    "sess-F",
                    &format!("msg-{}", i),
                    10,
                    0,
                    0,
                    5,
                    content.clone(),
                )
            })
            .collect();

        let file = write_lines(&lines);
        let result = parse_file_incremental(file.path(), 0, 0).unwrap();

        assert_eq!(result.turns.len(), 4);
        for (i, turn) in result.turns.iter().enumerate() {
            assert_eq!(turn.turn_index, i as i64);
        }
    }

    #[test]
    fn test_session_populated_from_first_turn() {
        let content = serde_json::json!([{"type": "text", "text": "hi"}]);
        let line = make_assistant_line("sess-G", "msg-x", 5, 0, 0, 3, content);
        let file = write_lines(&[line]);

        let result = parse_file_incremental(file.path(), 0, 0).unwrap();
        let session = result.session.expect("session should be populated");
        assert_eq!(session.id, "sess-G");
        assert_eq!(session.project_name, "project");
        assert_eq!(session.slug, "test-slug");
        assert_eq!(session.git_branch, "main");
    }

    #[test]
    fn test_cost_estimated_per_turn() {
        let content = serde_json::json!([{"type": "text", "text": "hi"}]);
        // 1M input tokens on claude-opus-4-5 at $5/MTok = $5.00
        let line = make_assistant_line("sess-H", "msg-cost", 1_000_000, 0, 0, 0, content);
        let file = write_lines(&[line]);

        let result = parse_file_incremental(file.path(), 0, 0).unwrap();
        let turn = &result.turns[0];
        assert!((turn.estimated_cost_usd - 5.0).abs() < 0.001);
    }

    #[test]
    fn test_empty_file_returns_no_turns() {
        let file = write_lines(&[]);
        let result = parse_file_incremental(file.path(), 0, 0).unwrap();
        assert!(result.turns.is_empty());
        assert!(result.session.is_none());
    }

    #[test]
    fn test_parse_iso_timestamp_rfc3339() {
        // Standard RFC 3339 timestamp — parsed correctly (not epoch)
        let content = serde_json::json!([{"type": "text", "text": "hi"}]);
        let line = make_assistant_line("sess-ts1", "msg-ts1", 10, 0, 0, 5, content);
        let file = write_lines(&[line]);
        let result = parse_file_incremental(file.path(), 0, 0).unwrap();
        let ts = result.turns[0].timestamp;
        assert!(ts > 0, "RFC3339 timestamp should parse to non-zero ms");
        // 2024-01-15T10:30:00Z ≈ 1705312200000
        assert!(ts > 1_700_000_000_000, "timestamp should be in 2024 range");
    }

    #[test]
    fn test_parse_iso_timestamp_naive_fallback() {
        // YYYY-MM-DD HH:MM:SS format (no timezone) — uses NaiveDateTime fallback
        let naive_ts = "2024-03-01 12:00:00";
        let line = serde_json::json!({
            "parentUuid": "parent-uuid",
            "isSidechain": false,
            "userType": "external",
            "cwd": "/home/user/project",
            "sessionId": "sess-naive",
            "version": "2.1.15",
            "gitBranch": "main",
            "slug": "test-slug",
            "type": "assistant",
            "timestamp": naive_ts,
            "durationMs": 500,
            "message": {
                "id": "msg-naive",
                "model": "claude-opus-4-5-20251101",
                "type": "message",
                "role": "assistant",
                "content": [{"type": "text", "text": "hi"}],
                "usage": {
                    "input_tokens": 10,
                    "cache_read_input_tokens": 0,
                    "cache_creation_input_tokens": 0,
                    "cache_creation": {"ephemeral_5m_input_tokens": 0, "ephemeral_1h_input_tokens": 0},
                    "output_tokens": 5,
                    "service_tier": "standard"
                }
            }
        }).to_string();
        let file = write_lines(&[line]);
        let result = parse_file_incremental(file.path(), 0, 0).unwrap();
        assert_eq!(result.turns.len(), 1);
        // 2024-03-01 12:00:00 UTC ≈ 1709294400000
        assert!(
            result.turns[0].timestamp > 1_700_000_000_000,
            "naive timestamp fallback should parse correctly"
        );
    }

    #[test]
    fn test_parse_mcp_tool_input_size_chars() {
        // Tool input chars should be recorded on the tool call
        let big_input = "x".repeat(200);
        let content = serde_json::json!([
            {"type": "tool_use", "id": "t1", "name": "read_file",
             "input": {"path": big_input}},
            {"type": "text", "text": "done"}
        ]);
        let line = make_assistant_line("sess-tc2", "msg-tc2", 50, 0, 0, 20, content);
        let file = write_lines(&[line]);
        let result = parse_file_incremental(file.path(), 0, 0).unwrap();
        let tc = &result.tool_calls[0];
        assert!(
            tc.input_size_chars > 0,
            "input_size_chars should capture tool input length"
        );
    }

    #[test]
    fn test_parse_multiple_tool_uses_creates_separate_records() {
        // §6.5: mcp__-prefixed tools count; built-in tools (bash, write_file) do not.
        let content = serde_json::json!([
            {"type": "tool_use", "id": "t1", "name": "mcp__jira__create_ticket", "input": {}},
            {"type": "tool_use", "id": "t2", "name": "mcp__linear__add_comment", "input": {}},
            {"type": "tool_use", "id": "t3", "name": "bash", "input": {"cmd": "ls"}},
        ]);
        let line = make_assistant_line("sess-multi", "msg-multi", 100, 0, 0, 50, content);
        let file = write_lines(&[line]);
        let result = parse_file_incremental(file.path(), 0, 0).unwrap();
        // Two MCP server tools; built-in bash does not count.
        assert_eq!(result.turns[0].mcp_call_count, 2);
        assert_eq!(result.tool_calls.len(), 3);
    }

    #[test]
    fn test_turn_index_continues_from_existing_count() {
        // Simulate a re-parse from offset with start_turn_index = 5
        let content = serde_json::json!([{"type": "text", "text": "hi"}]);
        let line = make_assistant_line("sess-idx", "msg-idx", 10, 0, 0, 5, content);
        let file = write_lines(&[line]);
        // start_turn_index = 5 means new turns should start at index 5
        let result = parse_file_incremental(file.path(), 0, 5).unwrap();
        assert_eq!(result.turns[0].turn_index, 5);
    }

    #[test]
    fn test_streaming_dedup_same_msg_id_last_wins() {
        // IS-A: Three streaming records with the same msg_id but increasing output_tokens.
        // Only 1 turn should be produced, with the last record's token values.
        let content = serde_json::json!([{"type": "text", "text": "response"}]);
        let record1 = make_assistant_line("sess-sd", "msg-stream", 5000, 0, 0, 0, content.clone());
        let record2 =
            make_assistant_line("sess-sd", "msg-stream", 5000, 0, 0, 250, content.clone());
        let record3 =
            make_assistant_line("sess-sd", "msg-stream", 5000, 0, 0, 1823, content.clone());
        let file = write_lines(&[record1, record2, record3]);
        let result = parse_file_incremental(file.path(), 0, 0).unwrap();
        assert_eq!(
            result.turns.len(),
            1,
            "streaming duplicates must be deduplicated to 1 turn"
        );
        assert_eq!(
            result.turns[0].output_tokens, 1823,
            "last record's output_tokens must win"
        );
        assert_eq!(
            result.turns[0].turn_index, 0,
            "turn_index must not be inflated"
        );
    }

    #[test]
    fn test_streaming_dedup_different_msg_ids_stay_separate() {
        // IS-A: Two different msg_ids must produce two distinct turns.
        let content = serde_json::json!([{"type": "text", "text": "hi"}]);
        let line1 = make_assistant_line("sess-two", "msg-aaa", 1000, 0, 0, 100, content.clone());
        let line2 = make_assistant_line("sess-two", "msg-bbb", 2000, 0, 0, 200, content.clone());
        let file = write_lines(&[line1, line2]);
        let result = parse_file_incremental(file.path(), 0, 0).unwrap();
        assert_eq!(
            result.turns.len(),
            2,
            "different msg_ids must produce 2 turns"
        );
        assert_eq!(result.turns[0].output_tokens, 100);
        assert_eq!(result.turns[1].output_tokens, 200);
        assert_eq!(result.turns[0].turn_index, 0);
        assert_eq!(result.turns[1].turn_index, 1);
    }

    #[test]
    fn test_corrupt_json_line_skipped_gracefully() {
        let corrupt = "{ this is not valid json }";
        let content = serde_json::json!([{"type": "text", "text": "hi"}]);
        let good = make_assistant_line("sess-ok", "msg-ok", 10, 0, 0, 5, content);
        let file = write_lines(&[corrupt.to_string(), good]);
        // Should skip the corrupt line and parse the good one
        let result = parse_file_incremental(file.path(), 0, 0).unwrap();
        assert_eq!(result.turns.len(), 1);
    }
}
