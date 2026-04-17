//! Composite health score for a session/context.
//!
//! Health score is a 0–100 number that summarizes how efficiently an AI session
//! is using tokens and money. High = efficient. Low = wasteful or risky.
//!
//! # §8.3 — Formula (authoritative)
//!
//! ```text
//! health_score = cache_pts + context_pts + cost_pts + waste_pts   (clamped 0–100)
//! ```
//!
//! ## Cache efficiency (0–30 pts)
//! ```text
//! cache_rate = cache_read_tokens / (input_tokens + cache_read_tokens + cache_write_tokens)
//! cache_pts  = cache_rate * 30.0
//! ```
//! Linear scaling; higher cache reuse earns more points.
//!
//! ## Context safety (0–25 pts)
//! ```text
//! fill_pct   = (input_tokens + cache_read_tokens) / context_window_size
//! context_pts = (1.0 − fill_pct) * 25.0   (clamped ≥ 0)
//! ```
//! Sessions near the context limit score close to 0; low-fill sessions earn ~25.
//!
//! ## Cost efficiency (0–25 pts)
//! ```text
//! output_per_dollar = output_tokens / max(estimated_cost_usd, ε)
//! baseline          = 50_000 output tokens per dollar (Sonnet-class reference)
//! cost_pts          = min(output_per_dollar / baseline, 1.0) * 25.0
//! ```
//! Rewards sessions that produce more output per dollar, capped at 25.
//!
//! ## Waste penalty (0–20 pts)
//! ```text
//! waste_pts = (1.0 − waste.waste_score / 100.0) * 20.0   (waste_score ∈ [0.0, 100.0])
//! ```
//! Presence of detected waste signals (duplicate calls, oversized context, etc.)
//! reduces this component. Zero waste earns the full 20 pts.
//!
//! ## Historical persistence
//! This module computes health scores in memory only.
//! If `daily_rollup.health_score_avg` is populated, it must come from an ingest-time
//! or offline rollup path, never from a render loop or other read path.

use crate::{metric::MetricContext, waste::WasteReport};
use scopeon_core::{cache_hit_rate, context_window_for_model};

// ── S-5: Self-calibrating adaptive weight system (TRIZ PC-5 resolution) ──────

/// Project profile inferred from session behaviour.
/// Determines which health-score weight preset is used so that scores are
/// meaningful for the current workflow rather than a one-size-fits-all average.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectProfile {
    /// High cache reuse (≥ 60% of tokens come from the cache read slot).
    CacheHeavy,
    /// Deep reasoning (thinking tokens ≥ 30% of output).
    Exploration,
    /// Dense tool use (≥ 5 MCP calls per turn on average).
    ToolHeavy,
    /// Mixed / default workload.
    Balanced,
}

/// Per-profile health-score weights (must sum to 100).
pub struct WeightSet {
    pub cache: f64,
    pub context: f64,
    pub cost: f64,
    pub waste: f64,
}

impl WeightSet {
    pub fn for_profile(p: ProjectProfile) -> Self {
        match p {
            // CacheHeavy: cache is the primary value-driver — reward it heavily.
            ProjectProfile::CacheHeavy => WeightSet {
                cache: 40.0,
                context: 20.0,
                cost: 20.0,
                waste: 20.0,
            },
            // Exploration: thinking tokens consume budget fast — context & waste matter most.
            ProjectProfile::Exploration => WeightSet {
                cache: 15.0,
                context: 30.0,
                cost: 20.0,
                waste: 35.0,
            },
            // ToolHeavy: many short calls accumulate cost quickly — penalise spend and waste.
            ProjectProfile::ToolHeavy => WeightSet {
                cache: 25.0,
                context: 20.0,
                cost: 30.0,
                waste: 25.0,
            },
            // Balanced: mild upgrade to cache vs historical baseline (still rewards good caching).
            ProjectProfile::Balanced => WeightSet {
                cache: 30.0,
                context: 25.0,
                cost: 25.0,
                waste: 20.0,
            },
        }
    }
}

