use super::Provider;
use crate::watcher;
use anyhow::Result;
use scopeon_core::Database;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub struct ClaudeCodeProvider {
    pub projects_dir: PathBuf,
}

impl ClaudeCodeProvider {
    pub fn new() -> Self {
        // Priority: CLAUDE_CONFIG_DIR env var → ~/.claude → /nonexistent
        let dir = std::env::var("CLAUDE_CONFIG_DIR")
            .ok()
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|h| h.join(".claude")))
            .unwrap_or_else(|| PathBuf::from("/nonexistent"))
            .join("projects");
        ClaudeCodeProvider { projects_dir: dir }
    }
}

impl Default for ClaudeCodeProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for ClaudeCodeProvider {
    fn id(&self) -> &str {
        "claude-code"
    }
    fn name(&self) -> &str {
        "Claude Code"
    }
    fn description(&self) -> &str {
        "Reads Claude Code JSONL session logs from $CLAUDE_CONFIG_DIR/projects/ (or ~/.claude/projects/)"
    }
    fn is_available(&self) -> bool {
        self.projects_dir.exists()
    }
    fn watch_paths(&self) -> Vec<PathBuf> {
        vec![self.projects_dir.clone()]
    }
    fn scan(&self, db: &Database) -> Result<usize> {
        if !self.is_available() {
            return Ok(0);
        }
        let mut count = 0;
        for entry in walkdir_jsonl(&self.projects_dir) {
            watcher::process_file(&entry, db)?;
            count += 1;
        }
        Ok(count)
    }

    /// Override to release the DB mutex between files.
    ///
    /// With 31+ large JSONL files the default one-shot lock would block the
    /// TUI for the entire backfill duration.  Releasing between files lets
    /// the TUI refresh in the gaps.
    fn scan_incremental(&self, db: Arc<Mutex<Database>>) -> Result<usize> {
        if !self.is_available() {
            return Ok(0);
        }
        let mut count = 0;
        for entry in walkdir_jsonl(&self.projects_dir) {
            {
                let db_guard = db
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Database mutex poisoned"))?;
                watcher::process_file(&entry, &db_guard)?;
            } // lock released here — TUI can refresh before next file
            count += 1;
        }
        Ok(count)
    }
}

pub(crate) fn walkdir_jsonl(dir: &std::path::Path) -> Vec<PathBuf> {
    walkdir_jsonl_inner(dir, 0)
}

fn walkdir_jsonl_inner(dir: &std::path::Path, depth: u32) -> Vec<PathBuf> {
    const MAX_DEPTH: u32 = 8;
    if depth >= MAX_DEPTH || !dir.is_dir() {
        return vec![];
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut out = vec![];
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
            out.push(path);
        } else if path.is_dir() {
            out.extend(walkdir_jsonl_inner(&path, depth + 1));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Env-var tests mutate process-wide state; serialize them to avoid races.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_claude_config_dir_env_var() {
        let _guard = ENV_LOCK.lock().unwrap();
        // Use a temp-dir path so the test works on all platforms.
        let custom_dir = std::env::temp_dir().join("scopeon_test_claude");
        std::env::set_var("CLAUDE_CONFIG_DIR", custom_dir.to_str().unwrap());
        let provider = ClaudeCodeProvider::new();
        assert_eq!(provider.projects_dir, custom_dir.join("projects"));
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn test_default_path_when_no_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        let provider = ClaudeCodeProvider::new();
        // Should end with .claude/projects
        let path_str = provider.projects_dir.to_string_lossy();
        assert!(
            path_str.contains(".claude") && path_str.ends_with("projects"),
            "Expected path ending in .claude/projects, got: {path_str}"
        );
    }
}
