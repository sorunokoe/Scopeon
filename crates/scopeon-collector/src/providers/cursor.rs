//! Cursor AI editor detection provider.
//!
//! Detects the Cursor editor installation but **cannot** read token data.
//! Cursor is built on VS Code and its conversation state is stored in the same
//! binary LevelDB/IndexedDB format as VS Code — not directly readable.
//!
//! This provider appears in the Providers tab to inform users that Cursor is detected.

use super::Provider;
use anyhow::Result;
use scopeon_core::Database;
use std::path::PathBuf;

pub struct CursorProvider;

impl CursorProvider {
    pub fn new() -> Self {
        Self
    }

    fn detection_paths() -> Vec<PathBuf> {
        let base = dirs::home_dir().unwrap_or_default();
        vec![
            // macOS app bundle
            PathBuf::from("/Applications/Cursor.app"),
            // Cursor extension storage (VS Code-based)
            base.join("Library/Application Support/Cursor/User/globalStorage"),
            // Linux
            base.join(".config/Cursor/User/globalStorage"),
            // Windows
            base.join("AppData/Roaming/Cursor/User/globalStorage"),
        ]
    }
}

impl Default for CursorProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for CursorProvider {
    fn id(&self) -> &str {
        "cursor"
    }
    fn name(&self) -> &str {
        "Cursor"
    }
    fn description(&self) -> &str {
        "Cursor AI editor (VS Code-based). Detected but token data stored in binary LevelDB. \
         Cursor's API pricing follows standard OpenAI/Anthropic rates for the model selected."
    }

    fn is_available(&self) -> bool {
        Self::detection_paths().iter().any(|p| p.exists())
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        vec![]
    }

    fn scan(&self, _db: &Database) -> Result<usize> {
        // Detection only
        Ok(0)
    }
}
