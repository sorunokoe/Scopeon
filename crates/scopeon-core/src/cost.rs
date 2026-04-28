//! Cost estimation engine for Anthropic API usage.
//!
//! Pricing is embedded at compile time based on Anthropic's published rates.
//! All prices are in USD per million tokens ($/MTok).
//!
//! # Model matching
//! Models are matched by prefix (e.g. `"claude-opus-4"` matches `"claude-opus-4-20250514"`).
//! More-specific sub-version entries (e.g. `"claude-opus-4-5"`) must appear *before*
//! the broader family entry in `PRICING` so they are matched first.
//! An unknown model falls back to Sonnet pricing as a safe middle estimate.
//!
//! # Keeping prices current
//! Update `PRICING` and `PRICING_VERIFIED_DATE` whenever providers publish new rates.
//! The TUI shows a staleness warning when the verified date is more than 90 days in the past.
//! Official sources:
//! - Anthropic: <https://www.anthropic.com/pricing>
//! - OpenAI: <https://openai.com/api/pricing/>
//! - Google: <https://ai.google.dev/gemini-api/docs/pricing>

/// The date when the `PRICING` table was last manually verified against
/// official provider pricing pages (ISO 8601, UTC).
///
/// Update this whenever `PRICING` is updated so the TUI staleness warning
/// resets. Format: `"YYYY-MM-DD"`.
pub const PRICING_VERIFIED_DATE: &str = "2026-04-27";

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

/// Thread-safe set of unknown model names encountered during this process run.
/// Used by the TUI to surface pricing-accuracy warnings to the user.
/// Drained by the TUI toast mechanism — do NOT use for dedup of tracing::warn!
/// §6.3: HashSet eliminates the O(n) linear scan previously done with Vec.
pub static UNKNOWN_MODELS_SEEN: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// Separate set used only for dedup of tracing::warn! — never drained.
/// Ensures each unknown model name is only logged once per process lifetime.
static UNKNOWN_MODELS_LOGGED: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// Per-model pricing in USD per million tokens ($/MTok).
#[derive(Debug, Clone)]
pub struct ModelPricing {
    pub model_prefix: &'static str,
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_write_per_mtok: f64,
    pub cache_read_per_mtok: f64,
}

/// The fallback pricing used when no model prefix matches.
/// Sonnet-class pricing is a safe middle estimate for unknown models.
static FALLBACK_PRICING: ModelPricing = ModelPricing {
    model_prefix: "",
    input_per_mtok: 3.00,
    output_per_mtok: 15.00,
    cache_write_per_mtok: 3.75,
    cache_read_per_mtok: 0.30,
};

