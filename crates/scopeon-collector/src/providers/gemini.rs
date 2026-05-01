//! Google Gemini CLI provider.
//!
//! Reads Gemini CLI session JSONL files stored in `~/.gemini/tmp/<project-hash>/`.
//! Each session file is a JSONL where every line is a `MessageRecord`.
//!
//! # Session file format (one JSON object per line)
//! ```json
//! {"id":"...","timestamp":"2025-01-01T00:00:00.000Z","type":"gemini",
//!  "content":"...","model":"gemini-2.5-pro",
//!  "tokens":{"input":1234,"output":56,"cached":100,"thoughts":200,"tool":50,"total":1540},
//!  "toolCalls":[]}
//! ```
//!
//! Only lines with `type == "gemini"` and a `tokens` field have token data.

use std::path::PathBuf;

use anyhow::Result;
use serde::Deserialize;

use scopeon_core::{Database, Session, Turn};

use super::Provider;

pub struct GeminiCLIProvider {
    /// Root directory to scan, defaults to `~/.gemini/tmp`.
    pub root: PathBuf,
}

impl GeminiCLIProvider {
    pub fn new(root: Option<PathBuf>) -> Self {
        let r = root.unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_default()
                .join(".gemini")
                .join("tmp")
        });
        Self { root: r }
    }

    fn session_files(&self) -> Vec<PathBuf> {
        let mut files = vec![];
        if !self.root.is_dir() {
            return files;
        }
        // ~/.gemini/tmp/<project-hash>/session-*.jsonl
        if let Ok(entries) = std::fs::read_dir(&self.root) {
            for entry in entries.flatten() {
                let hash_dir = entry.path();
                if hash_dir.is_dir() {
                    if let Ok(sub) = std::fs::read_dir(&hash_dir) {
                        for f in sub.flatten() {
                            let p = f.path();
                            if p.extension().map(|e| e == "jsonl").unwrap_or(false)
                                && p.file_name()
                                    .and_then(|n| n.to_str())
                                    .map(|n| n.starts_with("session-"))
                                    .unwrap_or(false)
                            {
                                files.push(p);
                            }
                        }
                    }
                }
            }
        }
        files
    }
}

#[derive(Deserialize)]
struct MessageRecord {
    id: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "type")]
    msg_type: Option<String>,
    model: Option<String>,
    tokens: Option<TokensSummary>,
    #[serde(rename = "toolCalls")]
    tool_calls: Option<Vec<serde_json::Value>>,
}

#[derive(Deserialize)]
struct TokensSummary {
    input: Option<i64>,
    output: Option<i64>,
    cached: Option<i64>,
    thoughts: Option<i64>,
}

impl Provider for GeminiCLIProvider {
    fn id(&self) -> &str {
        "gemini-cli"
    }
    fn name(&self) -> &str {
        "Gemini CLI"
    }
    fn description(&self) -> &str {
        "Google Gemini CLI (reads ~/.gemini/tmp session JSONL)"
    }

    fn is_available(&self) -> bool {
        self.root.exists()
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        if self.root.exists() {
            vec![self.root.clone()]
        } else {
            vec![]
        }
    }

    fn scan(&self, db: &Database) -> Result<usize> {
        let files = self.session_files();
        if files.is_empty() {
            return Ok(0);
        }

        let mut total_new = 0usize;

        for path in &files {
            let path_str = path.to_str().unwrap_or("");
            let offset = db.get_file_offset(path_str)?;

            let content = std::fs::read_to_string(path)?;
            let mut byte_pos: usize = 0;
            let mut records: Vec<(usize, MessageRecord)> = vec![];

            for line in content.lines() {
                let line_start = byte_pos;
                byte_pos += line.len() + 1;
                if (line_start as u64) < offset {
                    continue;
                }
                if line.trim().is_empty() {
                    continue;
                }

                if let Ok(rec) = serde_json::from_str::<MessageRecord>(line) {
                    records.push((line_start, rec));
                }
            }

            // Extract meaningful records (type == gemini with tokens)
            let gemini_records: Vec<&(usize, MessageRecord)> = records
                .iter()
                .filter(|(_, r)| r.msg_type.as_deref() == Some("gemini") && r.tokens.is_some())
                .collect();

            if gemini_records.is_empty() {
                continue;
            }

            // Derive session_id from file name (session-<id>.jsonl)
            let file_stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("session-unknown");
            let session_id = format!("gemini-{file_stem}");

            // Derive project from parent directory name (project hash)
            let project_hash = path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");

            let first = &gemini_records[0].1;
            let last = &gemini_records[gemini_records.len() - 1].1;
            let model = first
                .model
                .clone()
                .unwrap_or_else(|| "gemini-2.5-pro".to_string());

            fn parse_ts(s: &str) -> i64 {
                chrono::DateTime::parse_from_rfc3339(s)
                    .map(|dt| dt.timestamp_millis())
                    .unwrap_or(0)
            }

            let started_at = first.timestamp.as_deref().map(parse_ts).unwrap_or(0);
            let last_turn_at = last
                .timestamp
                .as_deref()
                .map(parse_ts)
                .unwrap_or(started_at);

            let session = Session {
                id: session_id.clone(),
                project: self.root.to_string_lossy().to_string(),
                project_name: format!("gemini-{project_hash}"),
                slug: file_stem.to_string(),
                provider: "gemini-cli".to_string(),
                provider_version: String::new(),
                model: model.clone(),
                git_branch: String::new(),
                started_at,
                last_turn_at,
                total_turns: gemini_records.len() as i64,
                is_subagent: false,
                parent_session_id: None,
                context_window_tokens: None,
            };
            let _ = db.upsert_session(&session);

            for (turn_idx, (_, rec)) in gemini_records.iter().enumerate() {
                let Some(tok) = rec.tokens.as_ref() else {
                    continue;
                };
                let input = tok.input.unwrap_or(0);
                let output = tok.output.unwrap_or(0);
                let cached = tok.cached.unwrap_or(0);
                let thoughts = tok.thoughts.unwrap_or(0);
                let ts = rec.timestamp.as_deref().map(parse_ts).unwrap_or(started_at);
                let turn_model = rec.model.clone().unwrap_or_else(|| model.clone());
                let mcp_count = rec.tool_calls.as_ref().map(|v| v.len() as i64).unwrap_or(0);
                let rec_id = rec
                    .id
                    .clone()
                    .unwrap_or_else(|| format!("{session_id}-t{turn_idx}"));

                let cost =
                    scopeon_core::cost::calculate_turn_cost(&turn_model, input, output, 0, cached)
                        .total_usd;

                let turn = Turn {
                    id: format!("gemini-{rec_id}"),
                    session_id: session_id.clone(),
                    turn_index: turn_idx as i64,
                    timestamp: ts,
                    duration_ms: None,
                    input_tokens: input,
                    cache_read_tokens: cached,
                    cache_write_tokens: 0,
                    cache_write_5m_tokens: 0,
                    cache_write_1h_tokens: 0,
                    output_tokens: output,
                    thinking_tokens: thoughts,
                    mcp_call_count: mcp_count,
                    mcp_input_token_est: 0,
                    text_output_tokens: output.saturating_sub(thoughts),
                    model: turn_model,
                    service_tier: String::new(),
                    estimated_cost_usd: cost,
                    is_compaction_event: false,
                };
                if db.upsert_turn(&turn).is_ok() {
                    total_new += 1;
                }
            }

            let new_offset = content.len() as u64;
            let _ = db.set_file_offset(
                path_str,
                new_offset,
                std::fs::metadata(path)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
                    .unwrap_or(0),
            );
        }

        Ok(total_new)
    }
}
