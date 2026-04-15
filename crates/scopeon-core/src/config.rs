//! Runtime configuration for Scopeon.
//!
//! Resolves paths for the SQLite database and Claude Code projects directory,
//! and creates the `~/.scopeon/` directory if it does not exist.

use anyhow::Result;
use std::path::PathBuf;

/// Runtime configuration loaded from the user's environment.
#[derive(Debug, Clone)]
pub struct Config {
    /// Path to `~/.scopeon/scopeon.db`
    pub db_path: PathBuf,
    /// Path to `~/.claude/projects/` where Claude Code writes session JSONL logs
    pub claude_projects_dir: PathBuf,
}

impl Config {
    /// Load configuration from the user's environment.
    ///
    /// Creates `~/.scopeon/` if it does not already exist.
    pub fn load() -> Result<Self> {
        let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot find home directory"))?;
        let db_path = home.join(".scopeon").join("scopeon.db");
        let claude_projects_dir = home.join(".claude").join("projects");

        std::fs::create_dir_all(db_path.parent().unwrap())?;

        Ok(Config {
            db_path,
            claude_projects_dir,
        })
    }
}