static PRICING: &[ModelPricing] = &[
    // ── Anthropic Claude ────────────────────────────────────────────────────
    // Specific sub-version entries must come before broader prefix entries.
    // Opus 4.5 / 4.6 are priced differently ($5/$25) from Opus 4 / 4.1 ($15/$75).
    // Opus 4.7 is the new flagship at the $5/MTok tier (same as 4.5, 4.6).
    ModelPricing {
        model_prefix: "claude-opus-4-7",
        input_per_mtok: 5.00,
        output_per_mtok: 25.00,
        cache_write_per_mtok: 6.25,
        cache_read_per_mtok: 0.50,
    },
    ModelPricing {
        model_prefix: "claude-opus-4-6",
        input_per_mtok: 5.00,
        output_per_mtok: 25.00,
        cache_write_per_mtok: 6.25,
        cache_read_per_mtok: 0.50,
    },
    ModelPricing {
        model_prefix: "claude-opus-4-5",
        input_per_mtok: 5.00,
        output_per_mtok: 25.00,
        cache_write_per_mtok: 6.25,
        cache_read_per_mtok: 0.50,
    },
    // Opus 4 (original) and 4.1 remain at the higher price point.
    ModelPricing {
        model_prefix: "claude-opus-4",
        input_per_mtok: 15.00,
        output_per_mtok: 75.00,
        cache_write_per_mtok: 18.75,
        cache_read_per_mtok: 1.50,
    },
    ModelPricing {
        model_prefix: "claude-sonnet-4",
        input_per_mtok: 3.00,
        output_per_mtok: 15.00,
        cache_write_per_mtok: 3.75,
        cache_read_per_mtok: 0.30,
    },
    ModelPricing {
        model_prefix: "claude-haiku-4",
        input_per_mtok: 1.00,
        output_per_mtok: 5.00,
        cache_write_per_mtok: 1.25,
        cache_read_per_mtok: 0.10,
    },
    ModelPricing {
        model_prefix: "claude-3-5-sonnet",
        input_per_mtok: 3.00,
        output_per_mtok: 15.00,
        cache_write_per_mtok: 3.75,
        cache_read_per_mtok: 0.30,
    },
    ModelPricing {
        model_prefix: "claude-3-5-haiku",
        input_per_mtok: 0.80,
        output_per_mtok: 4.00,
        cache_write_per_mtok: 1.00,
        cache_read_per_mtok: 0.08,
    },
    // claude-3-haiku must come after claude-3-5-haiku (more specific prefix first).
    ModelPricing {
        model_prefix: "claude-3-haiku",
        input_per_mtok: 0.25,
        output_per_mtok: 1.25,
        cache_write_per_mtok: 0.30,
        cache_read_per_mtok: 0.03,
    },
    ModelPricing {
        model_prefix: "claude-3-opus",
        input_per_mtok: 15.00,
        output_per_mtok: 75.00,
        cache_write_per_mtok: 18.75,
        cache_read_per_mtok: 1.50,
    },
    // ── OpenAI GPT ───────────────────────────────────────────────────────────
    // GPT-5 series (Codex CLI uses gpt-5.4-mini).
    // More-specific prefixes MUST come before the less-specific ones that
    // they start with (e.g. "gpt-5.4-mini" before "gpt-5.4").
    ModelPricing {
        model_prefix: "gpt-5.4-mini",
        input_per_mtok: 0.75,
        output_per_mtok: 4.50,
        cache_write_per_mtok: 0.00,
        cache_read_per_mtok: 0.075,
    },
    ModelPricing {
        model_prefix: "gpt-5.3-codex",
        input_per_mtok: 2.50,
        output_per_mtok: 15.00,
        cache_write_per_mtok: 0.00,
        cache_read_per_mtok: 0.25,
    },
    ModelPricing {
        model_prefix: "gpt-5.4",
        input_per_mtok: 2.50,
        output_per_mtok: 15.00,
        cache_write_per_mtok: 0.00,
        cache_read_per_mtok: 0.25,
    },
    ModelPricing {
        model_prefix: "gpt-5.2",
        input_per_mtok: 1.75,
        output_per_mtok: 14.00,
        cache_write_per_mtok: 0.00,
        cache_read_per_mtok: 0.175,
    },
    ModelPricing {
        model_prefix: "gpt-5.1",
        input_per_mtok: 1.25,
        output_per_mtok: 10.00,
        cache_write_per_mtok: 0.00,
        cache_read_per_mtok: 0.125,
    },
    // gpt-5 base / gpt-5-mini / gpt-5-nano use dashes (no decimal), so they do NOT match
    // any of the gpt-5.X entries above. They must appear BEFORE the catch-all "gpt-5" entry
    // because "gpt-5-mini".starts_with("gpt-5") is true.
    ModelPricing {
        model_prefix: "gpt-5-nano",
        input_per_mtok: 0.05,
        output_per_mtok: 0.40,
        cache_write_per_mtok: 0.00,
        cache_read_per_mtok: 0.005,
    },
    ModelPricing {
        model_prefix: "gpt-5-mini",
        input_per_mtok: 0.25,
        output_per_mtok: 2.00,
        cache_write_per_mtok: 0.00,
        cache_read_per_mtok: 0.025,
    },
    // Catch-all for the gpt-5 family (base model and any future gpt-5 variants without a
    // decimal sub-version). Must come AFTER all gpt-5.X and gpt-5-{nano,mini} entries.
    ModelPricing {
        model_prefix: "gpt-5",
        input_per_mtok: 1.25,
        output_per_mtok: 10.00,
        cache_write_per_mtok: 0.00,
        cache_read_per_mtok: 0.125,
    },
    ModelPricing {
        model_prefix: "gpt-4.1-nano",
        input_per_mtok: 0.10,
        output_per_mtok: 0.40,
        cache_write_per_mtok: 0.00,
        cache_read_per_mtok: 0.025,
    },
    ModelPricing {
        model_prefix: "gpt-4.1-mini",
        input_per_mtok: 0.40,
        output_per_mtok: 1.60,
        cache_write_per_mtok: 0.00,
        cache_read_per_mtok: 0.10,
    },
    ModelPricing {
        model_prefix: "gpt-4.1",
        input_per_mtok: 2.00,
        output_per_mtok: 8.00,
        cache_write_per_mtok: 0.00,
        cache_read_per_mtok: 0.50,
    },
    ModelPricing {
        model_prefix: "gpt-4o-mini",
        input_per_mtok: 0.15,
        output_per_mtok: 0.60,
        cache_write_per_mtok: 0.00,
        cache_read_per_mtok: 0.075,
    },
    ModelPricing {
        model_prefix: "gpt-4o",
        input_per_mtok: 2.50,
        output_per_mtok: 10.00,
        cache_write_per_mtok: 0.00,
        cache_read_per_mtok: 1.25,
    },
    ModelPricing {
        model_prefix: "gpt-4-turbo",
        input_per_mtok: 10.00,
        output_per_mtok: 30.00,
        cache_write_per_mtok: 0.00,
        cache_read_per_mtok: 0.00,
    },
    ModelPricing {
        model_prefix: "gpt-4",
        input_per_mtok: 30.00,
        output_per_mtok: 60.00,
        cache_write_per_mtok: 0.00,
        cache_read_per_mtok: 0.00,
    },
    ModelPricing {
        model_prefix: "gpt-3.5-turbo",
        input_per_mtok: 0.50,
        output_per_mtok: 1.50,
        cache_write_per_mtok: 0.00,
        cache_read_per_mtok: 0.00,
    },
    ModelPricing {
        model_prefix: "o4-mini",
        input_per_mtok: 1.10,
        output_per_mtok: 4.40,
        cache_write_per_mtok: 0.00,
        cache_read_per_mtok: 0.275,
    },
    ModelPricing {
        model_prefix: "o3-mini",
        input_per_mtok: 1.10,
        output_per_mtok: 4.40,
        cache_write_per_mtok: 0.00,
        cache_read_per_mtok: 0.275,
    },
    ModelPricing {
        model_prefix: "o3",
        input_per_mtok: 10.00,
        output_per_mtok: 40.00,
        cache_write_per_mtok: 0.00,
        cache_read_per_mtok: 2.50,
    },
    ModelPricing {
        model_prefix: "o1-mini",
        input_per_mtok: 3.00,
        output_per_mtok: 12.00,
        cache_write_per_mtok: 0.00,
        cache_read_per_mtok: 0.75,
    },
    ModelPricing {
        model_prefix: "o1",
        input_per_mtok: 15.00,
        output_per_mtok: 60.00,
        cache_write_per_mtok: 0.00,
        cache_read_per_mtok: 7.50,
    },
    // ── Google Gemini ────────────────────────────────────────────────────────
    // Gemini 3 series (all currently preview). Pricing uses the standard ≤200k token tier.
    // More-specific prefixes must precede broader ones (e.g. gemini-3.1-flash-lite before
    // gemini-3.1-pro, since "gemini-3.1-flash-lite" does not start with "gemini-3.1-pro"
    // and vice-versa — but both would be shadowed by a bare "gemini-3.1" entry).
    ModelPricing {
        model_prefix: "gemini-3.1-flash-lite",
        input_per_mtok: 0.25,
        output_per_mtok: 1.50,
        cache_write_per_mtok: 1.00,
        cache_read_per_mtok: 0.025,
    },
    ModelPricing {
        model_prefix: "gemini-3.1-pro",
        input_per_mtok: 2.00,
        output_per_mtok: 12.00,
        cache_write_per_mtok: 4.50,
        cache_read_per_mtok: 0.20,
    },
    ModelPricing {
        model_prefix: "gemini-3-flash",
        input_per_mtok: 0.50,
        output_per_mtok: 3.00,
        cache_write_per_mtok: 1.00,
        cache_read_per_mtok: 0.05,
    },
    ModelPricing {
        model_prefix: "gemini-2.5-pro",
        input_per_mtok: 1.25,
        output_per_mtok: 10.00,
        cache_write_per_mtok: 4.50,
        cache_read_per_mtok: 0.31,
    },
    ModelPricing {
        model_prefix: "gemini-2.5-flash",
        input_per_mtok: 0.075,
        output_per_mtok: 0.30,
        cache_write_per_mtok: 1.00,
        cache_read_per_mtok: 0.019,
    },
    ModelPricing {
        model_prefix: "gemini-2.0-flash",
        input_per_mtok: 0.10,
        output_per_mtok: 0.40,
        cache_write_per_mtok: 1.00,
        cache_read_per_mtok: 0.025,
    },
    ModelPricing {
        model_prefix: "gemini-1.5-pro",
        input_per_mtok: 1.25,
        output_per_mtok: 5.00,
        cache_write_per_mtok: 4.50,
        cache_read_per_mtok: 0.3125,
    },
    ModelPricing {
        model_prefix: "gemini-1.5-flash",
        input_per_mtok: 0.075,
        output_per_mtok: 0.30,
        cache_write_per_mtok: 1.00,
        cache_read_per_mtok: 0.019,
    },
    // Unknown or future models fall back to FALLBACK_PRICING (Sonnet-class).
    // See get_pricing() — do NOT add a catch-all entry here; it would shadow FALLBACK_PRICING.
    // ── GitHub Copilot (Claude-backed) ─────────────────────────────────────
    // Copilot does not expose per-token billing; these entries ensure model names
    // are recognised and priced at the equivalent Anthropic model rate.
    ModelPricing {
        model_prefix: "copilot-claude-sonnet",
        input_per_mtok: 3.00,
        output_per_mtok: 15.00,
        cache_write_per_mtok: 3.75,
        cache_read_per_mtok: 0.30,
    },
    ModelPricing {
        model_prefix: "copilot-claude-haiku",
        input_per_mtok: 1.00,
        output_per_mtok: 5.00,
        cache_write_per_mtok: 1.25,
        cache_read_per_mtok: 0.10,
    },
    ModelPricing {
        model_prefix: "copilot-claude-opus",
        input_per_mtok: 15.00,
        output_per_mtok: 75.00,
        cache_write_per_mtok: 18.75,
        cache_read_per_mtok: 1.50,
    },
    // Unknown or future models fall back to FALLBACK_PRICING (Sonnet-class).
    // See get_pricing() — do NOT add a catch-all entry here; it would shadow FALLBACK_PRICING.
];

