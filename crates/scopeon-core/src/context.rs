//! Context window intelligence — per-model token limit lookup.
//!
//! Each AI provider exposes a maximum context window (in tokens). Knowing this
//! lets Scopeon compute how "full" the current session's context is and warn
//! before compaction or context-loss becomes necessary.
//!
//! Models are matched by prefix (same strategy as cost.rs pricing).
//! Unknown models fall back to 128k as a conservative estimate.

/// (model_prefix, context_window_tokens)
static CONTEXT_WINDOWS: &[(&str, i64)] = &[
    // ── Anthropic Claude ─────────────────────────────────────────────────────
    ("claude", 200_000),
    // ── OpenAI ───────────────────────────────────────────────────────────────
    ("gpt-4.1", 1_047_576), // GPT-4.1 / 4.1-mini / 4.1-nano: 1M
    ("gpt-4o-mini", 128_000),
    ("gpt-4o", 128_000),
    ("gpt-4-turbo", 128_000),
    ("gpt-4", 128_000),
    ("gpt-3.5-turbo", 16_385),
    ("o4-mini", 200_000),
    ("o3-mini", 200_000),
    ("o3", 200_000),
    ("o1-mini", 128_000),
    ("o1", 200_000),
    // ── Google Gemini ─────────────────────────────────────────────────────────
    ("gemini-1.5-pro", 2_000_000),
    ("gemini-1.5-flash", 1_000_000),
    ("gemini-2.5-pro", 1_000_000),
    ("gemini-2.5-flash", 1_000_000),
    ("gemini-2.0-flash", 1_000_000),
    ("gemini-1.0-pro", 32_768),
    ("gemini", 1_000_000), // catch-all for future gemini models
    // ── Misc / edge cases ─────────────────────────────────────────────────────
    ("copilot", 128_000),
    ("aider", 128_000),
    ("cursor", 128_000),
];

/// Return the context window size (in tokens) for the given model name.
///
/// Matching is case-insensitive prefix (`starts_with`) search ordered by
/// specificity (longer / more-specific entries must come first in
/// `CONTEXT_WINDOWS`). Falls back to 128k for unknown models.
pub fn context_window_for_model(model: &str) -> i64 {
    let model_lower = model.to_lowercase();
    CONTEXT_WINDOWS
        .iter()
        .find(|(prefix, _)| model_lower.starts_with(*prefix))
        .map(|(_, limit)| *limit)
        .unwrap_or(128_000)
}

/// Compute context fill percentage and remaining token count.
///
/// Returns `(fill_pct, tokens_remaining)` where `fill_pct` is 0.0–100.0.
/// Uses the model-prefix table in `CONTEXT_WINDOWS` to determine the window size.
pub fn context_pressure(model: &str, input_tokens: i64) -> (f64, i64) {
    let window = context_window_for_model(model);
    let fill_pct = (input_tokens as f64 / window as f64 * 100.0).clamp(0.0, 100.0);
    let remaining = (window - input_tokens).max(0);
    (fill_pct, remaining)
}

