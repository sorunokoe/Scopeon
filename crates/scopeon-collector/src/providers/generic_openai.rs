use super::Provider;
use anyhow::Result;
use scopeon_core::{Database, Session, Turn};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
struct OpenAICompletion {
    id: Option<String>,
    model: Option<String>,
    usage: Option<OpenAIUsage>,
    choices: Option<Vec<OpenAIChoice>>,
}

#[derive(Deserialize)]
struct OpenAIUsage {
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
}

#[derive(Deserialize)]
struct OpenAIChoice {
    message: Option<OpenAIMessage>,
}

#[derive(Deserialize)]
struct OpenAIMessage {
    role: Option<String>,
}

pub struct GenericOpenAIProvider {
    paths: Vec<PathBuf>,
    name: String,
}

impl GenericOpenAIProvider {
    pub fn new(paths: Vec<String>, name: String) -> Self {
        let mut resolved: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        if let Some(home) = dirs::home_dir() {
            let codex_path = home.join(".codex").join("sessions");
            if codex_path.exists() && !resolved.contains(&codex_path) {
                resolved.push(codex_path);
            }
        }
        GenericOpenAIProvider {
            paths: resolved,
            name,
        }
    }
}

impl Provider for GenericOpenAIProvider {
    fn id(&self) -> &str {
        "generic"
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        "Reads JSONL files in OpenAI API format (Codex CLI, etc.)"
    }
    fn is_available(&self) -> bool {
        self.paths.iter().any(|p| p.exists())
    }
    fn watch_paths(&self) -> Vec<PathBuf> {
        self.paths.clone()
    }

    fn scan(&self, db: &Database) -> Result<usize> {
        let mut total = 0;
        for base_path in &self.paths {
            if !base_path.exists() {
                continue;
            }
            for file in collect_jsonl(base_path) {
                total += scan_openai_file(&file, db)?;
            }
        }
        Ok(total)
    }
}

fn collect_jsonl(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut results = Vec::new();
    if dir.is_file() && dir.extension().map(|e| e == "jsonl").unwrap_or(false) {
        results.push(dir.to_path_buf());
        return results;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                results.extend(collect_jsonl(&path));
            } else if path.extension().map(|e| e == "jsonl").unwrap_or(false) {
                results.push(path);
            }
        }
    }
    results
}

fn scan_openai_file(file: &std::path::Path, db: &Database) -> Result<usize> {
    use std::io::{BufRead, BufReader};
    let f = std::fs::File::open(file)?;
    let reader = BufReader::new(f);

    let session_id = format!(
        "generic-{}",
        file.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
    );
    let file_name = file
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let mut turns = Vec::new();
    let mut turn_index = 0i64;
    let mut first_ts = 0i64;
    let mut last_ts = 0i64;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let Ok(completion) = serde_json::from_str::<OpenAICompletion>(trimmed) else {
            continue;
        };

        let usage = match &completion.usage {
            Some(u) => u,
            None => continue,
        };
        let is_assistant = completion
            .choices
            .as_ref()
            .and_then(|c| c.first())
            .and_then(|c| c.message.as_ref())
            .and_then(|m| m.role.as_deref())
            .map(|r| r == "assistant")
            .unwrap_or(false);
        if !is_assistant {
            continue;
        }

        let ts = now_ms + turn_index;
        if first_ts == 0 {
            first_ts = ts;
        }
        last_ts = ts;

        let turn = Turn {
            id: format!(
                "{}-{}",
                session_id,
                completion.id.as_deref().unwrap_or(&turn_index.to_string())
            ),
            session_id: session_id.clone(),
            turn_index,
            timestamp: ts,
            duration_ms: None,
            input_tokens: usage.prompt_tokens.unwrap_or(0),
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cache_write_5m_tokens: 0,
            cache_write_1h_tokens: 0,
            output_tokens: usage.completion_tokens.unwrap_or(0),
            thinking_tokens: 0,
            mcp_call_count: 0,
            mcp_input_token_est: 0,
            text_output_tokens: usage.completion_tokens.unwrap_or(0),
            model: completion.model.clone().unwrap_or_default(),
            service_tier: "standard".to_string(),
            estimated_cost_usd: 0.0,
            is_compaction_event: false,
        };
        turns.push(turn);
        turn_index += 1;
    }

    if turns.is_empty() {
        return Ok(0);
    }

    let session = Session {
        id: session_id.clone(),
        project: "generic".to_string(),
        project_name: file_name.clone(),
        slug: file_name.trim_end_matches(".jsonl").to_string(),
        model: turns.first().map(|t| t.model.clone()).unwrap_or_default(),
        git_branch: String::new(),
        started_at: first_ts,
        last_turn_at: last_ts,
        total_turns: turns.len() as i64,
        is_subagent: false,
        parent_session_id: None,
        context_window_tokens: None,
    };
    db.upsert_session(&session)?;
    let n = turns.len();
    for turn in turns {
        db.upsert_turn(&turn)?;
    }
    Ok(n)
}