/// Classify the current session into a `ProjectProfile`.
pub fn classify_project_profile(ctx: &MetricContext) -> ProjectProfile {
    let total_input: i64 = ctx.turns.iter().map(|t| t.input_tokens).sum();
    let cache_read: i64 = ctx.turns.iter().map(|t| t.cache_read_tokens).sum();
    let cache_write: i64 = ctx.turns.iter().map(|t| t.cache_write_tokens).sum();
    let total_output: i64 = ctx.turns.iter().map(|t| t.output_tokens).sum();

    let denominator = (total_input + cache_read + cache_write).max(1);
    let cache_intensity = cache_read as f64 / denominator as f64;

    // thinking_frac: thinking tokens are not stored per turn, but deeply-reasoning
    // sessions typically produce proportionally more output relative to input.
    // Use output / (input + cache_read) as a cheap proxy.
    let thinking_frac = if total_input + cache_read > 0 {
        total_output as f64 / (total_input + cache_read) as f64
    } else {
        0.0
    };

    let turns_count = ctx.turns.len().max(1);
    let mcp_density = ctx.tool_calls.len() as f64 / turns_count as f64;

    if cache_intensity > 0.60 {
        ProjectProfile::CacheHeavy
    } else if thinking_frac > 2.5 {
        // Output > 2.5× input is a strong signal of extended thinking/exploration.
        ProjectProfile::Exploration
    } else if mcp_density > 5.0 {
        ProjectProfile::ToolHeavy
    } else {
        ProjectProfile::Balanced
    }
}

/// Adaptive health score breakdown with profile-specific maxima.
pub struct AdaptiveHealthBreakdown {
    pub profile: ProjectProfile,
    pub weights: WeightSet,
    pub cache_pts: f64,
    pub context_pts: f64,
    pub cost_pts: f64,
    pub waste_pts: f64,
    pub total: f64,
}

impl AdaptiveHealthBreakdown {
    pub fn as_rows(&self) -> [(&'static str, f64, f64); 4] {
        [
            ("Cache", self.cache_pts, self.weights.cache),
            ("Context", self.context_pts, self.weights.context),
            ("Cost eff", self.cost_pts, self.weights.cost),
            ("Waste", self.waste_pts, self.weights.waste),
        ]
    }

    pub fn profile_label(&self) -> &'static str {
        match self.profile {
            ProjectProfile::CacheHeavy => "Cache-Heavy",
            ProjectProfile::Exploration => "Exploration",
            ProjectProfile::ToolHeavy => "Tool-Heavy",
            ProjectProfile::Balanced => "Balanced",
        }
    }
}

/// Compute health score with adaptive weights calibrated to the project profile.
///
/// Preferred over `compute_health_score` for the MCP and TUI insights paths.
pub fn compute_health_score_adaptive(
    ctx: &MetricContext,
    waste: &WasteReport,
) -> (f64, AdaptiveHealthBreakdown) {
    let profile = classify_project_profile(ctx);
    let w = WeightSet::for_profile(profile);

    let cache_pts = (cache_score(ctx) / 30.0 * w.cache).clamp(0.0, w.cache);
    let context_pts = (context_score(ctx) / 25.0 * w.context).clamp(0.0, w.context);
    let cost_pts = (cost_score(ctx) / 25.0 * w.cost).clamp(0.0, w.cost);
    let waste_pts = (waste_score_pts(waste) / 20.0 * w.waste).clamp(0.0, w.waste);
    let total = (cache_pts + context_pts + cost_pts + waste_pts).clamp(0.0, 100.0);

    (
        total,
        AdaptiveHealthBreakdown {
            profile,
            weights: WeightSet::for_profile(profile),
            cache_pts,
            context_pts,
            cost_pts,
            waste_pts,
            total,
        },
    )
}

/// Compute a 0–100 health score from the current session context.
pub fn compute_health_score(ctx: &MetricContext, waste: &WasteReport) -> f64 {
    let cache_pts = cache_score(ctx);
    let context_pts = context_score(ctx);
    let cost_pts = cost_score(ctx);
    let waste_pts = waste_score_pts(waste);
    (cache_pts + context_pts + cost_pts + waste_pts).clamp(0.0, 100.0)
}