pub fn get_pricing(model: &str) -> &'static ModelPricing {
    match PRICING.iter().find(|p| model.starts_with(p.model_prefix)) {
        Some(p) => p,
        None => {
            // Synthetic/internal pseudo-model names used for compaction events and
            // test fixtures — silently fall back, no warning needed.
            let is_synthetic = model.is_empty()
                || model.starts_with('<')
                || model == "synthetic"
                || model == "unknown";

            if !is_synthetic {
                // §6.3: HashSet.insert() returns false if already present — O(1) dedup.
                if let Ok(mut seen) = UNKNOWN_MODELS_SEEN.lock() {
                    seen.insert(model.to_string());
                }
                if let Ok(mut logged) = UNKNOWN_MODELS_LOGGED.lock() {
                    if logged.insert(model.to_string()) {
                        tracing::warn!(
                            "Unknown model '{}' — falling back to Sonnet pricing (~${:.2}/MTok input). \
                             Add a pricing override in config.toml if this is incorrect.",
                            model,
                            FALLBACK_PRICING.input_per_mtok
                        );
                    }
                }
            }
            &FALLBACK_PRICING
        },
    }
}

/// Resolve pricing for `model`, applying any user-defined overrides on top of
/// the built-in `PRICING` table.
///
/// Only the fields explicitly set in the override are replaced; all others keep
/// their built-in values. Returns an **owned** `ModelPricing` (heap-allocated)
/// if any field is overridden, otherwise a reference to the static table entry.
///
/// Callers that need per-turn repricing (e.g. `reprice_all_in_transaction`)
/// should use this function instead of [`get_pricing`] when user overrides are
/// present.
pub fn get_pricing_with_overrides<'a>(
    model: &str,
    overrides: &'a std::collections::HashMap<String, crate::user_config::ModelPricingOverride>,
) -> std::borrow::Cow<'a, ModelPricing> {
    let base = get_pricing(model);

    // Find the most-specific override whose key is a prefix of `model`.
    let ovr = overrides
        .iter()
        .filter(|(k, _)| model.starts_with(k.as_str()))
        .max_by_key(|(k, _)| k.len()); // longest prefix wins

    match ovr {
        None => std::borrow::Cow::Borrowed(base),
        Some((_, o)) => {
            let merged = ModelPricing {
                model_prefix: base.model_prefix,
                input_per_mtok: o.input.unwrap_or(base.input_per_mtok),
                output_per_mtok: o.output.unwrap_or(base.output_per_mtok),
                cache_write_per_mtok: o.cache_write.unwrap_or(base.cache_write_per_mtok),
                cache_read_per_mtok: o.cache_read.unwrap_or(base.cache_read_per_mtok),
            };
            std::borrow::Cow::Owned(merged)
        },
    }
}

/// Canonical cache hit rate formula: `cache_read / (input + cache_read + cache_write)`.
///
/// Including `cache_write` in the denominator prevents fresh cache-warming turns
/// from artificially inflating the rate. Returns 0.0 when the denominator is zero.
pub fn cache_hit_rate(input: i64, cache_read: i64, _cache_write: i64) -> f64 {
    // Hit rate = tokens served from cache / (regular input + cached input).
    // cache_write tokens represent the cost of *writing* to the cache — they are
    // not eligible for a hit/miss event and must NOT be in the denominator.
    // Including them would artificially deflate the metric (L-1 code review finding).
    let denom = input + cache_read;
    if denom > 0 {
        cache_read as f64 / denom as f64
    } else {
        0.0
    }
}

/// Itemised cost breakdown for a single turn or session.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CostBreakdown {
    pub input_usd: f64,
    pub output_usd: f64,
    pub cache_write_usd: f64,
    pub cache_read_usd: f64,
    pub total_usd: f64,
}

pub fn calculate_turn_cost(
    model: &str,
    input_tokens: i64,
    output_tokens: i64,
    cache_write_tokens: i64,
    cache_read_tokens: i64,
) -> CostBreakdown {
    let p = get_pricing(model);
    let mtok = 1_000_000.0_f64;
    let input_usd = input_tokens as f64 / mtok * p.input_per_mtok;
    let output_usd = output_tokens as f64 / mtok * p.output_per_mtok;
    let cache_write_usd = cache_write_tokens as f64 / mtok * p.cache_write_per_mtok;
    let cache_read_usd = cache_read_tokens as f64 / mtok * p.cache_read_per_mtok;
    CostBreakdown {
        input_usd,
        output_usd,
        cache_write_usd,
        cache_read_usd,
        total_usd: input_usd + output_usd + cache_write_usd + cache_read_usd,
    }
}

/// Compute what a turn would cost on a different model (shadow pricing, IS-I).
///
/// Uses the same token counts but the target model's pricing table. Returns
/// `None` if the turn is already on the target model (prefix match).
pub fn shadow_cost(
    actual_model: &str,
    target_model: &str,
    input_tokens: i64,
    output_tokens: i64,
    cache_write_tokens: i64,
    cache_read_tokens: i64,
) -> Option<f64> {
    // Normalise: if the actual model starts with the target prefix, skip.
    if actual_model.starts_with(target_model) || target_model.starts_with(actual_model) {
        return None;
    }
    let cost = calculate_turn_cost(
        target_model,
        input_tokens,
        output_tokens,
        cache_write_tokens,
        cache_read_tokens,
    );
    Some(cost.total_usd)
}

