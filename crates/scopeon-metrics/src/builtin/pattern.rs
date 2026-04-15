use crate::metric::{Metric, MetricCategory, MetricContext, MetricValue};

pub struct ContextGrowthRate;
impl Metric for ContextGrowthRate {
    fn id(&self) -> &str {
        "context_growth_rate"
    }
    fn name(&self) -> &str {
        "Context Growth"
    }
    fn description(&self) -> &str {
        "Average increase in context tokens per turn"
    }
    fn category(&self) -> MetricCategory {
        MetricCategory::Pattern
    }
    fn unit(&self) -> &str {
        "tok/turn"
    }
    fn compute(&self, ctx: &MetricContext) -> MetricValue {
        if ctx.turns.len() < 2 {
            return MetricValue::Unavailable;
        }
        let sizes: Vec<i64> = ctx
            .turns
            .iter()
            .map(|t| t.input_tokens + t.cache_read_tokens)
            .collect();
        let deltas: Vec<f64> = sizes.windows(2).map(|w| (w[1] - w[0]) as f64).collect();
        let avg = deltas.iter().sum::<f64>() / deltas.len() as f64;
        MetricValue::Float(avg)
    }
}

pub struct TurnSizeCv;
impl Metric for TurnSizeCv {
    fn id(&self) -> &str {
        "turn_size_cv"
    }
    fn name(&self) -> &str {
        "Turn Size CV"
    }
    fn description(&self) -> &str {
        "Coefficient of variation of turn sizes — high = bursty"
    }
    fn category(&self) -> MetricCategory {
        MetricCategory::Pattern
    }
    fn unit(&self) -> &str {
        "%"
    }
    fn compute(&self, ctx: &MetricContext) -> MetricValue {
        if ctx.turns.len() < 2 {
            return MetricValue::Unavailable;
        }
        let sizes: Vec<f64> = ctx
            .turns
            .iter()
            .map(|t| (t.input_tokens + t.output_tokens + t.cache_read_tokens) as f64)
            .collect();
        let mean = sizes.iter().sum::<f64>() / sizes.len() as f64;
        if mean == 0.0 {
            return MetricValue::Unavailable;
        }
        let variance = sizes.iter().map(|s| (s - mean).powi(2)).sum::<f64>() / sizes.len() as f64;
        let std_dev = variance.sqrt();
        MetricValue::Float(std_dev / mean * 100.0)
    }
}

pub struct SessionDepth;
impl Metric for SessionDepth {
    fn id(&self) -> &str {
        "session_depth"
    }
    fn name(&self) -> &str {
        "Session Depth"
    }
    fn description(&self) -> &str {
        "Total number of turns in the session"
    }
    fn category(&self) -> MetricCategory {
        MetricCategory::Pattern
    }
    fn unit(&self) -> &str {
        "turns"
    }
    fn compute(&self, ctx: &MetricContext) -> MetricValue {
        MetricValue::Count(ctx.turns.len() as i64)
    }
}

pub struct SessionDurationMins;
impl Metric for SessionDurationMins {
    fn id(&self) -> &str {
        "session_duration_mins"
    }
    fn name(&self) -> &str {
        "Session Duration"
    }
    fn description(&self) -> &str {
        "Minutes from first to last turn"
    }
    fn category(&self) -> MetricCategory {
        MetricCategory::Pattern
    }
    fn unit(&self) -> &str {
        "min"
    }
    fn compute(&self, ctx: &MetricContext) -> MetricValue {
        if ctx.turns.len() < 2 {
            return MetricValue::Unavailable;
        }
        let first = ctx.turns.iter().map(|t| t.timestamp).min().unwrap_or(0);
        let last = ctx.turns.iter().map(|t| t.timestamp).max().unwrap_or(0);
        MetricValue::Duration((last - first) as f64 / 60_000.0)
    }
    fn format(&self, value: &MetricValue) -> String {
        match value {
            MetricValue::Duration(d) => format!("{:.0}min", d),
            _ => "—".to_string(),
        }
    }
}

pub struct WasteScore;
impl Metric for WasteScore {
    fn id(&self) -> &str {
        "waste_score"
    }
    fn name(&self) -> &str {
        "Waste Score"
    }
    fn description(&self) -> &str {
        "Session inefficiency score (0=clean, 100=wasteful)"
    }
    fn category(&self) -> MetricCategory {
        MetricCategory::Pattern
    }
    fn unit(&self) -> &str {
        "%"
    }
    fn compute(&self, ctx: &MetricContext) -> MetricValue {
        if ctx.turns.is_empty() {
            return MetricValue::Unavailable;
        }
        let score = crate::waste::WasteReport::compute(ctx).waste_score;
        MetricValue::Percentage(score)
    }
}
