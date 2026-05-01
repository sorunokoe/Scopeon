//! # scopeon-core
//!
//! Shared data models, SQLite persistence, cost estimation, and context window
//! intelligence for Scopeon.
//!
//! This crate is the foundation layer. All other Scopeon crates depend on it.
//!
//! ## Modules
//! - [`models`] — strongly-typed structs: [`Session`], [`Turn`], [`ToolCall`],
//!   [`SessionStats`], [`GlobalStats`], [`DailyRollup`], [`AgentNode`], …
//! - [`db`] — [`Database`] wrapping SQLite with WAL mode and schema migrations
//!   (7 migrations, auto-applied on open).
//! - [`cost`] — per-model cost calculation using published pricing;
//!   [`cache_hit_rate`] canonical formula used everywhere.
//! - [`context`] — [`context_window_for_model`] and [`context_pressure`] for
//!   model-aware context fill computation.
//! - [`config`] — runtime configuration ([`Config`]: DB path, provider dirs).
//! - [`user_config`] — user preferences ([`UserConfig`]: theme, budget, alerts).

pub mod config;
pub mod context;
pub mod cost;
pub mod db;
pub mod models;
pub mod optimization;
pub mod provenance;
pub mod tags;
pub mod user_config;

pub use config::Config;
pub use context::{context_pressure, context_pressure_with_window, context_window_for_model};
pub use cost::{
    cache_hit_rate, cache_savings_usd, get_pricing, get_pricing_with_overrides, shadow_cost,
    ModelPricing, PRICING_VERIFIED_DATE, UNKNOWN_MODELS_SEEN,
};
pub use db::{Database, COMPACTION_MIN_PREV_TOKENS};
pub use models::{
    fnv1a_64, AgentNode, DailyRollup, GlobalStats, InteractionEvent, ProjectStats,
    ProviderCapability, Session, SessionAnomaly, SessionStats, SessionSummary, TaskRun,
    ToolBreakdownItem, ToolCall, ToolStat, Turn,
};
pub use optimization::{
    apply_provider_preset, list_provider_optimization_reports, preview_provider_preset,
    ApplyReport, FileArtifactPreview, OptimizationPreset, OptimizationPresetId,
    OptimizationProviderId, OptimizationSupport, PresetPreview, ProviderOptimizationReport,
};
pub use provenance::{derive_hook_effects, interaction_token_total, provider_capabilities};
pub use tags::{branch_to_tag, infer_tag_from_tool_calls};
pub use user_config::{
    redact_webhook_url, ModelPricingOverride, OptimizerConfig, PricingConfig, StorageConfig,
    UserConfig, WebhookConfig,
};
