//! Aider AI pair programmer provider.
//!
//! Reads Aider's analytics JSONL log (enabled via `--analytics-log`).
//! Default path: `~/.aider/analytics.jsonl`
//! Override via `AIDER_ANALYTICS_LOG` env var or Scopeon config.
//!
//! # Aider log format (one JSON object per line)
//! ```json
//! {"event":"message_send","properties":{"main_model":"gpt-4o","prompt_tokens":1234,
//!  "completion_tokens":56,"total_tokens":1290,"cost":0.003,"total_cost":0.045},
//!  "user_id":"uuid","time":1700000000}
//! ```

use std::path::PathBuf;

use anyhow::Result;
use serde::Deserialize;

use scopeon_core::{Database, Session, Turn};

use super::Provider;

pub struct AiderProvider {
    pub log_path: PathBuf,
}

impl AiderProvider {
    pub fn new(log_path: Option<PathBuf>) -> Self {
        let path = log_path
            .or_else(|| std::env::var("AIDER_ANALYTICS_LOG").ok().map(PathBuf::from))
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_default()
                    .join(".aider")
                    .join("analytics.jsonl")
            });
        Self { log_path: path }
    }
}

#[derive(Deserialize)]
struct AiderEntry {
    event: String,
    properties: Option<AiderProperties>,
    user_id: Option<String>,
    time: Option<i64>,
}

#[derive(Deserialize)]
struct AiderProperties {
    main_model: Option<String>,
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
    cost: Option<f64>,
}

impl Provider for AiderProvider {
    fn id(&self) -> &str {
        "aider"
    }
    fn name(&self) -> &str {
        "Aider"
    }
    fn description(&self) -> &str {
        "Aider AI pair programmer (reads --analytics-log JSONL)"
    }

    fn is_available(&self) -> bool {
        self.log_path.exists()
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        if self.log_path.exists() {
            vec![self.log_path.clone()]
        } else {
            vec![]
        }
    }

    fn scan(&self, db: &Database) -> Result<usize> {
        if !self.log_path.exists() {
            return Ok(0);
        }

        let offset = db.get_file_offset(self.log_path.to_str().unwrap_or(""))?;
        let content = std::fs::read_to_string(&self.log_path)?;

        // Parse all lines, skipping those before our tracked byte offset
        let mut byte_pos: usize = 0;
        let mut entries: Vec<(i64, AiderProperties, String)> = vec![];

        for line in content.lines() {
            let line_start = byte_pos;
            byte_pos += line.len() + 1;

            if (line_start as u64) < offset {
                continue;
            }
            if line.trim().is_empty() {
                continue;
            }

            if let Ok(entry) = serde_json::from_str::<AiderEntry>(line) {
                if entry.event == "message_send" {
                    if let (Some(props), Some(ts)) = (entry.properties, entry.time) {
                        let user_id = entry.user_id.unwrap_or_else(|| "aider-user".to_string());
                        entries.push((ts, props, user_id));
                    }
                }
            }
        }

        if entries.is_empty() {
            return Ok(0);
        }

        // Group consecutive entries into sessions (gap > 30 minutes = new session)
        let session_gap_secs = 30 * 60_i64;
        let mut sessions: Vec<Vec<(i64, AiderProperties, String)>> = vec![];
        let mut current: Vec<(i64, AiderProperties, String)> = vec![];

        for entry in entries {
            if let Some(last) = current.last() {
                if entry.0 - last.0 > session_gap_secs {
                    sessions.push(std::mem::take(&mut current));
                }
            }
            current.push(entry);
        }
        if !current.is_empty() {
            sessions.push(current);
        }

        let mut new_turns = 0usize;

        for session_entries in &sessions {
            let first = &session_entries[0];
            let last = &session_entries[session_entries.len() - 1];
            let session_id = format!("aider-{}-{}", first.2, first.0);
            let model = first
                .1
                .main_model
                .clone()
                .unwrap_or_else(|| "unknown".to_string());

            let session = Session {
                id: session_id.clone(),
                project: dirs::home_dir()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
                project_name: "aider".to_string(),
                slug: format!("aider-{}", first.0),
                model: model.clone(),
                git_branch: String::new(),
                started_at: first.0 * 1000,
                last_turn_at: last.0 * 1000,
                total_turns: session_entries.len() as i64,
                is_subagent: false,
                parent_session_id: None,
                context_window_tokens: None,
            };
            let _ = db.upsert_session(&session);

            for (turn_idx, (ts, props, _)) in session_entries.iter().enumerate() {
                let input = props.prompt_tokens.unwrap_or(0);
                let output = props.completion_tokens.unwrap_or(0);
                let cost = props.cost.unwrap_or(0.0);
                let turn_model = props.main_model.clone().unwrap_or_else(|| model.clone());

                let turn = Turn {
                    id: format!("{}-t{}", session_id, turn_idx),
                    session_id: session_id.clone(),
                    turn_index: turn_idx as i64,
                    timestamp: ts * 1000,
                    duration_ms: None,
                    input_tokens: input,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                    cache_write_5m_tokens: 0,
                    cache_write_1h_tokens: 0,
                    output_tokens: output,
                    thinking_tokens: 0,
                    mcp_call_count: 0,
                    mcp_input_token_est: 0,
                    text_output_tokens: output,
                    model: turn_model,
                    service_tier: String::new(),
                    estimated_cost_usd: cost,
                    is_compaction_event: false,
                };
                if db.upsert_turn(&turn).is_ok() {
                    new_turns += 1;
                }
            }
        }

        // Update file offset
        let new_offset = content.len() as u64;
        db.set_file_offset(
            self.log_path.to_str().unwrap_or(""),
            new_offset,
            std::fs::metadata(&self.log_path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
        )?;

        Ok(new_turns)
    }
}
