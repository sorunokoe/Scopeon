use anyhow::Result;
use scopeon_core::Database;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub mod aider;
pub mod claude;
pub mod copilot;
pub mod cursor;
pub mod gemini;
pub mod generic_openai;
pub mod ollama;

pub use aider::AiderProvider;
pub use claude::ClaudeCodeProvider;
pub use copilot::CopilotCliProvider;
pub use cursor::CursorProvider;
pub use gemini::GeminiCLIProvider;
pub use generic_openai::GenericOpenAIProvider;
pub use ollama::OllamaProvider;

pub trait Provider: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn is_available(&self) -> bool;
    fn watch_paths(&self) -> Vec<PathBuf>;
    fn scan(&self, db: &Database) -> Result<usize>;

    /// Scan with fine-grained mutex control.
    ///
    /// The default implementation acquires the mutex once for the entire scan.
    /// Providers that process many files (e.g. ClaudeCodeProvider) should override
    /// this to release the mutex between files so the TUI can refresh while
    /// background backfill is running.
    fn scan_incremental(&self, db: Arc<Mutex<Database>>) -> Result<usize> {
        let db_guard = db
            .lock()
            .map_err(|_| anyhow::anyhow!("Database mutex poisoned"))?;
        self.scan(&db_guard)
    }
}
