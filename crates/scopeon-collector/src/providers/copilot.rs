//! GitHub Copilot CLI provider.
//!
//! Reads session data from `~/.copilot/session-state/` JSONL files written by
//! the GitHub Copilot CLI (the terminal agent). In addition to turn totals,
//! Copilot sessions emit rich lifecycle events for skills, hooks, MCP tools,
//! tasks, subagents, plan changes, and notifications. This provider normalizes
//! those events into Scopeon's provenance model.
//!
//! Note: only some Copilot events expose exact token totals. Interaction-level
//! attribution therefore carries a confidence label:
//! - exact      — token totals were emitted directly by the source event
//! - estimated  — derived from safe sizes (JSON/result lengths)

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Deserialize;
use serde_json::Value;

use super::Provider;
use scopeon_core::{
    cost::calculate_turn_cost, fnv1a_64, Database, InteractionEvent, Session, TaskRun, ToolCall,
    Turn,
};

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
         Captures turns, tasks, skills, hooks, MCP usage, subagents, and compaction data."
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
        let entries: Vec<_> = fs::read_dir(&dir)
            .map(|d| d.filter_map(|e| e.ok()).collect())
            .unwrap_or_default();

        for entry in &entries {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                if let Ok(n) = self.scan_session_file(&path, db) {
                    total_new += n;
                }
            } else if path.is_dir() {
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
        const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
        if let Ok(meta) = fs::metadata(path) {
            if meta.len() > MAX_FILE_BYTES {
                return Ok(0);
            }
        }

        let file = fs::File::open(path)?;
        let reader = BufReader::new(file);

        let fallback_session_id = fallback_session_id(path);
        let mut new_turns = 0usize;

        let mut session: Option<CopilotSession> = None;
        let mut pending_turns: HashMap<String, PendingTurn> = HashMap::new();
        let mut active_turn_id: Option<String> = None;
        let mut turn_tool_counts: HashMap<String, i64> = HashMap::new();
        let mut session_token_limit: Option<i64> = None;
        let mut current_model = "copilot-claude-sonnet".to_string();
        let mut pending_user_input_tokens: i64 = 0;

        let mut tool_starts: HashMap<String, ToolExecutionStart> = HashMap::new();
        let mut pending_tool_calls: Vec<ToolCall> = Vec::new();
        let mut interaction_events: Vec<InteractionEvent> = Vec::new();
        let mut task_runs: HashMap<String, TaskRun> = HashMap::new();
        let mut open_tasks: Vec<String> = Vec::new();

        for line in reader.lines().take(100_000) {
            let Ok(line) = line else { continue };
            let Ok(evt) = serde_json::from_str::<CopilotEvent>(&line) else {
                continue;
            };

            let ts_ms = parse_iso_ms(evt.timestamp.as_deref().unwrap_or_default());
            let session_id = session
                .as_ref()
                .map(|s| s.id.clone())
                .unwrap_or_else(|| fallback_session_id.clone());

            match evt.event_type.as_str() {
                "session.start" => {
                    let data = &evt.data;
                    let raw_sid = data
                        .get("sessionId")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if raw_sid.is_empty() {
                        continue;
                    }

                    let start_time = data.get("startTime").and_then(Value::as_str).unwrap_or("");
                    let started_at = parse_iso_ms(start_time);
                    let started_at = if started_at == 0 { ts_ms } else { started_at };

                    let ctx = data.get("context");
                    let cwd = ctx
                        .and_then(|c| c.get("cwd"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let branch = ctx
                        .and_then(|c| c.get("branch"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let project_name = cwd
                        .rsplit('/')
                        .next()
                        .unwrap_or("copilot-session")
                        .to_string();
                    let provider_version = data
                        .get("copilotVersion")
                        .and_then(Value::as_str)
                        .or_else(|| data.get("version").and_then(Value::as_str))
                        .unwrap_or("")
                        .to_string();

                    session = Some(CopilotSession {
                        id: format!("copilot-{}", raw_sid),
                        project_path: cwd.to_string(),
                        project_name,
                        branch,
                        started_at,
                        provider_version,
                    });
                },

                "user.message" => {
                    // Extract input tokens from user message content (estimate: ~4 chars per token)
                    let input_tokens = evt
                        .data
                        .get("content")
                        .and_then(Value::as_str)
                        .map(|s| (s.len() as i64) / 4)
                        .unwrap_or(0);

                    // Store for the next turn that starts
                    pending_user_input_tokens = input_tokens;
                },

                "assistant.turn_start" => {
                    let turn_id = evt
                        .data
                        .get("turnId")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if turn_id.is_empty() {
                        continue;
                    }
                    active_turn_id = Some(turn_id.clone());
                    pending_turns.insert(
                        turn_id.clone(),
                        PendingTurn {
                            id: turn_id,
                            start_ms: ts_ms,
                            end_ms: 0,
                            context_tokens: 0,
                            token_limit: session_token_limit.unwrap_or(200_000),
                            output_tokens: 0,
                            thinking_tokens: 0,
                            input_tokens: pending_user_input_tokens,
                            cache_read_tokens: 0,
                            cache_write_tokens: 0,
                            model: Some(current_model.clone()),
                            compaction_input: 0,
                            compaction_output: 0,
                            compaction_cached: 0,
                            is_compaction: false,
                        },
                    );
                    pending_user_input_tokens = 0; // Reset after applying
                },

                "assistant.message" => {
                    let output_tokens = evt
                        .data
                        .get("outputTokens")
                        .and_then(Value::as_i64)
                        .unwrap_or(0);

                    // Extract thinking tokens from reasoningText length (estimate: ~4 chars per token)
                    let thinking_tokens = evt
                        .data
                        .get("reasoningText")
                        .and_then(Value::as_str)
                        .map(|s| (s.len() as i64) / 4)
                        .unwrap_or(0);

                    if let Some(tid) = &active_turn_id {
                        if let Some(turn) = pending_turns.get_mut(tid) {
                            turn.output_tokens = turn.output_tokens.max(output_tokens);
                            turn.thinking_tokens = turn.thinking_tokens.max(thinking_tokens);
                            if let Some(model) = evt
                                .data
                                .get("model")
                                .and_then(Value::as_str)
                                .map(ToString::to_string)
                            {
                                turn.model = Some(model.clone());
                                current_model = model;
                            }
                        }
                    }
                },

                "session.truncation" => {
                    let pre = evt
                        .data
                        .get("preTruncationTokensInMessages")
                        .and_then(Value::as_i64)
                        .unwrap_or(0);
                    let limit = evt
                        .data
                        .get("tokenLimit")
                        .and_then(Value::as_i64)
                        .unwrap_or(200_000);
                    session_token_limit = Some(session_token_limit.unwrap_or(limit).max(limit));

                    if let Some(tid) = &active_turn_id {
                        if let Some(turn) = pending_turns.get_mut(tid) {
                            turn.context_tokens = pre;
                            turn.token_limit = limit;
                        }
                    }
                },

                "tool.execution_start" => {
                    let tool_name = evt
                        .data
                        .get("toolName")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_string();
                    let tool_call_id = evt
                        .data
                        .get("toolCallId")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let mcp_server = evt
                        .data
                        .get("mcpServerName")
                        .and_then(Value::as_str)
                        .map(ToString::to_string);
                    let mcp_tool = evt
                        .data
                        .get("mcpToolName")
                        .and_then(Value::as_str)
                        .map(ToString::to_string);
                    let args = evt.data.get("arguments").cloned().unwrap_or(Value::Null);
                    let input_chars = json_chars(&args);
                    let turn_id = active_turn_id.clone();

                    if let Some(tid) = &turn_id {
                        *turn_tool_counts.entry(tid.clone()).or_insert(0) += 1;
                    }

                    tool_starts.insert(
                        tool_call_id.clone(),
                        ToolExecutionStart {
                            tool_name: tool_name.clone(),
                            mcp_server: mcp_server.clone(),
                            mcp_tool: mcp_tool.clone(),
                            input_chars,
                            args: args.clone(),
                            turn_id: turn_id.clone(),
                        },
                    );

                    if let Some(tid) = &turn_id {
                        pending_tool_calls.push(ToolCall {
                            id: format!("{}-toolcall-{}", session_id, tool_call_id),
                            turn_id: tid.clone(),
                            session_id: session_id.clone(),
                            tool_name: display_name_for_tool(
                                &tool_name,
                                mcp_server.as_deref(),
                                mcp_tool.as_deref(),
                            ),
                            input_size_chars: input_chars,
                            input_hash: fnv1a_64(&args.to_string()),
                            timestamp: ts_ms,
                        });
                    }

                    let (kind, name) = interaction_kind_and_name(
                        &tool_name,
                        mcp_server.as_deref(),
                        mcp_tool.as_deref(),
                    );
                    let task_run_id = if tool_name == "task" && !tool_call_id.is_empty() {
                        let task_id = format!("task-{}-{}", session_id, tool_call_id);
                        let task = TaskRun {
                            id: task_id.clone(),
                            session_id: session_id.clone(),
                            correlation_id: Some(tool_call_id.clone()),
                            name: args
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or("task")
                                .to_string(),
                            display_name: None,
                            agent_type: args
                                .get("agent_type")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                            execution_mode: args
                                .get("mode")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                            requested_model: args
                                .get("model")
                                .and_then(Value::as_str)
                                .map(ToString::to_string),
                            actual_model: None,
                            started_at: ts_ms,
                            completed_at: None,
                            duration_ms: None,
                            success: None,
                            total_tokens: None,
                            total_tool_calls: None,
                            description_size_chars: evt
                                .data
                                .get("arguments")
                                .and_then(|v| v.get("description"))
                                .and_then(Value::as_str)
                                .map(|s| s.len() as i64)
                                .unwrap_or(0),
                            prompt_size_chars: evt
                                .data
                                .get("arguments")
                                .and_then(|v| v.get("prompt"))
                                .and_then(Value::as_str)
                                .map(|s| s.len() as i64)
                                .unwrap_or(0),
                            summary_size_chars: 0,
                            confidence: "estimated".to_string(),
                        };
                        task_runs.insert(task_id.clone(), task);
                        open_tasks.push(task_id.clone());
                        Some(task_id)
                    } else {
                        None
                    };

                    interaction_events.push(InteractionEvent {
                        id: stable_event_id(
                            &session_id,
                            "tool-start",
                            &tool_call_id,
                            evt.id.as_deref(),
                            ts_ms,
                        ),
                        session_id: session_id.clone(),
                        turn_id,
                        task_run_id,
                        correlation_id: non_empty(tool_call_id),
                        parent_id: None,
                        provider: "copilot-cli".to_string(),
                        timestamp: ts_ms,
                        kind: kind.to_string(),
                        phase: "start".to_string(),
                        name,
                        display_name: None,
                        mcp_server,
                        mcp_tool,
                        hook_type: None,
                        agent_type: args
                            .get("agent_type")
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                        execution_mode: args
                            .get("mode")
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                        model: args
                            .get("model")
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                        status: Some("started".to_string()),
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

                "tool.execution_complete" => {
                    let tool_call_id = evt
                        .data
                        .get("toolCallId")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let tool_start = tool_starts.get(&tool_call_id);
                    let tool_name = tool_start
                        .map(|s| s.tool_name.clone())
                        .unwrap_or_else(|| "unknown".to_string());
                    let mcp_server = tool_start.and_then(|s| s.mcp_server.clone());
                    let mcp_tool = tool_start.and_then(|s| s.mcp_tool.clone());
                    let turn_id = tool_start.and_then(|s| s.turn_id.clone());
                    let input_chars = tool_start.map(|s| s.input_chars).unwrap_or(0);
                    let telemetry = evt.data.get("toolTelemetry");
                    let prompt_chars = telemetry
                        .and_then(|t| t.get("metrics"))
                        .and_then(|m| metric_i64(m, &["skillContentLength"]))
                        .unwrap_or(0);
                    let output_chars = output_chars_from_tool_complete(&evt.data);
                    let success = evt
                        .data
                        .get("success")
                        .and_then(Value::as_bool)
                        .or(Some(true));
                    let model = evt
                        .data
                        .get("model")
                        .and_then(Value::as_str)
                        .map(ToString::to_string);

                    let (kind, name) = interaction_kind_and_name(
                        &tool_name,
                        mcp_server.as_deref(),
                        mcp_tool.as_deref(),
                    );
                    let task_run_id = if tool_name == "task" && !tool_call_id.is_empty() {
                        Some(format!("task-{}-{}", session_id, tool_call_id))
                    } else {
                        None
                    };

                    interaction_events.push(InteractionEvent {
                        id: stable_event_id(
                            &session_id,
                            "tool-complete",
                            &tool_call_id,
                            evt.id.as_deref(),
                            ts_ms,
                        ),
                        session_id: session_id.clone(),
                        turn_id,
                        task_run_id,
                        correlation_id: non_empty(tool_call_id.clone()),
                        parent_id: None,
                        provider: "copilot-cli".to_string(),
                        timestamp: ts_ms,
                        kind: kind.to_string(),
                        phase: "complete".to_string(),
                        name,
                        display_name: None,
                        mcp_server,
                        mcp_tool,
                        hook_type: None,
                        agent_type: tool_start
                            .and_then(|s| s.args.get("agent_type"))
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                        execution_mode: tool_start
                            .and_then(|s| s.args.get("mode"))
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                        model,
                        status: Some(if success == Some(true) {
                            "completed".to_string()
                        } else {
                            "failed".to_string()
                        }),
                        success,
                        input_size_chars: input_chars,
                        output_size_chars: output_chars,
                        prompt_size_chars: prompt_chars,
                        summary_size_chars: 0,
                        total_tokens: None,
                        total_tool_calls: None,
                        duration_ms: None,
                        estimated_input_tokens: input_chars / 4,
                        estimated_output_tokens: output_chars / 4,
                        estimated_cost_usd: 0.0,
                        confidence: "estimated".to_string(),
                    });
                },

                "hook.start" => {
                    let hook_id = evt
                        .data
                        .get("hookInvocationId")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let hook_type = evt
                        .data
                        .get("hookType")
                        .and_then(Value::as_str)
                        .unwrap_or("hook")
                        .to_string();
                    let calls = evt
                        .data
                        .get("input")
                        .and_then(|i| i.get("toolCalls"))
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();

                    for call in calls {
                        let tool_id = call
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let tool_name = call
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                            .to_string();
                        let args = call.get("args").cloned().unwrap_or(Value::Null);
                        let input_chars = json_chars(&args);
                        interaction_events.push(InteractionEvent {
                            id: stable_event_id(
                                &session_id,
                                "hook-start",
                                &format!("{hook_id}:{tool_id}"),
                                evt.id.as_deref(),
                                ts_ms,
                            ),
                            session_id: session_id.clone(),
                            turn_id: active_turn_id.clone(),
                            task_run_id: None,
                            correlation_id: non_empty(tool_id),
                            parent_id: non_empty(hook_id.clone()),
                            provider: "copilot-cli".to_string(),
                            timestamp: ts_ms,
                            kind: "hook".to_string(),
                            phase: "start".to_string(),
                            name: tool_name,
                            display_name: None,
                            mcp_server: None,
                            mcp_tool: None,
                            hook_type: Some(hook_type.clone()),
                            agent_type: None,
                            execution_mode: None,
                            model: None,
                            status: Some("started".to_string()),
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
                    }
                },

                "hook.end" => {
                    let hook_id = evt
                        .data
                        .get("hookInvocationId")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let hook_type = evt
                        .data
                        .get("hookType")
                        .and_then(Value::as_str)
                        .unwrap_or("hook")
                        .to_string();
                    let success = evt.data.get("success").and_then(Value::as_bool);
                    interaction_events.push(InteractionEvent {
                        id: stable_event_id(
                            &session_id,
                            "hook-end",
                            &hook_id,
                            evt.id.as_deref(),
                            ts_ms,
                        ),
                        session_id: session_id.clone(),
                        turn_id: active_turn_id.clone(),
                        task_run_id: None,
                        correlation_id: None,
                        parent_id: non_empty(hook_id),
                        provider: "copilot-cli".to_string(),
                        timestamp: ts_ms,
                        kind: "hook".to_string(),
                        phase: "complete".to_string(),
                        name: hook_type.clone(),
                        display_name: None,
                        mcp_server: None,
                        mcp_tool: None,
                        hook_type: Some(hook_type),
                        agent_type: None,
                        execution_mode: None,
                        model: None,
                        status: Some(if success == Some(true) {
                            "completed".to_string()
                        } else {
                            "failed".to_string()
                        }),
                        success,
                        input_size_chars: 0,
                        output_size_chars: 0,
                        prompt_size_chars: 0,
                        summary_size_chars: 0,
                        total_tokens: None,
                        total_tool_calls: None,
                        duration_ms: None,
                        estimated_input_tokens: 0,
                        estimated_output_tokens: 0,
                        estimated_cost_usd: 0.0,
                        confidence: "estimated".to_string(),
                    });
                },

                "skill.invoked" => {
                    let name = evt
                        .data
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("skill")
                        .to_string();
                    let display_name = evt
                        .data
                        .get("description")
                        .and_then(Value::as_str)
                        .map(ToString::to_string);
                    let prompt_chars = evt
                        .data
                        .get("content")
                        .and_then(Value::as_str)
                        .map(|s| s.len() as i64)
                        .unwrap_or(0);
                    interaction_events.push(InteractionEvent {
                        id: stable_event_id(&session_id, "skill", &name, evt.id.as_deref(), ts_ms),
                        session_id: session_id.clone(),
                        turn_id: active_turn_id.clone(),
                        task_run_id: None,
                        correlation_id: None,
                        parent_id: None,
                        provider: "copilot-cli".to_string(),
                        timestamp: ts_ms,
                        kind: "skill".to_string(),
                        phase: "invoked".to_string(),
                        name,
                        display_name,
                        mcp_server: None,
                        mcp_tool: None,
                        hook_type: None,
                        agent_type: None,
                        execution_mode: None,
                        model: None,
                        status: Some("invoked".to_string()),
                        success: Some(true),
                        input_size_chars: 0,
                        output_size_chars: 0,
                        prompt_size_chars: prompt_chars,
                        summary_size_chars: 0,
                        total_tokens: None,
                        total_tool_calls: None,
                        duration_ms: None,
                        estimated_input_tokens: prompt_chars / 4,
                        estimated_output_tokens: 0,
                        estimated_cost_usd: 0.0,
                        confidence: "estimated".to_string(),
                    });
                },

                "subagent.started" => {
                    let tool_call_id = evt
                        .data
                        .get("toolCallId")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let display_name = evt
                        .data
                        .get("agentDisplayName")
                        .and_then(Value::as_str)
                        .map(ToString::to_string);
                    let name = evt
                        .data
                        .get("agentName")
                        .and_then(Value::as_str)
                        .unwrap_or("subagent")
                        .to_string();
                    let task_run_id = format!("task-{}-{}", session_id, tool_call_id);
                    if let Some(task) = task_runs.get_mut(&task_run_id) {
                        if task.display_name.is_none() {
                            task.display_name = display_name.clone();
                        }
                    }
                    interaction_events.push(InteractionEvent {
                        id: stable_event_id(
                            &session_id,
                            "subagent-start",
                            &tool_call_id,
                            evt.id.as_deref(),
                            ts_ms,
                        ),
                        session_id: session_id.clone(),
                        turn_id: active_turn_id.clone(),
                        task_run_id: Some(task_run_id),
                        correlation_id: non_empty(tool_call_id),
                        parent_id: None,
                        provider: "copilot-cli".to_string(),
                        timestamp: ts_ms,
                        kind: "subagent".to_string(),
                        phase: "start".to_string(),
                        name,
                        display_name,
                        mcp_server: None,
                        mcp_tool: None,
                        hook_type: None,
                        agent_type: None,
                        execution_mode: None,
                        model: None,
                        status: Some("started".to_string()),
                        success: None,
                        input_size_chars: 0,
                        output_size_chars: 0,
                        prompt_size_chars: 0,
                        summary_size_chars: 0,
                        total_tokens: None,
                        total_tool_calls: None,
                        duration_ms: None,
                        estimated_input_tokens: 0,
                        estimated_output_tokens: 0,
                        estimated_cost_usd: 0.0,
                        confidence: "estimated".to_string(),
                    });
                },

                "subagent.completed" => {
                    let tool_call_id = evt
                        .data
                        .get("toolCallId")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let total_tokens = evt.data.get("totalTokens").and_then(Value::as_i64);
                    let total_tool_calls = evt.data.get("totalToolCalls").and_then(Value::as_i64);
                    let duration_ms = evt.data.get("durationMs").and_then(Value::as_i64);
                    let model = evt
                        .data
                        .get("model")
                        .and_then(Value::as_str)
                        .map(ToString::to_string);
                    let display_name = evt
                        .data
                        .get("agentDisplayName")
                        .and_then(Value::as_str)
                        .map(ToString::to_string);
                    let name = evt
                        .data
                        .get("agentName")
                        .and_then(Value::as_str)
                        .unwrap_or("subagent")
                        .to_string();
                    let task_run_id = format!("task-{}-{}", session_id, tool_call_id);

                    if let Some(task) = task_runs.get_mut(&task_run_id) {
                        task.actual_model = model.clone();
                        task.duration_ms = duration_ms;
                        task.total_tokens = total_tokens;
                        task.total_tool_calls = total_tool_calls;
                        if task.display_name.is_none() {
                            task.display_name = display_name.clone();
                        }
                        task.confidence = if total_tokens.is_some() {
                            "exact".to_string()
                        } else {
                            "estimated".to_string()
                        };
                    }

                    interaction_events.push(InteractionEvent {
                        id: stable_event_id(
                            &session_id,
                            "subagent-complete",
                            &tool_call_id,
                            evt.id.as_deref(),
                            ts_ms,
                        ),
                        session_id: session_id.clone(),
                        turn_id: active_turn_id.clone(),
                        task_run_id: Some(task_run_id),
                        correlation_id: non_empty(tool_call_id),
                        parent_id: None,
                        provider: "copilot-cli".to_string(),
                        timestamp: ts_ms,
                        kind: "subagent".to_string(),
                        phase: "complete".to_string(),
                        name,
                        display_name,
                        mcp_server: None,
                        mcp_tool: None,
                        hook_type: None,
                        agent_type: None,
                        execution_mode: None,
                        model,
                        status: Some("completed".to_string()),
                        success: Some(true),
                        input_size_chars: 0,
                        output_size_chars: 0,
                        prompt_size_chars: 0,
                        summary_size_chars: 0,
                        total_tokens,
                        total_tool_calls,
                        duration_ms,
                        estimated_input_tokens: 0,
                        estimated_output_tokens: 0,
                        estimated_cost_usd: 0.0,
                        confidence: if total_tokens.is_some() {
                            "exact".to_string()
                        } else {
                            "estimated".to_string()
                        },
                    });
                },

                "session.task_complete" => {
                    let success = evt.data.get("success").and_then(Value::as_bool);
                    let summary_chars = evt
                        .data
                        .get("summary")
                        .and_then(Value::as_str)
                        .map(|s| s.len() as i64)
                        .unwrap_or(0);
                    let task_run_id = open_tasks
                        .iter()
                        .rev()
                        .find(|task_id| {
                            task_runs
                                .get(task_id.as_str())
                                .map(|task| task.completed_at.is_none())
                                .unwrap_or(false)
                        })
                        .cloned();

                    if let Some(task_id) = &task_run_id {
                        if let Some(task) = task_runs.get_mut(task_id) {
                            task.completed_at = Some(ts_ms);
                            task.success = success;
                            task.summary_size_chars = summary_chars;
                        }
                    }

                    interaction_events.push(InteractionEvent {
                        id: stable_event_id(
                            &session_id,
                            "task-complete",
                            task_run_id.as_deref().unwrap_or("task"),
                            evt.id.as_deref(),
                            ts_ms,
                        ),
                        session_id: session_id.clone(),
                        turn_id: active_turn_id.clone(),
                        task_run_id,
                        correlation_id: None,
                        parent_id: None,
                        provider: "copilot-cli".to_string(),
                        timestamp: ts_ms,
                        kind: "task".to_string(),
                        phase: "complete".to_string(),
                        name: "task_complete".to_string(),
                        display_name: None,
                        mcp_server: None,
                        mcp_tool: None,
                        hook_type: None,
                        agent_type: None,
                        execution_mode: None,
                        model: None,
                        status: Some(if success == Some(true) {
                            "completed".to_string()
                        } else {
                            "failed".to_string()
                        }),
                        success,
                        input_size_chars: 0,
                        output_size_chars: 0,
                        prompt_size_chars: 0,
                        summary_size_chars: summary_chars,
                        total_tokens: None,
                        total_tool_calls: None,
                        duration_ms: None,
                        estimated_input_tokens: 0,
                        estimated_output_tokens: 0,
                        estimated_cost_usd: 0.0,
                        confidence: "estimated".to_string(),
                    });
                },

                "session.plan_changed" => {
                    let operation = evt
                        .data
                        .get("operation")
                        .and_then(Value::as_str)
                        .unwrap_or("changed")
                        .to_string();
                    interaction_events.push(InteractionEvent {
                        id: stable_event_id(
                            &session_id,
                            "plan",
                            &operation,
                            evt.id.as_deref(),
                            ts_ms,
                        ),
                        session_id: session_id.clone(),
                        turn_id: active_turn_id.clone(),
                        task_run_id: None,
                        correlation_id: None,
                        parent_id: None,
                        provider: "copilot-cli".to_string(),
                        timestamp: ts_ms,
                        kind: "plan".to_string(),
                        phase: "changed".to_string(),
                        name: operation,
                        display_name: None,
                        mcp_server: None,
                        mcp_tool: None,
                        hook_type: None,
                        agent_type: None,
                        execution_mode: None,
                        model: None,
                        status: Some("changed".to_string()),
                        success: Some(true),
                        input_size_chars: 0,
                        output_size_chars: 0,
                        prompt_size_chars: 0,
                        summary_size_chars: 0,
                        total_tokens: None,
                        total_tool_calls: None,
                        duration_ms: None,
                        estimated_input_tokens: 0,
                        estimated_output_tokens: 0,
                        estimated_cost_usd: 0.0,
                        confidence: "estimated".to_string(),
                    });
                },

                "session.mode_changed" => {
                    let new_mode = evt
                        .data
                        .get("newMode")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_string();
                    let previous = evt
                        .data
                        .get("previousMode")
                        .and_then(Value::as_str)
                        .map(ToString::to_string);
                    interaction_events.push(InteractionEvent {
                        id: stable_event_id(
                            &session_id,
                            "mode",
                            &new_mode,
                            evt.id.as_deref(),
                            ts_ms,
                        ),
                        session_id: session_id.clone(),
                        turn_id: active_turn_id.clone(),
                        task_run_id: None,
                        correlation_id: None,
                        parent_id: None,
                        provider: "copilot-cli".to_string(),
                        timestamp: ts_ms,
                        kind: "mode_change".to_string(),
                        phase: "changed".to_string(),
                        name: new_mode,
                        display_name: previous,
                        mcp_server: None,
                        mcp_tool: None,
                        hook_type: None,
                        agent_type: None,
                        execution_mode: None,
                        model: None,
                        status: Some("changed".to_string()),
                        success: Some(true),
                        input_size_chars: 0,
                        output_size_chars: 0,
                        prompt_size_chars: 0,
                        summary_size_chars: 0,
                        total_tokens: None,
                        total_tool_calls: None,
                        duration_ms: None,
                        estimated_input_tokens: 0,
                        estimated_output_tokens: 0,
                        estimated_cost_usd: 0.0,
                        confidence: "estimated".to_string(),
                    });
                },

                "session.model_change" => {
                    let new_model = evt
                        .data
                        .get("newModel")
                        .and_then(Value::as_str)
                        .unwrap_or("copilot-claude-sonnet")
                        .to_string();
                    current_model = new_model.clone();
                    interaction_events.push(InteractionEvent {
                        id: stable_event_id(
                            &session_id,
                            "model",
                            &new_model,
                            evt.id.as_deref(),
                            ts_ms,
                        ),
                        session_id: session_id.clone(),
                        turn_id: active_turn_id.clone(),
                        task_run_id: None,
                        correlation_id: None,
                        parent_id: None,
                        provider: "copilot-cli".to_string(),
                        timestamp: ts_ms,
                        kind: "model_change".to_string(),
                        phase: "changed".to_string(),
                        name: new_model,
                        display_name: None,
                        mcp_server: None,
                        mcp_tool: None,
                        hook_type: None,
                        agent_type: None,
                        execution_mode: None,
                        model: Some(current_model.clone()),
                        status: Some("changed".to_string()),
                        success: Some(true),
                        input_size_chars: 0,
                        output_size_chars: 0,
                        prompt_size_chars: 0,
                        summary_size_chars: 0,
                        total_tokens: None,
                        total_tool_calls: None,
                        duration_ms: None,
                        estimated_input_tokens: 0,
                        estimated_output_tokens: 0,
                        estimated_cost_usd: 0.0,
                        confidence: "estimated".to_string(),
                    });
                },

                "system.notification" => {
                    let kind = evt
                        .data
                        .get("kind")
                        .and_then(Value::as_str)
                        .unwrap_or("notification")
                        .to_string();
                    let output_chars = evt
                        .data
                        .get("content")
                        .and_then(Value::as_str)
                        .map(|s| s.len() as i64)
                        .unwrap_or(0);
                    interaction_events.push(InteractionEvent {
                        id: stable_event_id(
                            &session_id,
                            "notification",
                            &kind,
                            evt.id.as_deref(),
                            ts_ms,
                        ),
                        session_id: session_id.clone(),
                        turn_id: active_turn_id.clone(),
                        task_run_id: None,
                        correlation_id: None,
                        parent_id: None,
                        provider: "copilot-cli".to_string(),
                        timestamp: ts_ms,
                        kind: "notification".to_string(),
                        phase: "emitted".to_string(),
                        name: kind,
                        display_name: None,
                        mcp_server: None,
                        mcp_tool: None,
                        hook_type: None,
                        agent_type: None,
                        execution_mode: None,
                        model: None,
                        status: Some("emitted".to_string()),
                        success: Some(true),
                        input_size_chars: 0,
                        output_size_chars: output_chars,
                        prompt_size_chars: 0,
                        summary_size_chars: 0,
                        total_tokens: None,
                        total_tool_calls: None,
                        duration_ms: None,
                        estimated_input_tokens: 0,
                        estimated_output_tokens: output_chars / 4,
                        estimated_cost_usd: 0.0,
                        confidence: "estimated".to_string(),
                    });
                },

                "assistant.turn_end" => {
                    let turn_id = evt
                        .data
                        .get("turnId")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if let Some(turn) = pending_turns.get_mut(&turn_id) {
                        turn.end_ms = ts_ms;
                        turn.model = Some(current_model.clone());
                    }
                    active_turn_id = None;
                },

                "session.compaction_complete" => {
                    let used = evt.data.get("compactionTokensUsed");
                    if let Some(used) = used {
                        let input = used.get("input").and_then(Value::as_i64).unwrap_or(0);
                        let output = used.get("output").and_then(Value::as_i64).unwrap_or(0);
                        let cached = used.get("cachedInput").and_then(Value::as_i64).unwrap_or(0);
                        let comp_id = format!("compaction-{}", evt.id.as_deref().unwrap_or("0"));
                        pending_turns.insert(
                            comp_id.clone(),
                            PendingTurn {
                                id: comp_id,
                                start_ms: ts_ms,
                                end_ms: ts_ms,
                                context_tokens: input,
                                token_limit: session_token_limit.unwrap_or(200_000),
                                output_tokens: output,
                                thinking_tokens: 0,
                                input_tokens: input,
                                cache_read_tokens: cached,
                                cache_write_tokens: 0,
                                model: Some(current_model.clone()),
                                compaction_input: input,
                                compaction_output: output,
                                compaction_cached: cached,
                                is_compaction: true,
                            },
                        );

                        let total_tokens = input + output + cached;
                        let summary_chars = evt
                            .data
                            .get("summaryContent")
                            .and_then(Value::as_str)
                            .map(|s| s.len() as i64)
                            .unwrap_or(0);
                        interaction_events.push(InteractionEvent {
                            id: stable_event_id(
                                &session_id,
                                "compaction",
                                evt.id.as_deref().unwrap_or("compaction"),
                                evt.id.as_deref(),
                                ts_ms,
                            ),
                            session_id: session_id.clone(),
                            turn_id: None,
                            task_run_id: None,
                            correlation_id: None,
                            parent_id: None,
                            provider: "copilot-cli".to_string(),
                            timestamp: ts_ms,
                            kind: "compaction".to_string(),
                            phase: "complete".to_string(),
                            name: "compaction".to_string(),
                            display_name: None,
                            mcp_server: None,
                            mcp_tool: None,
                            hook_type: None,
                            agent_type: None,
                            execution_mode: None,
                            model: Some(current_model.clone()),
                            status: Some("completed".to_string()),
                            success: evt.data.get("success").and_then(Value::as_bool),
                            input_size_chars: 0,
                            output_size_chars: 0,
                            prompt_size_chars: 0,
                            summary_size_chars: summary_chars,
                            total_tokens: Some(total_tokens),
                            total_tool_calls: None,
                            duration_ms: None,
                            estimated_input_tokens: input,
                            estimated_output_tokens: output,
                            estimated_cost_usd: 0.0,
                            confidence: "exact".to_string(),
                        });
                    }
                },

                _ => {},
            }
        }

        let Some(sess) = session else {
            return Ok(0);
        };

        let last_turn_at = pending_turns
            .values()
            .map(|t| t.end_ms.max(t.start_ms))
            .max()
            .unwrap_or(sess.started_at);

        let db_session = Session {
            id: sess.id.clone(),
            project: sess.project_path.clone(),
            project_name: sess.project_name.clone(),
            slug: sess.project_name.to_lowercase().replace(' ', "-"),
            provider: "copilot-cli".to_string(),
            provider_version: sess.provider_version.clone(),
            model: current_model.clone(),
            git_branch: sess.branch.clone(),
            started_at: sess.started_at,
            last_turn_at,
            total_turns: pending_turns.len() as i64,
            is_subagent: false,
            parent_session_id: None,
            context_window_tokens: session_token_limit,
        };

        db.upsert_session(&db_session)?;

        let mut sorted_turns: Vec<_> = pending_turns.values().cloned().collect();
        sorted_turns.sort_by_key(|t| t.start_ms);

        let mut raw_turn_to_db_id: HashMap<String, String> = HashMap::new();
        for (idx, t) in sorted_turns.iter().enumerate() {
            let tool_count = turn_tool_counts.get(&t.id).copied().unwrap_or(0);
            let duration_ms = if t.end_ms > t.start_ms {
                Some(t.end_ms - t.start_ms)
            } else {
                None
            };
            let (input_tokens, cache_read, cache_write, output_tokens, thinking_tokens) =
                if t.is_compaction {
                    (
                        t.compaction_input,
                        t.compaction_cached,
                        0,
                        t.compaction_output,
                        0,
                    )
                } else {
                    // Prefer accurate context_tokens from truncation events over estimated input_tokens
                    let input = if t.context_tokens > 0 {
                        t.context_tokens
                    } else {
                        t.input_tokens
                    };
                    (
                        input,
                        t.cache_read_tokens,
                        t.cache_write_tokens,
                        t.output_tokens,
                        t.thinking_tokens,
                    )
                };
            let turn_id = format!("{}-turn-{}", sess.id, idx);
            raw_turn_to_db_id.insert(t.id.clone(), turn_id.clone());

            let turn_model = t.model.clone().unwrap_or_else(|| current_model.clone());
            let cost = calculate_turn_cost(
                &turn_model,
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
                thinking_tokens,
                mcp_call_count: tool_count,
                mcp_input_token_est: 0,
                text_output_tokens: output_tokens,
                model: turn_model,
                service_tier: "default".to_string(),
                estimated_cost_usd: cost,
                is_compaction_event: t.is_compaction,
            };

            if db.upsert_turn(&turn).is_ok() {
                new_turns += 1;
            }
        }

        for tc in &mut pending_tool_calls {
            if let Some(db_turn_id) = raw_turn_to_db_id.get(&tc.turn_id).cloned() {
                tc.turn_id = db_turn_id;
                tc.session_id = sess.id.clone();
                let _ = db.upsert_tool_call(tc);
            }
        }

        for event in &mut interaction_events {
            event.session_id = sess.id.clone();
            if let Some(raw_turn_id) = event.turn_id.clone() {
                event.turn_id = raw_turn_to_db_id.get(&raw_turn_id).cloned();
            }
            if let Some(task_run_id) = &event.task_run_id {
                if let Some(task) = task_runs.get(task_run_id) {
                    event.task_run_id = Some(task.id.clone());
                }
            }
            let _ = db.upsert_interaction_event(event);
        }

        for task in task_runs.values() {
            let _ = db.upsert_task_run(task);
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

#[derive(Debug, Clone)]
struct CopilotSession {
    id: String,
    project_path: String,
    project_name: String,
    branch: String,
    started_at: i64,
    provider_version: String,
}

#[derive(Debug, Clone)]
struct PendingTurn {
    id: String,
    start_ms: i64,
    end_ms: i64,
    context_tokens: i64,
    token_limit: i64,
    output_tokens: i64,
    thinking_tokens: i64,
    input_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    model: Option<String>,
    compaction_input: i64,
    compaction_output: i64,
    compaction_cached: i64,
    is_compaction: bool,
}

#[derive(Debug, Clone)]
struct ToolExecutionStart {
    tool_name: String,
    mcp_server: Option<String>,
    mcp_tool: Option<String>,
    input_chars: i64,
    args: Value,
    turn_id: Option<String>,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn parse_iso_ms(s: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(0)
}

fn fallback_session_id(path: &Path) -> String {
    if path.file_name().and_then(|n| n.to_str()) == Some("events.jsonl") {
        let id = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("session");
        format!("copilot-{id}")
    } else {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("session");
        format!("copilot-{stem}")
    }
}

fn json_chars(value: &Value) -> i64 {
    match value {
        Value::Null => 0,
        _ => value.to_string().len() as i64,
    }
}

fn stable_event_id(
    session_id: &str,
    prefix: &str,
    correlation: &str,
    raw_id: Option<&str>,
    timestamp: i64,
) -> String {
    if let Some(raw) = raw_id {
        if !raw.is_empty() {
            return format!("{session_id}-{prefix}-{raw}");
        }
    }
    if !correlation.is_empty() {
        return format!("{session_id}-{prefix}-{correlation}");
    }
    format!("{session_id}-{prefix}-{timestamp}")
}

fn non_empty(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}

fn interaction_kind_and_name(
    tool_name: &str,
    mcp_server: Option<&str>,
    mcp_tool: Option<&str>,
) -> (&'static str, String) {
    if let Some(tool) = mcp_tool {
        let name = if let Some(server) = mcp_server {
            format!("{server}::{tool}")
        } else {
            tool.to_string()
        };
        ("mcp", name)
    } else if tool_name == "task" {
        ("task", "task".to_string())
    } else if tool_name == "skill" {
        ("skill", "skill".to_string())
    } else {
        ("tool", tool_name.to_string())
    }
}

fn display_name_for_tool(
    tool_name: &str,
    mcp_server: Option<&str>,
    mcp_tool: Option<&str>,
) -> String {
    if let Some(tool) = mcp_tool {
        if let Some(server) = mcp_server {
            return format!("{server}::{tool}");
        }
        return tool.to_string();
    }
    tool_name.to_string()
}

fn metric_i64(metrics: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter()
        .find_map(|key| metrics.get(*key).and_then(Value::as_i64))
}

fn output_chars_from_tool_complete(data: &Value) -> i64 {
    let telemetry_len = data
        .get("toolTelemetry")
        .and_then(|t| t.get("metrics"))
        .and_then(|m| {
            metric_i64(
                m,
                &[
                    "resultForLlmLength",
                    "resultLength",
                    "result_length_original",
                    "result_length",
                    "response_length",
                ],
            )
        })
        .unwrap_or(0);
    let result_len = data.get("result").map(json_chars).unwrap_or(0);
    telemetry_len.max(result_len)
}