/// Per-component breakdown of the health score.
/// Returned as (label, earned, max) tuples for display in the Insights tab.
pub struct HealthBreakdown {
    pub cache_pts: f64,
    pub context_pts: f64,
    pub cost_pts: f64,
    pub waste_pts: f64,
    pub total: f64,
}

impl HealthBreakdown {
    pub const CACHE_MAX: f64 = 30.0;
    pub const CONTEXT_MAX: f64 = 25.0;
    pub const COST_MAX: f64 = 25.0;
    pub const WASTE_MAX: f64 = 20.0;

    pub fn as_rows(&self) -> [(&'static str, f64, f64); 4] {
        [
            ("Cache", self.cache_pts, Self::CACHE_MAX),
            ("Context", self.context_pts, Self::CONTEXT_MAX),
            ("Cost eff", self.cost_pts, Self::COST_MAX),
            ("Waste", self.waste_pts, Self::WASTE_MAX),
        ]
    }
}

/// Compute health score with per-component breakdown for the Insights tab.
pub fn compute_health_score_with_breakdown(
    ctx: &MetricContext,
    waste: &WasteReport,
) -> (f64, HealthBreakdown) {
    let cache_pts = cache_score(ctx);
    let context_pts = context_score(ctx);
    let cost_pts = cost_score(ctx);
    let waste_pts = waste_score_pts(waste);
    let total = (cache_pts + context_pts + cost_pts + waste_pts).clamp(0.0, 100.0);
    (
        total,
        HealthBreakdown {
            cache_pts,
            context_pts,
            cost_pts,
            waste_pts,
            total,
        },
    )
}

/// Cache efficiency: 0–30 pts (linear — avoids step-function cliffs).
fn cache_score(ctx: &MetricContext) -> f64 {
    let total_input: i64 = ctx.turns.iter().map(|t| t.input_tokens).sum();
    let cache_read: i64 = ctx.turns.iter().map(|t| t.cache_read_tokens).sum();
    let cache_write: i64 = ctx.turns.iter().map(|t| t.cache_write_tokens).sum();
    if total_input + cache_read + cache_write == 0 {
        return 15.0; // neutral if no data
    }
    let rate = cache_hit_rate(total_input, cache_read, cache_write);
    (rate * 30.0).clamp(0.0, 30.0)
}

/// Context window safety: 0–25 pts (linear inverse — higher fill = fewer pts).
fn context_score(ctx: &MetricContext) -> f64 {
    let last = ctx.turns.last();
    let used = last
        .map(|t| t.input_tokens + t.cache_read_tokens)
        .unwrap_or(0);
    let model = ctx
        .turns
        .last()
        .map(|t| t.model.as_str())
        .unwrap_or(ctx.session.map(|s| s.model.as_str()).unwrap_or("unknown"));
    let limit = context_window_for_model(model);
    let pct = (used as f64 / limit as f64).min(1.0);
    // 25 pts at 0% fill, 0 pts at 100% fill — linear.
    ((1.0 - pct) * 25.0).clamp(0.0, 25.0)
}

/// Cost efficiency: 0–25 pts
/// Based on output tokens per dollar (higher = better)
fn cost_score(ctx: &MetricContext) -> f64 {
    let total_output: i64 = ctx.turns.iter().map(|t| t.output_tokens).sum();
    let total_cost: f64 = ctx.turns.iter().map(|t| t.estimated_cost_usd).sum();
    if total_cost <= 0.0 || total_output == 0 {
        return 15.0; // neutral
    }
    let output_per_dollar = total_output as f64 / total_cost;
    let baseline = 50_000.0_f64;
    ((output_per_dollar / baseline).min(1.0) * 25.0).clamp(0.0, 25.0)
}

/// Waste signal penalty: 0–20 pts (inverse of waste score)
fn waste_score_pts(waste: &WasteReport) -> f64 {
    // waste_score is 0–100 where higher = more waste
    // We want 20 pts when waste is 0, 0 pts when waste is 100
    let inverted = (100.0 - waste.waste_score).max(0.0) / 100.0;
    inverted * 20.0
}

/// Compute health score trend vs a previous day's score.
/// Returns delta as signed f64 (positive = improved, negative = declined).
pub fn health_trend(today: f64, yesterday: f64) -> f64 {
    today - yesterday
}
