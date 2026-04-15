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
//! waste_pts = (1.0 − waste.waste_score) * 20.0   (waste_score ∈ [0.0, 1.0])
//! ```
//! Presence of detected waste signals (duplicate calls, oversized context, etc.)
//! reduces this component. Zero waste earns the full 20 pts.
//!
//! ## Daily rollup EMA (for sparklines / historical scores)
//! The daily health score is stored in `daily_rollup.health_score_avg` using a
//! 70/30 exponential moving average:
//! ```text
//! new_avg = 0.70 × today_score + 0.30 × prior_avg
//! ```
//! (First day uses today's score as the initial value.)

use crate::{metric::MetricContext, waste::WasteReport};
use scopeon_core::{cache_hit_rate, context_window_for_model};

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
        .map(|t| t.input_tokens + t.cache_read_tokens + t.cache_write_tokens)
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
    // Output tokens per dollar
    let efficiency = total_output as f64 / total_cost;
    // Baseline: ~33k output tokens per dollar at Sonnet pricing
    // >50k = excellent, 20-50k = good, 5-20k = ok, <5k = expensive
    if efficiency > 200_000.0 {
        25.0
    } else if efficiency > 50_000.0 {
        22.0
    } else if efficiency > 20_000.0 {
        17.0
    } else if efficiency > 5_000.0 {
        12.0
    } else if efficiency > 1_000.0 {
        6.0
    } else {
        2.0
    }
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
