use super::Provider;
use crate::watcher;
use anyhow::Result;
use scopeon_core::Database;
use std::path::PathBuf;

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claude_config_dir_env_var() {
        // When CLAUDE_CONFIG_DIR is set, it should be used instead of ~/.claude
        std::env::set_var("CLAUDE_CONFIG_DIR", "/custom/claude");
        let provider = ClaudeCodeProvider::new();
        assert_eq!(
            provider.projects_dir,
            PathBuf::from("/custom/claude/projects")
        );
        std::env::remove_var("CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn test_default_path_when_no_env() {
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

pub(crate) fn walkdir_jsonl(dir: &std::path::Path) -> Vec<PathBuf> {
    walkdir_jsonl_inner(dir, 0)
}

fn walkdir_jsonl_inner(dir: &std::path::Path, depth: u32) -> Vec<PathBuf> {
    const MAX_DEPTH: u32 = 8;
    let mut results = Vec::new();
    if depth >= MAX_DEPTH {
        return results;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            let path = entry.path();
            if ft.is_dir() {
                results.extend(walkdir_jsonl_inner(&path, depth + 1));
            } else if path.extension().map(|e| e == "jsonl").unwrap_or(false) {
                results.push(path);
            }
        }
    }
    results
}
