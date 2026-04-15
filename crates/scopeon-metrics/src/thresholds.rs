//! Adaptive threshold engine.
//!
//! Derives personalised signal thresholds from the user's own historical
//! percentiles stored in `daily_rollup`.  When insufficient history exists
//! (< 7 days), every threshold falls back to a built-in default so the tool
//! works perfectly on day one.
//!
//! # Thresholds computed
//! | Field | Percentile | Default | Meaning |
//! |-------|-----------|---------|---------|
//! `cold_cache_pct`           | P10 cache hit rate  | 5.0%  | Below this → cold cache signal |
//! `thinking_ratio_warn`      | P90 thinking ratio  | 2.0   | Above this → thinking waste warn |
//! `thinking_ratio_critical`  | 2× P90              | 5.0   | Above this → thinking waste critical |
//! `context_bloat_multiplier` | P90 input/avg ratio | 2.0   | Turn input > avg × this → context bloat |

/// Personalised waste/health thresholds derived from the user's history.
///
/// All fields have sensible defaults for new installs where no history exists.
#[derive(Debug, Clone)]
pub struct UserThresholds {
    /// Cache hit rate below this percentage (after turn 3) fires a cold-cache signal.
    pub cold_cache_pct: f64,
    /// Thinking/output token ratio above this fires a Warning waste signal.
    pub thinking_ratio_warn: f64,
    /// Thinking/output token ratio above this fires a Critical waste signal.
    pub thinking_ratio_critical: f64,
    /// A turn whose input exceeds `avg_input × context_bloat_multiplier` fires a context-bloat signal.
    pub context_bloat_multiplier: f64,
}

impl Default for UserThresholds {
    fn default() -> Self {
        Self {
            cold_cache_pct: 5.0,
            thinking_ratio_warn: 2.0,
            thinking_ratio_critical: 5.0,
            context_bloat_multiplier: 2.0,
        }
    }
}

impl UserThresholds {
    /// Compute personalised thresholds from a slice of per-day stats.
    ///
    /// Requires ≥ 7 data points to move away from defaults; falls back
    /// gracefully otherwise.
    ///
    /// Each tuple is `(cache_hit_rate_0_to_1, thinking_ratio, avg_input_per_turn)`.
    pub fn from_daily_data(data: &[(f64, f64, f64)]) -> Self {
        if data.len() < 7 {
            return Self::default();
        }

        let mut cache_rates: Vec<f64> = data.iter().map(|(c, _, _)| *c * 100.0).collect();
        let mut thinking_ratios: Vec<f64> = data
            .iter()
            .filter(|(_, r, _)| *r > 0.0)
            .map(|(_, r, _)| *r)
            .collect();

        cache_rates.sort_by(f64::total_cmp);
        thinking_ratios.sort_by(f64::total_cmp);

        let cold_cache_pct = if cache_rates.is_empty() {
            5.0
        } else {
            percentile(&cache_rates, 10).max(1.0) // never set threshold below 1%
        };

        let (thinking_ratio_warn, thinking_ratio_critical) = if thinking_ratios.len() < 3 {
            (2.0, 5.0)
        } else {
            let p90 = percentile(&thinking_ratios, 90).max(1.5);
            (p90, (p90 * 2.0).max(4.0))
        };

        Self {
            cold_cache_pct,
            thinking_ratio_warn,
            thinking_ratio_critical,
            context_bloat_multiplier: 2.0, // kept fixed; input variance is too session-dependent
        }
    }
}

/// Linear interpolation percentile (0–100) for a **sorted** slice.
fn percentile(sorted: &[f64], p: usize) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let p = p.min(100) as f64 / 100.0;
    let idx = p * (sorted.len() - 1) as f64;
    let lo = idx.floor() as usize;
    let hi = (lo + 1).min(sorted.len() - 1);
    let frac = idx - lo as f64;
    sorted[lo] + frac * (sorted[hi] - sorted[lo])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults_when_insufficient_data() {
        let t = UserThresholds::from_daily_data(&[]);
        assert_eq!(t.cold_cache_pct, 5.0);
        assert_eq!(t.thinking_ratio_warn, 2.0);

        let sparse: Vec<_> = (0..6).map(|_| (0.5, 1.0, 1000.0)).collect();
        let t2 = UserThresholds::from_daily_data(&sparse);
        assert_eq!(t2.cold_cache_pct, 5.0);
    }

    #[test]
    fn test_adaptive_cold_cache_threshold() {
        // User consistently has 40–80% cache hit rate → P10 is well above default 5%
        let data: Vec<_> = (0..10)
            .map(|i| (0.40 + i as f64 * 0.04, 1.5, 5000.0))
            .collect();
        let t = UserThresholds::from_daily_data(&data);
        assert!(
            t.cold_cache_pct > 5.0,
            "should raise threshold for user with good history, got {}",
            t.cold_cache_pct
        );
    }

    #[test]
    fn test_adaptive_thinking_ratio() {
        // User regularly uses high thinking budget → P90 should be above 2.0
        let data: Vec<_> = (0..10)
            .map(|i| (0.5, 3.0 + i as f64 * 0.5, 5000.0))
            .collect();
        let t = UserThresholds::from_daily_data(&data);
        assert!(
            t.thinking_ratio_warn > 2.0,
            "should adapt warn threshold for heavy thinker"
        );
        assert!(
            t.thinking_ratio_critical > t.thinking_ratio_warn,
            "critical must exceed warn"
        );
    }

    #[test]
    fn test_percentile_basic() {
        let sorted = vec![0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 100.0];
        let p50 = percentile(&sorted, 50);
        assert!((p50 - 45.0).abs() < 5.0, "p50≈45 got {}", p50);
        let p10 = percentile(&sorted, 10);
        assert!((p10 - 9.0).abs() < 5.0, "p10≈9 got {}", p10);
    }
}