/// §8.2: Like `context_pressure` but accepts an optional stored window size.
/// When `stored_window` is `Some`, it is used in place of the model-lookup table.
/// This provides accurate fill percentages when the JSONL recorded `max_tokens`.
pub fn context_pressure_with_window(
    model: &str,
    input_tokens: i64,
    stored_window: Option<i64>,
) -> (f64, i64) {
    let window = stored_window.unwrap_or_else(|| context_window_for_model(model));
    let window = window.max(1); // guard against zero / invalid stored value
    let fill_pct = (input_tokens as f64 / window as f64 * 100.0).clamp(0.0, 100.0);
    let remaining = (window - input_tokens).max(0);
    (fill_pct, remaining)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claude_200k() {
        assert_eq!(context_window_for_model("claude-sonnet-4-5"), 200_000);
        assert_eq!(context_window_for_model("claude-opus-4"), 200_000);
        assert_eq!(context_window_for_model("claude-haiku-4-5"), 200_000);
    }

    #[test]
    fn test_gpt4o_128k() {
        assert_eq!(context_window_for_model("gpt-4o"), 128_000);
        assert_eq!(context_window_for_model("gpt-4o-mini"), 128_000);
    }

    #[test]
    fn test_gpt41_1m() {
        assert_eq!(context_window_for_model("gpt-4.1"), 1_047_576);
        assert_eq!(context_window_for_model("gpt-4.1-mini"), 1_047_576);
    }

    #[test]
    fn test_gemini_limits() {
        assert_eq!(context_window_for_model("gemini-1.5-pro"), 2_000_000);
        assert_eq!(context_window_for_model("gemini-2.0-flash"), 1_000_000);
    }

    #[test]
    fn test_unknown_fallback() {
        assert_eq!(context_window_for_model("my-custom-model"), 128_000);
    }

    #[test]
    fn test_context_pressure_half_full() {
        let (pct, remaining) = context_pressure("claude-sonnet-4", 100_000);
        assert!((pct - 50.0).abs() < 0.01);
        assert_eq!(remaining, 100_000);
    }

    #[test]
    fn test_context_pressure_clamps_at_100() {
        let (pct, remaining) = context_pressure("claude-sonnet-4", 999_999);
        assert_eq!(pct as u64, 100);
        assert_eq!(remaining, 0);
    }

    #[test]
    fn test_context_pressure_zero_tokens() {
        let (pct, remaining) = context_pressure("claude-sonnet-4", 0);
        assert_eq!(pct, 0.0);
        assert_eq!(remaining, 200_000);
    }

    #[test]
    fn test_context_pressure_exactly_at_window_limit() {
        let window = context_window_for_model("claude-sonnet-4");
        let (pct, remaining) = context_pressure("claude-sonnet-4", window);
        assert!((pct - 100.0).abs() < 0.01);
        assert_eq!(remaining, 0);
    }

    #[test]
    fn test_context_pressure_overflow_clamped() {
        // More tokens than the window — should not produce > 100% or negative remaining
        let window = context_window_for_model("gpt-4o"); // 128k
        let (pct, remaining) = context_pressure("gpt-4o", window * 2);
        assert_eq!(pct as u64, 100, "pressure must clamp at 100%");
        assert_eq!(remaining, 0, "remaining must not go negative");
    }

    #[test]
    fn test_gemini_15_pro_has_larger_window_than_20_flash() {
        // Ensures prefix specificity works: gemini-1.5-pro > gemini-2.0-flash in window
        let w_15 = context_window_for_model("gemini-1.5-pro");
        let w_20 = context_window_for_model("gemini-2.0-flash");
        assert_eq!(w_15, 2_000_000);
        assert_eq!(w_20, 1_000_000);
        assert!(w_15 > w_20);
    }

    #[test]
    fn test_ollama_model_uses_default_fallback() {
        // Ollama model names are arbitrary; must still return a sensible default
        let w = context_window_for_model("llama3.2:latest");
        assert!(w > 0, "fallback window must be positive");
        assert_eq!(w, 128_000, "unknown models should fall back to 128k");
    }

    /// Every prefix in CONTEXT_WINDOWS that is a strict prefix of another must
    /// appear AFTER (i.e., at a higher table index than) all entries that extend
    /// it. Prefix lookup uses `starts_with` and returns the first match, so
    /// more-specific entries must precede more-general ones.
    #[test]
    fn test_no_prefix_shadowing_in_context_windows() {
        for (i, (prefix_i, _)) in CONTEXT_WINDOWS.iter().enumerate() {
            for (j, (prefix_j, _)) in CONTEXT_WINDOWS.iter().enumerate() {
                if i == j {
                    continue;
                }
                if prefix_j.starts_with(prefix_i) && prefix_j.len() > prefix_i.len() {
                    assert!(
                        j < i,
                        "CONTEXT_WINDOWS ordering violation: \
                         '{}' (index {}) shadows '{}' (index {}). \
                         The more-specific entry must appear first.",
                        prefix_i,
                        i,
                        prefix_j,
                        j
                    );
                }
            }
        }
    }
}
