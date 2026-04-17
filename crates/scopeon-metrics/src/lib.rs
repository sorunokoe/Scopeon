//! # scopeon-metrics
//!
//! Metric computation, waste analysis, health scoring, and optimization suggestions
//! for Scopeon.
//!
//! ## Modules
//! - [`registry`] — [`MetricRegistry`] holding all built-in metrics, queryable by ID or category.
//! - [`waste`] — [`WasteReport`] with severity-weighted waste signals (cold cache, spiky input,
//!   oversized tool payloads, etc.).
//! - [`suggestions`] — [`Suggestion`] list derived from waste signals and session patterns.
//! - [`health`] — [`compute_health_score`] — composite 0–100 efficiency score.
//! - [`builtin`] — default metric implementations (cache efficiency, cost velocity, session depth, …).

pub mod builtin;
pub mod health;
pub mod metric;
pub mod registry;
pub mod suggestions;
pub mod thresholds;
pub mod waste;

pub use health::{
    classify_project_profile, compute_health_score, compute_health_score_adaptive,
    compute_health_score_with_breakdown, AdaptiveHealthBreakdown, HealthBreakdown, ProjectProfile,
    WeightSet,
};
pub use metric::{Metric, MetricCategory, MetricContext, MetricValue};
pub use registry::MetricRegistry;
pub use suggestions::{compute_suggestions, Suggestion};
pub use thresholds::UserThresholds;
pub use waste::WasteReport;

#[cfg(test)]
mod tests;
