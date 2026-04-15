use crate::metric::{Metric, MetricCategory, MetricContext, MetricValue};
use scopeon_core::{cache_savings_usd, context_window_for_model, get_pricing};

pub struct CacheHitRate;
impl Metric for CacheHitRate {
    fn id(&self) -> &str {
        "cache_hit_rate"
    }
    fn name(&self) -> &str {
        "Prompt Cache Hit Rate"
    }
    fn description(&self) -> &str {
        "Percentage of context tokens served from cache"
    }
    fn category(&self) -> MetricCategory {
        MetricCategory::Cache
    }
    fn unit(&self) -> &str {
        "%"
    }
    fn compute(&self, ctx: &MetricContext) -> MetricValue {
        if ctx.turns.is_empty() {
            return MetricValue::Unavailable;
        }
        let cache_read: i64 = ctx.turns.iter().map(|t| t.cache_read_tokens).sum();
        let input: i64 = ctx.turns.iter().map(|t| t.input_tokens).sum();
        let cache_write: i64 = ctx.turns.iter().map(|t| t.cache_write_tokens).sum();
        let total = cache_read + input + cache_write;
        if total == 0 {
            return MetricValue::Percentage(0.0);
        }
        MetricValue::Percentage(cache_read as f64 / total as f64 * 100.0)
    }
    fn format(&self, value: &MetricValue) -> String {
        match value {
            MetricValue::Percentage(p) => format!("{:.1}%", p),
            _ => "—".to_string(),
        }
    }
}

pub struct CacheRoi;
impl Metric for CacheRoi {
    fn id(&self) -> &str {
        "cache_roi"
    }
    fn name(&self) -> &str {
        "Cache ROI"
    }
    fn description(&self) -> &str {
        "Return per dollar invested in caching"
    }
    fn category(&self) -> MetricCategory {
        MetricCategory::Cache
    }
    fn unit(&self) -> &str {
        "×"
    }
    fn compute(&self, ctx: &MetricContext) -> MetricValue {
        if ctx.turns.is_empty() {
            return MetricValue::Unavailable;
        }
        let mtok = 1_000_000.0f64;
        // Accumulate savings and investment per turn so that each turn's own
        // model pricing is used. This handles multi-model sessions correctly,
        // matching the approach already used by CacheSavingsUsd.
        let mut total_savings = 0.0f64;
        let mut total_investment = 0.0f64;
        for t in ctx.turns {
            let pricing = get_pricing(&t.model);
            total_savings += t.cache_read_tokens as f64 / mtok
                * (pricing.input_per_mtok - pricing.cache_read_per_mtok);
            total_investment += t.cache_write_tokens as f64 / mtok * pricing.cache_write_per_mtok;
        }
        if total_investment == 0.0 {
            return MetricValue::Unavailable;
        }
        MetricValue::Float(total_savings / total_investment)
    }
    fn format(&self, value: &MetricValue) -> String {
        match value {
            MetricValue::Float(f) => format!("{:.1}×", f),
            _ => "—".to_string(),
        }
    }
}

pub struct CacheSavingsUsd;
impl Metric for CacheSavingsUsd {
    fn id(&self) -> &str {
        "cache_savings_usd"
    }
    fn name(&self) -> &str {
        "Cache Savings"
    }
    fn description(&self) -> &str {
        "Net USD saved by the prompt cache (read savings minus write overhead)"
    }
    fn category(&self) -> MetricCategory {
        MetricCategory::Cache
    }
    fn unit(&self) -> &str {
        "USD"
    }
    fn compute(&self, ctx: &MetricContext) -> MetricValue {
        if ctx.turns.is_empty() {
            return MetricValue::Unavailable;
        }
        let savings: f64 = ctx
            .turns
            .iter()
            .map(|t| cache_savings_usd(&t.model, t.cache_read_tokens, t.cache_write_tokens))
            .sum();
        MetricValue::Currency(savings)
    }
}

pub struct CacheWarmupTurns;
impl Metric for CacheWarmupTurns {
    fn id(&self) -> &str {
        "cache_warmup_turns"
    }
    fn name(&self) -> &str {
        "Cache Warmup"
    }
    fn description(&self) -> &str {
        "Turns until first cache hit"
    }
    fn category(&self) -> MetricCategory {
        MetricCategory::Cache
    }
    fn unit(&self) -> &str {
        "turns"
    }
    fn compute(&self, ctx: &MetricContext) -> MetricValue {
        if ctx.turns.is_empty() {
            return MetricValue::Unavailable;
        }
        match ctx.turns.iter().position(|t| t.cache_read_tokens > 0) {
            Some(idx) => MetricValue::Count(idx as i64 + 1),
            None => MetricValue::Unavailable,
        }
    }
}

pub struct CompactionCount;
impl Metric for CompactionCount {
    fn id(&self) -> &str {
        "compaction_count"
    }
    fn name(&self) -> &str {
        "Compactions"
    }
    fn description(&self) -> &str {
        "Number of context compaction events in this session"
    }
    fn category(&self) -> MetricCategory {
        MetricCategory::Cache
    }
    fn unit(&self) -> &str {
        "events"
    }
    fn compute(&self, ctx: &MetricContext) -> MetricValue {
        if ctx.turns.is_empty() {
            return MetricValue::Unavailable;
        }
        let count = ctx.turns.iter().filter(|t| t.is_compaction_event).count();
        MetricValue::Count(count as i64)
    }
}

pub struct ContextPressurePct;
impl Metric for ContextPressurePct {
    fn id(&self) -> &str {
        "context_pressure_pct"
    }
    fn name(&self) -> &str {
        "Context Pressure"
    }
    fn description(&self) -> &str {
        "Last turn context usage as % of model's context window limit"
    }
    fn category(&self) -> MetricCategory {
        MetricCategory::Cache
    }
    fn unit(&self) -> &str {
        "%"
    }
    fn compute(&self, ctx: &MetricContext) -> MetricValue {
        if ctx.turns.is_empty() {
            return MetricValue::Unavailable;
        }
        let last = ctx.turns.last().unwrap();
        let model = if last.model.is_empty() {
            ctx.session.map(|s| s.model.as_str()).unwrap_or("unknown")
        } else {
            last.model.as_str()
        };
        let window = context_window_for_model(model);
        let used = last.input_tokens + last.cache_read_tokens + last.cache_write_tokens;
        let pct = used as f64 / window as f64 * 100.0;
        MetricValue::Percentage(pct.min(100.0))
    }
}
