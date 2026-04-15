//! # scopeon-mcp
//!
//! MCP (Model Context Protocol) server exposing Scopeon token-usage data to Claude Code.
//!
//! The server speaks JSON-RPC 2.0 over `stdin`/`stdout` and is configured in
//! `~/.claude/settings.json` via `scopeon init`.
//!
//! ## Available tools
//! | Tool | Description |
//! |------|-------------|
//! | `get_token_usage` | Current session breakdown |
//! | `get_session_summary` | Per-turn stats with cost |
//! | `get_cache_efficiency` | Hit rate, savings |
//! | `get_history` | Daily rollup history |
//! | `compare_sessions` | Before/after comparison |

pub mod server;

pub use server::run_mcp_server;
