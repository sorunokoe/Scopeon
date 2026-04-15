use super::Provider;
use anyhow::Result;
use scopeon_core::{Database, Session, Turn};
use std::path::PathBuf;

type OllamaMessage = (
    String,
    String,
    String,
    Option<String>,
    String,
    i64,
    Option<i64>,
    Option<i64>,
);

pub struct OllamaProvider {
    db_path: PathBuf,
}

impl OllamaProvider {
    pub fn new() -> Self {
        let db_path = dirs::home_dir()
            .map(|h| {
                h.join("Library")
                    .join("Application Support")
                    .join("Ollama")
                    .join("db.sqlite")
            })
            .unwrap_or_else(|| PathBuf::from("/nonexistent"));
        OllamaProvider { db_path }
    }
}

impl Default for OllamaProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for OllamaProvider {
    fn id(&self) -> &str {
        "ollama"
    }
    fn name(&self) -> &str {
        "Ollama"
    }
    fn description(&self) -> &str {
        "Reads Ollama local model chat history"
    }
    fn is_available(&self) -> bool {
        self.db_path.exists()
    }
    fn watch_paths(&self) -> Vec<PathBuf> {
        vec![self.db_path.clone()]
    }

    fn scan(&self, db: &Database) -> Result<usize> {
        if !self.is_available() {
            return Ok(0);
        }

        let conn = rusqlite::Connection::open(&self.db_path)?;

        let mut stmt =
            conn.prepare("SELECT id, title, created_at FROM chats ORDER BY created_at")?;

        let chats: Vec<(String, String, i64)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;

        let mut count = 0;
        for (chat_id, title, created_at) in chats {
            let mut msg_stmt = conn.prepare(
                "SELECT id, role, content, thinking, model_name, created_at, thinking_time_start, thinking_time_end
                 FROM messages WHERE chat_id = ?1 ORDER BY created_at"
            )?;

            let messages: Vec<OllamaMessage> = msg_stmt
                .query_map([&chat_id], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                })?
                .collect::<rusqlite::Result<_>>()?;

            if messages.is_empty() {
                continue;
            }

            let session = Session {
                id: format!("ollama-{}", chat_id),
                project: "ollama".to_string(),
                project_name: title.clone(),
                slug: slug_from_title(&title),
                model: messages
                    .iter()
                    .find(|m| m.1 == "assistant")
                    .map(|m| m.4.clone())
                    .unwrap_or_default(),
                git_branch: String::new(),
                started_at: created_at,
                last_turn_at: messages.last().map(|m| m.5).unwrap_or(created_at),
                total_turns: messages.iter().filter(|m| m.1 == "assistant").count() as i64,
                is_subagent: false,
                parent_session_id: None,
                context_window_tokens: None,
            };
            db.upsert_session(&session)?;

            for (idx, msg) in messages
                .iter()
                .enumerate()
                .filter(|(_, m)| m.1 == "assistant")
            {
                let (id, _, content, thinking, model, ts, ts_start, ts_end) = msg;
                let thinking_text = thinking.as_deref().unwrap_or("");
                let thinking_tokens = (thinking_text.len() / 4) as i64;
                let content_tokens = (content.len() / 4) as i64;
                let output_tokens = thinking_tokens + content_tokens;
                let duration_ms = match (ts_start, ts_end) {
                    (Some(s), Some(e)) => Some((e - s).abs()),
                    _ => None,
                };

                let turn = Turn {
                    id: format!("ollama-{}", id),
                    session_id: format!("ollama-{}", chat_id),
                    turn_index: idx as i64,
                    timestamp: *ts,
                    duration_ms,
                    input_tokens: 0,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                    cache_write_5m_tokens: 0,
                    cache_write_1h_tokens: 0,
                    output_tokens,
                    thinking_tokens,
                    mcp_call_count: 0,
                    mcp_input_token_est: 0,
                    text_output_tokens: content_tokens,
                    model: model.clone(),
                    service_tier: "local".to_string(),
                    estimated_cost_usd: 0.0,
                    is_compaction_event: false,
                };
                db.upsert_turn(&turn)?;
                count += 1;
            }
        }
        Ok(count)
    }
}

fn slug_from_title(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}
