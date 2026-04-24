//! # scopeon-collector
//!
//! Reads AI agent session logs and stores token usage in SQLite.
//!
//! ## Provider abstraction
//! Each supported AI tool implements the [`Provider`] trait, which specifies:
//! - where to find session logs (`watch_paths`)
//! - how to scan them (`scan`)
//!
//! Supported providers: Claude Code, GitHub Copilot CLI, Codex (OpenAI CLI),
//! Aider, Cursor, Gemini CLI, Ollama, and a generic OpenAI-compatible provider.
//!
//! ## Entry points
//! - [`parse_file_incremental`] — parse new lines from a single `.jsonl` file
//!   starting at a given byte offset (idempotent, offset-tracked in SQLite).
//! - [`watcher::start_watching_providers`] — watch all provider paths with
//!   `notify` (FSEvents on macOS, inotify on Linux) and process changes as they
//!   are written.
//! - [`watcher::backfill_providers`] — scan all existing provider files on startup.

pub mod parser;
pub mod providers;
pub mod watcher;

pub use parser::parse_file_incremental;
pub use providers::{
    AiderProvider, ClaudeCodeProvider, CodexProvider, CopilotCliProvider, CursorProvider,
    GeminiCLIProvider, GenericOpenAIProvider, OllamaProvider, Provider,
};
pub use watcher::{backfill_providers, start_watching_providers};
