//! # scopeon-tui
//!
//! Ratatui terminal dashboard for Scopeon.
//!
//! ## Tabs
//! | Tab | Content |
//! |-----|---------|
//! | **Dashboard** | Health score, live context pressure, cost, cache, and trend summary |
//! | **Sessions** | Session list, per-session drill-down, replay, and shadow pricing |
//! | **Insights** | Waste signals, suggestions, and optimization patterns |
//! | **Budget** | Daily/weekly/monthly spend, forecasts, and context pressure |
//! | **Providers** | Provider activity, availability, and attribution hints |
//! | **Agents** | Parent/sub-agent trees with per-node cost and token totals |
//!
//! ## Usage
//! ```no_run
//! # use std::sync::Arc;
//! # async fn example() -> anyhow::Result<()> {
//! // Open the database and run the TUI event loop.
//! // let db = Arc::new(std::sync::Mutex::new(scopeon_core::Database::open(path)?));
//! // scopeon_tui::run_tui(db).await?;
//! # Ok(())
//! # }
//! ```

pub mod app;
pub mod logo;
pub mod text;
pub mod theme;
pub mod ui;
pub mod views;
pub mod wizard;

pub use app::{run_tui, App};
pub use theme::Theme;
