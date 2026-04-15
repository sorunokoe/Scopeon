use anyhow::Result;
use scopeon_core::Database;
use std::path::PathBuf;

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
}