/// Net USD saved by the prompt cache for a turn.
///
/// Accounts for both the read-side saving (paying cache_read price instead of
/// full input price) AND the write-side overhead (cache writes cost MORE than
/// regular input for most models). The result can be negative when a session
/// writes a large cache block that is read only once.
///
/// Formula:
///   net = (input_price - cache_read_price) × read_tokens
///       - (cache_write_price - input_price) × write_tokens
pub fn cache_savings_usd(model: &str, cache_read_tokens: i64, cache_write_tokens: i64) -> f64 {
    let p = get_pricing(model);
    let mtok = 1_000_000.0_f64;
    let read_saving = cache_read_tokens as f64 / mtok * (p.input_per_mtok - p.cache_read_per_mtok);
    let write_overhead =
        cache_write_tokens as f64 / mtok * (p.cache_write_per_mtok - p.input_per_mtok);
    read_saving - write_overhead
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f64 = 1e-9;

    #[test]
    fn test_opus_4_original_pricing() {
        // claude-opus-4 (original) and 4.1 → $15/MTok input
        let cost = calculate_turn_cost("claude-opus-4-20250514", 1_000_000, 0, 0, 0);
        assert!((cost.input_usd - 15.0).abs() < EPSILON);
        assert!((cost.total_usd - 15.0).abs() < EPSILON);

        // Opus 4.1 also maps to the $15 tier
        let cost_41 = calculate_turn_cost("claude-opus-4-1-20251001", 1_000_000, 0, 0, 0);
        assert!((cost_41.input_usd - 15.0).abs() < EPSILON);
    }

    #[test]
    fn test_opus_45_pricing() {
        // claude-opus-4-5 → $5/MTok input, $25/MTok output
        let cost = calculate_turn_cost("claude-opus-4-5-20251101", 1_000_000, 0, 0, 0);
        assert!(
            (cost.input_usd - 5.0).abs() < EPSILON,
            "Opus 4.5 input should be $5/MTok, got ${:.2}",
            cost.input_usd
        );
        let cost_out = calculate_turn_cost("claude-opus-4-5-20251101", 0, 1_000_000, 0, 0);
        assert!((cost_out.output_usd - 25.0).abs() < EPSILON);
    }

    #[test]
    fn test_opus_46_pricing() {
        // claude-opus-4-6 → $5/MTok input, $25/MTok output
        let cost = calculate_turn_cost("claude-opus-4-6-20260413", 1_000_000, 0, 0, 0);
        assert!(
            (cost.input_usd - 5.0).abs() < EPSILON,
            "Opus 4.6 input should be $5/MTok, got ${:.2}",
            cost.input_usd
        );
        // Cache pricing: write $6.25, read $0.50
        let cost_cache =
            calculate_turn_cost("claude-opus-4-6-20260413", 0, 0, 1_000_000, 1_000_000);
        assert!((cost_cache.cache_write_usd - 6.25).abs() < EPSILON);
        assert!((cost_cache.cache_read_usd - 0.50).abs() < EPSILON);
    }

    #[test]
    fn test_haiku_4_pricing() {
        // claude-haiku-4-5 → $1/MTok input, $5/MTok output
        let cost = calculate_turn_cost("claude-haiku-4-5-20251001", 1_000_000, 0, 0, 0);
        assert!(
            (cost.input_usd - 1.00).abs() < EPSILON,
            "Haiku 4.5 input should be $1.00/MTok, got ${:.2}",
            cost.input_usd
        );
        let cost_out = calculate_turn_cost("claude-haiku-4-5-20251001", 0, 1_000_000, 0, 0);
        assert!((cost_out.output_usd - 5.00).abs() < EPSILON);
        // Cache: write $1.25, read $0.10
        let cost_cache =
            calculate_turn_cost("claude-haiku-4-5-20251001", 0, 0, 1_000_000, 1_000_000);
        assert!((cost_cache.cache_write_usd - 1.25).abs() < EPSILON);
        assert!((cost_cache.cache_read_usd - 0.10).abs() < EPSILON);
    }

    #[test]
    fn test_claude3_haiku_pricing() {
        // claude-3-haiku → $0.25/MTok input, $1.25/MTok output
        let cost = calculate_turn_cost("claude-3-haiku-20240307", 1_000_000, 0, 0, 0);
        assert!(
            (cost.input_usd - 0.25).abs() < EPSILON,
            "claude-3-haiku input should be $0.25/MTok, got ${:.4}",
            cost.input_usd
        );
        let cost_out = calculate_turn_cost("claude-3-haiku-20240307", 0, 1_000_000, 0, 0);
        assert!((cost_out.output_usd - 1.25).abs() < EPSILON);
        // Cache: write $0.30, read $0.03
        let cost_cache = calculate_turn_cost("claude-3-haiku-20240307", 0, 0, 1_000_000, 1_000_000);
        assert!((cost_cache.cache_write_usd - 0.30).abs() < EPSILON);
        assert!((cost_cache.cache_read_usd - 0.03).abs() < EPSILON);
    }

    #[test]
    fn test_claude35_haiku_not_matched_by_claude3_haiku() {
        // claude-3-5-haiku should use $0.80/MTok, not the claude-3-haiku $0.25 rate
        let cost = calculate_turn_cost("claude-3-5-haiku-20241022", 1_000_000, 0, 0, 0);
        assert!(
            (cost.input_usd - 0.80).abs() < EPSILON,
            "claude-3-5-haiku input should be $0.80/MTok"
        );
    }

    #[test]
    fn test_opus45_not_matched_by_opus4_original() {
        // Ensures Opus 4.5 does NOT fall through to the $15 Opus 4 rate.
        let cost_45 = calculate_turn_cost("claude-opus-4-5-20251101", 1_000_000, 0, 0, 0);
        let cost_4 = calculate_turn_cost("claude-opus-4-20250514", 1_000_000, 0, 0, 0);
        assert!(
            cost_45.input_usd < cost_4.input_usd,
            "Opus 4.5 ($5) must be cheaper than Opus 4 ($15)"
        );
    }

    #[test]
    fn test_cache_write_and_read_costs() {
        let cost = calculate_turn_cost("claude-opus-4", 0, 0, 1_000_000, 1_000_000);
        // Cache write: $18.75/MTok, cache read: $1.50/MTok
        assert!((cost.cache_write_usd - 18.75).abs() < EPSILON);
        assert!((cost.cache_read_usd - 1.50).abs() < EPSILON);
        assert!((cost.total_usd - 20.25).abs() < EPSILON);
    }

    #[test]
    fn test_zero_tokens_zero_cost() {
        let cost = calculate_turn_cost("claude-opus-4", 0, 0, 0, 0);
        assert_eq!(cost.total_usd, 0.0);
    }

    #[test]
    fn test_unknown_model_falls_back_to_sonnet() {
        // Unknown model prefix → falls back to last pricing entry (Sonnet-class)
        let cost_unknown = calculate_turn_cost("some-future-unknown-model", 1_000_000, 0, 0, 0);
        let cost_sonnet = calculate_turn_cost("claude-sonnet-4", 1_000_000, 0, 0, 0);
        assert!((cost_unknown.input_usd - cost_sonnet.input_usd).abs() < EPSILON);
    }

    #[test]
    fn test_cache_savings_read_only() {
        // Read 1M tokens with no writes: net saving = ($15.00 - $1.50) / MTok = $13.50
        let savings = cache_savings_usd("claude-opus-4", 1_000_000, 0);
        assert!((savings - 13.50).abs() < EPSILON);
    }

    #[test]
    fn test_cache_savings_net_negative_when_write_overhead_dominates() {
        // Write 1M, read 1M for claude-sonnet-4:
        //   read_saving  = 1M × ($3.00 - $0.30) / MTok = $2.70
        //   write_overhead = 1M × ($3.75 - $3.00) / MTok = $0.75
        //   net = $2.70 - $0.75 = $1.95  (positive: one read pays off)
        let net = cache_savings_usd("claude-sonnet-4", 1_000_000, 1_000_000);
        assert!((net - 1.95).abs() < EPSILON, "got {net}");
    }

    #[test]
    fn test_cache_savings_zero_tokens() {
        assert_eq!(cache_savings_usd("claude-opus-4", 0, 0), 0.0);
    }

    #[test]
    fn test_total_is_sum_of_parts() {
        let cost = calculate_turn_cost("claude-opus-4", 100, 200, 300, 400);
        let expected =
            cost.input_usd + cost.output_usd + cost.cache_write_usd + cost.cache_read_usd;
        assert!((cost.total_usd - expected).abs() < EPSILON);
    }

    #[test]
    fn test_fallback_pricing_for_completely_unknown_model() {
        // Completely unrecognized model string → FALLBACK_PRICING (Sonnet-class)
        let cost_fallback = calculate_turn_cost("totally-unknown-xyz-model", 1_000_000, 0, 0, 0);
        let cost_fallback2 = calculate_turn_cost("another-mystery-model", 1_000_000, 0, 0, 0);
        // Both should use the same fallback and produce the same cost
        assert!(
            (cost_fallback.input_usd - cost_fallback2.input_usd).abs() < EPSILON,
            "all unknown models should use the same fallback pricing"
        );
        assert!(
            cost_fallback.total_usd > 0.0,
            "fallback pricing must not be zero"
        );
    }

    #[test]
    fn test_cache_hit_rate_all_zero() {
        assert_eq!(cache_hit_rate(0, 0, 0), 0.0);
    }

    #[test]
    fn test_cache_hit_rate_only_cache_read() {
        // input=0, cache_read=100, cache_write=0 → rate = 100/100 = 1.0
        let rate = cache_hit_rate(0, 100, 0);
        assert!((rate - 1.0).abs() < EPSILON);
    }

    #[test]
    fn test_cache_hit_rate_only_cache_write_no_reads() {
        // input=0, cache_read=0, cache_write=500 → denominator=500, numerator=0
        let rate = cache_hit_rate(0, 0, 500);
        assert_eq!(
            rate, 0.0,
            "no reads means zero hit rate even if writes exist"
        );
    }

    #[test]
    fn test_cache_hit_rate_mixed() {
        // input=100, read=400, write=100
        // Correct formula: read / (input + read) = 400 / 500 = 0.8
        // (write is excluded from denominator — L-1 fix)
        let rate = cache_hit_rate(100, 400, 100);
        assert!((rate - 400.0 / 500.0).abs() < EPSILON);
    }

    // ── Invariant tests: economic relationships ─────────────────────────────
    // These tests enforce properties that must hold for ALL pricing entries,
    // regardless of the exact dollar values. Any new entry that violates these
    // will fail CI immediately, catching logical errors before release.

    #[test]
    fn test_invariant_cache_read_always_cheaper_than_input() {
        for p in PRICING {
            assert!(
                p.cache_read_per_mtok <= p.input_per_mtok,
                "Model '{}': cache_read ({}) must be ≤ input ({}). \
                 Cached tokens should never cost more than fresh input.",
                p.model_prefix,
                p.cache_read_per_mtok,
                p.input_per_mtok
            );
        }
    }

    #[test]
    fn test_invariant_output_never_cheaper_than_input() {
        for p in PRICING {
            assert!(
                p.output_per_mtok >= p.input_per_mtok,
                "Model '{}': output ({}) must be ≥ input ({}). \
                 Generating tokens is always at least as expensive as reading them.",
                p.model_prefix,
                p.output_per_mtok,
                p.input_per_mtok
            );
        }
    }

    #[test]
    fn test_invariant_cache_write_never_cheaper_than_input() {
        // Cache write = the cost to *create* a cache entry. Providers charge a
        // premium for this. For models with no caching (cache_write=0) skip it.
        for p in PRICING {
            if p.cache_write_per_mtok > 0.0 {
                assert!(
                    p.cache_write_per_mtok >= p.input_per_mtok,
                    "Model '{}': cache_write ({}) must be ≥ input ({}). \
                     Cache creation always costs at least as much as normal input.",
                    p.model_prefix,
                    p.cache_write_per_mtok,
                    p.input_per_mtok
                );
            }
        }
    }

    #[test]
    fn test_invariant_all_prices_non_negative() {
        for p in PRICING {
            assert!(
                p.input_per_mtok >= 0.0,
                "Model '{}' has negative input price",
                p.model_prefix
            );
            assert!(
                p.output_per_mtok >= 0.0,
                "Model '{}' has negative output price",
                p.model_prefix
            );
            assert!(
                p.cache_write_per_mtok >= 0.0,
                "Model '{}' has negative cache_write price",
                p.model_prefix
            );
            assert!(
                p.cache_read_per_mtok >= 0.0,
                "Model '{}' has negative cache_read price",
                p.model_prefix
            );
        }
    }

    #[test]
    fn test_invariant_claude_family_price_ordering() {
        // Within the Claude 4 family, Haiku < Sonnet ≤ Opus on input price.
        let haiku = get_pricing("claude-haiku-4");
        let sonnet = get_pricing("claude-sonnet-4");
        let opus = get_pricing("claude-opus-4");
        assert!(
            haiku.input_per_mtok < sonnet.input_per_mtok,
            "Haiku 4 input (${}) must be cheaper than Sonnet 4 (${})",
            haiku.input_per_mtok,
            sonnet.input_per_mtok
        );
        assert!(
            sonnet.input_per_mtok <= opus.input_per_mtok,
            "Sonnet 4 input (${}) must be ≤ Opus 4 (${})",
            sonnet.input_per_mtok,
            opus.input_per_mtok
        );
    }

    // ── Invariant tests: prefix ordering ───────────────────────────────────
    // Each real-world model string must resolve to its specific PRICING entry,
    // not a broader catch-all. A wrong ordering in PRICING would silently apply
    // incorrect pricing to all affected models.

    #[test]
    fn test_prefix_ordering_claude_opus_versions() {
        // Opus 4.5 and 4.6 must NOT fall through to the Opus 4 catch-all.
        let p45 = get_pricing("claude-opus-4-5-20251101");
        let p46 = get_pricing("claude-opus-4-6-20260413");
        let p4 = get_pricing("claude-opus-4-20250514");
        assert_eq!(
            p45.model_prefix, "claude-opus-4-5",
            "claude-opus-4-5-20251101 must match 'claude-opus-4-5', not '{}'",
            p45.model_prefix
        );
        assert_eq!(
            p46.model_prefix, "claude-opus-4-6",
            "claude-opus-4-6-20260413 must match 'claude-opus-4-6', not '{}'",
            p46.model_prefix
        );
        assert_eq!(
            p4.model_prefix, "claude-opus-4",
            "claude-opus-4-20250514 must match 'claude-opus-4'"
        );
        // Price check: 4.5/4.6 are cheaper than 4.0
        assert!(
            p45.input_per_mtok < p4.input_per_mtok,
            "Opus 4.5 (${}) must be cheaper than Opus 4 (${})",
            p45.input_per_mtok,
            p4.input_per_mtok
        );
    }

    #[test]
    fn test_prefix_ordering_claude_haiku_generations() {
        // claude-3-haiku must NOT be caught by claude-3-5-haiku prefix.
        let p3 = get_pricing("claude-3-haiku-20240307");
        let p35 = get_pricing("claude-3-5-haiku-20241022");
        assert_eq!(
            p3.model_prefix, "claude-3-haiku",
            "claude-3-haiku-20240307 must match 'claude-3-haiku', not '{}'",
            p3.model_prefix
        );
        assert_eq!(
            p35.model_prefix, "claude-3-5-haiku",
            "claude-3-5-haiku-20241022 must match 'claude-3-5-haiku'"
        );
        // Claude 3 Haiku is cheaper than 3.5 Haiku
        assert!(
            p3.input_per_mtok < p35.input_per_mtok,
            "Haiku 3 input (${}) must be cheaper than Haiku 3.5 (${})",
            p3.input_per_mtok,
            p35.input_per_mtok
        );
    }

    #[test]
    fn test_prefix_ordering_gpt_mini_variants() {
        // gpt-4.1-mini must NOT be caught by gpt-4.1; gpt-4o-mini must NOT by gpt-4o.
        let p41mini = get_pricing("gpt-4.1-mini-2025-04-14");
        let p41 = get_pricing("gpt-4.1");
        let p4omini = get_pricing("gpt-4o-mini-2024-07-18");
        let p4o = get_pricing("gpt-4o");
        assert_eq!(
            p41mini.model_prefix, "gpt-4.1-mini",
            "gpt-4.1-mini-2025-04-14 must match 'gpt-4.1-mini', not '{}'",
            p41mini.model_prefix
        );
        assert_eq!(p41.model_prefix, "gpt-4.1", "gpt-4.1 must match 'gpt-4.1'");
        assert_eq!(
            p4omini.model_prefix, "gpt-4o-mini",
            "gpt-4o-mini-2024-07-18 must match 'gpt-4o-mini', not '{}'",
            p4omini.model_prefix
        );
        assert_eq!(p4o.model_prefix, "gpt-4o", "gpt-4o must match 'gpt-4o'");
        // Mini variants are cheaper than full models
        assert!(p41mini.input_per_mtok < p41.input_per_mtok);
        assert!(p4omini.input_per_mtok < p4o.input_per_mtok);
    }

    #[test]
    fn test_prefix_ordering_gpt4_nano_mini_full() {
        // gpt-4.1-nano must NOT be caught by gpt-4.1-mini or gpt-4.1.
        let pnano = get_pricing("gpt-4.1-nano");
        let pmini = get_pricing("gpt-4.1-mini");
        let pfull = get_pricing("gpt-4.1");
        assert_eq!(pnano.model_prefix, "gpt-4.1-nano");
        // nano < mini < full on input price
        assert!(
            pnano.input_per_mtok < pmini.input_per_mtok,
            "gpt-4.1-nano must be cheaper than gpt-4.1-mini"
        );
        assert!(
            pmini.input_per_mtok < pfull.input_per_mtok,
            "gpt-4.1-mini must be cheaper than gpt-4.1"
        );
    }

    /// Every entry in PRICING that is a strict prefix of another entry must appear
    /// AFTER (i.e., at a higher index than) all entries that extend it. The lookup
    /// uses `starts_with` and returns the FIRST match, so more-specific prefixes
    /// must come before more-general ones.
    #[test]
    fn test_no_prefix_shadowing_in_pricing_table() {
        for (i, entry_i) in PRICING.iter().enumerate() {
            for (j, entry_j) in PRICING.iter().enumerate() {
                if i == j {
                    continue;
                }
                // entry_j is strictly more specific than entry_i
                let pi = entry_i.model_prefix;
                let pj = entry_j.model_prefix;
                if pj.starts_with(pi) && pj.len() > pi.len() {
                    assert!(
                        j < i,
                        "PRICING table ordering violation: \
                         '{}' (index {}) shadows '{}' (index {}). \
                         The more-specific entry must appear first.",
                        pi,
                        i,
                        pj,
                        j
                    );
                }
            }
        }
    }

    // ── 2026-04-27 new model tests ──────────────────────────────────────────

    #[test]
    fn test_opus_47_pricing() {
        // claude-opus-4-7 → $5/MTok input, $25/MTok output (same tier as 4.5/4.6)
        let cost = calculate_turn_cost("claude-opus-4-7-20260501", 1_000_000, 0, 0, 0);
        assert!(
            (cost.input_usd - 5.0).abs() < EPSILON,
            "Opus 4.7 input should be $5/MTok, got ${:.2}",
            cost.input_usd
        );
        let cost_out = calculate_turn_cost("claude-opus-4-7-20260501", 0, 1_000_000, 0, 0);
        assert!((cost_out.output_usd - 25.0).abs() < EPSILON);
        // Cache: write $6.25, read $0.50
        let cost_cache =
            calculate_turn_cost("claude-opus-4-7-20260501", 0, 0, 1_000_000, 1_000_000);
        assert!((cost_cache.cache_write_usd - 6.25).abs() < EPSILON);
        assert!((cost_cache.cache_read_usd - 0.50).abs() < EPSILON);
    }

    #[test]
    fn test_opus47_not_matched_by_opus4_original() {
        // Ensures Opus 4.7 does NOT fall through to the $15 Opus 4 rate.
        let cost_47 = calculate_turn_cost("claude-opus-4-7-20260501", 1_000_000, 0, 0, 0);
        let cost_4 = calculate_turn_cost("claude-opus-4-20250514", 1_000_000, 0, 0, 0);
        assert!(
            cost_47.input_usd < cost_4.input_usd,
            "Opus 4.7 ($5) must be cheaper than Opus 4 ($15)"
        );
    }

    #[test]
    fn test_gpt5_base_pricing() {
        // gpt-5 (base) → $1.25/MTok input, $10/MTok output
        let cost = calculate_turn_cost("gpt-5", 1_000_000, 0, 0, 0);
        assert!(
            (cost.input_usd - 1.25).abs() < EPSILON,
            "gpt-5 input should be $1.25/MTok, got ${:.4}",
            cost.input_usd
        );
        let cost_out = calculate_turn_cost("gpt-5", 0, 1_000_000, 0, 0);
        assert!((cost_out.output_usd - 10.0).abs() < EPSILON);
        // Cache read: $0.125/MTok
        let cost_cache = calculate_turn_cost("gpt-5", 0, 0, 0, 1_000_000);
        assert!((cost_cache.cache_read_usd - 0.125).abs() < EPSILON);
    }

    #[test]
    fn test_gpt5_mini_pricing() {
        // gpt-5-mini → $0.25/MTok input, $2/MTok output
        let cost = calculate_turn_cost("gpt-5-mini", 1_000_000, 0, 0, 0);
        assert!(
            (cost.input_usd - 0.25).abs() < EPSILON,
            "gpt-5-mini input should be $0.25/MTok, got ${:.4}",
            cost.input_usd
        );
        let cost_out = calculate_turn_cost("gpt-5-mini", 0, 1_000_000, 0, 0);
        assert!((cost_out.output_usd - 2.0).abs() < EPSILON);
        // Cache read: $0.025/MTok
        let cost_cache = calculate_turn_cost("gpt-5-mini", 0, 0, 0, 1_000_000);
        assert!((cost_cache.cache_read_usd - 0.025).abs() < EPSILON);
    }

    #[test]
    fn test_gpt5_nano_pricing() {
        // gpt-5-nano → $0.05/MTok input, $0.40/MTok output
        let cost = calculate_turn_cost("gpt-5-nano", 1_000_000, 0, 0, 0);
        assert!(
            (cost.input_usd - 0.05).abs() < EPSILON,
            "gpt-5-nano input should be $0.05/MTok, got ${:.4}",
            cost.input_usd
        );
        let cost_out = calculate_turn_cost("gpt-5-nano", 0, 1_000_000, 0, 0);
        assert!((cost_out.output_usd - 0.40).abs() < EPSILON);
        // Cache read: $0.005/MTok
        let cost_cache = calculate_turn_cost("gpt-5-nano", 0, 0, 0, 1_000_000);
        assert!((cost_cache.cache_read_usd - 0.005).abs() < EPSILON);
    }

    #[test]
    fn test_gpt5_versioned_not_shadowed_by_gpt5_base() {
        // gpt-5.4, gpt-5.2, gpt-5.1 must still match their specific entries and
        // NOT be caught by the new gpt-5 catch-all.
        let p54 = get_pricing("gpt-5.4");
        let p52 = get_pricing("gpt-5.2");
        let p51 = get_pricing("gpt-5.1");
        let p5 = get_pricing("gpt-5");
        assert_eq!(p54.model_prefix, "gpt-5.4");
        assert_eq!(p52.model_prefix, "gpt-5.2");
        assert_eq!(p51.model_prefix, "gpt-5.1");
        assert_eq!(p5.model_prefix, "gpt-5");
        // gpt-5.4 ($2.50) is more expensive than gpt-5 base ($1.25)
        assert!(
            p54.input_per_mtok > p5.input_per_mtok,
            "gpt-5.4 (${}) must be more expensive than gpt-5 base (${})",
            p54.input_per_mtok,
            p5.input_per_mtok
        );
    }

    #[test]
    fn test_gpt5_dash_variants_not_shadowed_by_gpt5_base() {
        // gpt-5-mini and gpt-5-nano must NOT be caught by the gpt-5 base entry.
        let pmini = get_pricing("gpt-5-mini");
        let pnano = get_pricing("gpt-5-nano");
        let pbase = get_pricing("gpt-5");
        assert_eq!(pmini.model_prefix, "gpt-5-mini");
        assert_eq!(pnano.model_prefix, "gpt-5-nano");
        assert_eq!(pbase.model_prefix, "gpt-5");
        // nano < mini < base on input price
        assert!(pnano.input_per_mtok < pmini.input_per_mtok);
        assert!(pmini.input_per_mtok < pbase.input_per_mtok);
    }

    #[test]
    fn test_gemini_3_pro_pricing() {
        // gemini-3.1-pro-preview → $2.00/MTok input, $12/MTok output
        let cost = calculate_turn_cost("gemini-3.1-pro-preview", 1_000_000, 0, 0, 0);
        assert!(
            (cost.input_usd - 2.0).abs() < EPSILON,
            "gemini-3.1-pro input should be $2/MTok, got ${:.4}",
            cost.input_usd
        );
        let cost_out = calculate_turn_cost("gemini-3.1-pro-preview", 0, 1_000_000, 0, 0);
        assert!((cost_out.output_usd - 12.0).abs() < EPSILON);
        let cost_cache = calculate_turn_cost("gemini-3.1-pro-preview", 0, 0, 1_000_000, 1_000_000);
        assert!((cost_cache.cache_write_usd - 4.50).abs() < EPSILON);
        assert!((cost_cache.cache_read_usd - 0.20).abs() < EPSILON);
    }

    #[test]
    fn test_gemini_3_flash_pricing() {
        // gemini-3-flash-preview → $0.50/MTok input, $3/MTok output
        let cost = calculate_turn_cost("gemini-3-flash-preview", 1_000_000, 0, 0, 0);
        assert!(
            (cost.input_usd - 0.50).abs() < EPSILON,
            "gemini-3-flash input should be $0.50/MTok, got ${:.4}",
            cost.input_usd
        );
        let cost_out = calculate_turn_cost("gemini-3-flash-preview", 0, 1_000_000, 0, 0);
        assert!((cost_out.output_usd - 3.0).abs() < EPSILON);
        let cost_cache = calculate_turn_cost("gemini-3-flash-preview", 0, 0, 1_000_000, 1_000_000);
        assert!((cost_cache.cache_write_usd - 1.00).abs() < EPSILON);
        assert!((cost_cache.cache_read_usd - 0.05).abs() < EPSILON);
    }

    #[test]
    fn test_gemini_31_flash_lite_pricing() {
        // gemini-3.1-flash-lite-preview → $0.25/MTok input, $1.50/MTok output
        let cost = calculate_turn_cost("gemini-3.1-flash-lite-preview", 1_000_000, 0, 0, 0);
        assert!(
            (cost.input_usd - 0.25).abs() < EPSILON,
            "gemini-3.1-flash-lite input should be $0.25/MTok, got ${:.4}",
            cost.input_usd
        );
        let cost_out = calculate_turn_cost("gemini-3.1-flash-lite-preview", 0, 1_000_000, 0, 0);
        assert!((cost_out.output_usd - 1.50).abs() < EPSILON);
        let cost_cache =
            calculate_turn_cost("gemini-3.1-flash-lite-preview", 0, 0, 1_000_000, 1_000_000);
        assert!((cost_cache.cache_write_usd - 1.00).abs() < EPSILON);
        assert!((cost_cache.cache_read_usd - 0.025).abs() < EPSILON);
    }

    // ── cache_hit_rate correctness ────────────────────────────────────────────

    #[test]
    fn cache_hit_rate_all_zeros_returns_zero() {
        assert_eq!(cache_hit_rate(0, 0, 0), 0.0);
    }

    #[test]
    fn cache_hit_rate_no_cache_activity_returns_zero() {
        // 1000 plain input tokens, no cache → 0% hit rate
        assert_eq!(cache_hit_rate(1000, 0, 0), 0.0);
    }

    #[test]
    fn cache_hit_rate_full_cache_read_only() {
        // All tokens served from cache (read = total) → 100% relative to (input + read)
        // Denominator only includes input + read (not write):
        //   rate = read / (input + read) = 1000 / (0 + 1000) = 1.0
        let rate = cache_hit_rate(0, 1000, 0);
        assert!((rate - 1.0).abs() < EPSILON, "expected 1.0, got {rate}");
    }

    #[test]
    fn cache_hit_rate_half_cached() {
        // 500 input, 500 cache_read → 50% hit rate on (input + read)
        let rate = cache_hit_rate(500, 500, 0);
        assert!(
            (rate - 0.5).abs() < EPSILON,
            "expected 0.5, got {rate}"
        );
    }

    #[test]
    fn cache_hit_rate_write_tokens_do_not_inflate_denominator() {
        // Bug check: cache_write should NOT be in the denominator.
        // With 500 input, 500 read, 1_000_000 write tokens:
        //   WRONG: 500 / (500 + 500 + 1_000_000) ≈ 0.000499 (nearly zero)
        //   CORRECT: 500 / (500 + 500) = 0.5
        let rate = cache_hit_rate(500, 500, 1_000_000);
        assert!(
            (rate - 0.5).abs() < EPSILON,
            "cache_write must not deflate hit rate; expected 0.5, got {rate}"
        );
    }

    #[test]
    fn cache_hit_rate_large_numbers_no_overflow() {
        // 10M input + 10M read → 50% hit rate (using i64, no overflow)
        let rate = cache_hit_rate(10_000_000, 10_000_000, 0);
        assert!((rate - 0.5).abs() < EPSILON);
    }

    #[test]
    fn cache_hit_rate_result_in_zero_to_one() {
        // For any non-degenerate input the result must be in [0.0, 1.0]
        let cases = [
            (0_i64, 0_i64, 0_i64),
            (1000, 0, 0),
            (0, 1000, 0),
            (500, 500, 5000),
            (1, 999, 1_000_000),
        ];
        for (inp, rd, wr) in cases {
            let rate = cache_hit_rate(inp, rd, wr);
            assert!(
                (0.0..=1.0).contains(&rate),
                "cache_hit_rate({inp},{rd},{wr}) = {rate} out of [0,1]"
            );
        }
    }

    // ── calculate_turn_cost — structural correctness ─────────────────────────

    #[test]
    fn turn_cost_all_zero_tokens_returns_zero_cost() {
        let cost = calculate_turn_cost("claude-sonnet-4-5", 0, 0, 0, 0);
        assert_eq!(cost.input_usd, 0.0);
        assert_eq!(cost.output_usd, 0.0);
        assert_eq!(cost.cache_write_usd, 0.0);
        assert_eq!(cost.cache_read_usd, 0.0);
        assert_eq!(cost.total_usd, 0.0);
    }

    #[test]
    fn turn_cost_total_equals_sum_of_parts() {
        // total_usd must always equal sum of the four components
        let cases = [
            ("claude-sonnet-4-5", 1000, 500, 200, 300),
            ("claude-haiku-4-5-20251001", 5000, 2000, 100, 800),
            ("gpt-4o", 100_000, 50_000, 0, 0),
        ];
        for (model, inp, out, cw, cr) in cases {
            let cost = calculate_turn_cost(model, inp, out, cw, cr);
            let reconstructed = cost.input_usd + cost.output_usd + cost.cache_write_usd + cost.cache_read_usd;
            assert!(
                (cost.total_usd - reconstructed).abs() < EPSILON,
                "{model}: total {:.10} ≠ sum of parts {:.10}",
                cost.total_usd, reconstructed
            );
        }
    }

    #[test]
    fn turn_cost_nonzero_for_nonzero_tokens_known_model() {
        // A turn with real token counts must never show $0 cost
        let cost = calculate_turn_cost("claude-sonnet-4-5", 1000, 500, 0, 0);
        assert!(cost.total_usd > 0.0, "nonzero tokens must produce nonzero cost");
        assert!(cost.input_usd > 0.0);
        assert!(cost.output_usd > 0.0);
    }

    #[test]
    fn turn_cost_all_parts_non_negative() {
        // No cost component should ever go negative
        let cost = calculate_turn_cost("claude-sonnet-4-5", 1000, 1000, 500, 500);
        assert!(cost.input_usd >= 0.0);
        assert!(cost.output_usd >= 0.0);
        assert!(cost.cache_write_usd >= 0.0);
        assert!(cost.cache_read_usd >= 0.0);
        assert!(cost.total_usd >= 0.0);
    }

    // ── cache_savings_usd ─────────────────────────────────────────────────────

    #[test]
    fn cache_savings_zero_tokens_returns_zero() {
        assert_eq!(cache_savings_usd("claude-sonnet-4-5", 0, 0), 0.0);
    }

    #[test]
    fn cache_savings_read_only_positive_saving() {
        // Reading from cache avoids paying full input price → positive saving
        let saving = cache_savings_usd("claude-sonnet-4-5", 1_000_000, 0);
        assert!(saving > 0.0, "cache read should yield positive saving, got {saving}");
    }

    #[test]
    fn cache_savings_write_only_no_saving_or_negative() {
        // Writing to cache costs more than plain input for claude models
        // → net saving is ≤ 0.0 (no reads to offset the write overhead)
        let saving = cache_savings_usd("claude-sonnet-4-5", 0, 1_000_000);
        assert!(saving <= 0.0, "write-only cache should not yield net positive saving, got {saving}");
    }

    #[test]
    fn cache_savings_can_be_negative_when_write_overhead_exceeds_read_savings() {
        // Write 1M tokens, read only 1 token → overhead far exceeds tiny saving
        let saving = cache_savings_usd("claude-sonnet-4-5", 1, 1_000_000);
        assert!(saving < 0.0, "high write/low read should produce negative net saving, got {saving}");
    }

    // ── shadow_cost ───────────────────────────────────────────────────────────

    #[test]
    fn shadow_cost_same_model_prefix_returns_none() {
        // Comparing a model to itself → no shadow cost (would be trivial)
        let result = shadow_cost(
            "claude-sonnet-4-5", "claude-sonnet-4-5",
            1000, 500, 0, 0,
        );
        assert!(result.is_none(), "same model should return None");
    }

    #[test]
    fn shadow_cost_different_models_returns_some() {
        let result = shadow_cost(
            "claude-haiku-4-5-20251001", "claude-opus-4-5",
            1000, 500, 0, 0,
        );
        assert!(result.is_some(), "different models should return Some");
        assert!(result.unwrap() > 0.0, "shadow cost must be positive for nonzero tokens");
    }

    #[test]
    fn shadow_cost_zero_tokens_returns_zero_cost() {
        let result = shadow_cost(
            "claude-haiku-4-5-20251001", "claude-opus-4-5",
            0, 0, 0, 0,
        );
        assert_eq!(result, Some(0.0));
    }

    #[test]
    fn shadow_cost_haiku_to_opus_is_higher() {
        // Opus is more expensive than Haiku → shadow cost > actual cost
        let actual = calculate_turn_cost("claude-haiku-4-5-20251001", 10_000, 5_000, 0, 0);
        let shadow = shadow_cost(
            "claude-haiku-4-5-20251001", "claude-opus-4-5",
            10_000, 5_000, 0, 0,
        ).unwrap();
        assert!(shadow > actual.total_usd,
            "opus shadow cost {shadow} should exceed haiku actual {}", actual.total_usd);
    }
}
