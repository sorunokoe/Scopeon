use crate::metric::{Metric, MetricCategory, MetricContext, MetricValue};

fn session_duration_mins(ctx: &MetricContext) -> f64 {
    if ctx.turns.len() < 2 {
        return 0.0;
    }
    let first = ctx.turns.iter().map(|t| t.timestamp).min().unwrap_or(0);
    let last = ctx.turns.iter().map(|t| t.timestamp).max().unwrap_or(0);
    (last - first) as f64 / 60_000.0
}

pub struct TokenVelocity;
impl Metric for TokenVelocity {
    fn id(&self) -> &str {
        "token_velocity"
    }
    fn name(&self) -> &str {
        "Token Velocity"
    }
    fn description(&self) -> &str {
        "Total tokens (input + output) per minute"
    }
    fn category(&self) -> MetricCategory {
        MetricCategory::Velocity
    }
    fn unit(&self) -> &str {
        "tok/min"
    }
    fn compute(&self, ctx: &MetricContext) -> MetricValue {
        let mins = session_duration_mins(ctx);
        if mins <= 0.0 {
            return MetricValue::Unavailable;
        }
        let total: i64 = ctx
            .turns
            .iter()
            .map(|t| t.input_tokens + t.output_tokens + t.cache_read_tokens)
            .sum();
        MetricValue::Float(total as f64 / mins)
    }
    fn format(&self, value: &MetricValue) -> String {
        match value {
            MetricValue::Float(f) => {
                if *f >= 1000.0 {
                    format!("{:.1}k tok/min", f / 1000.0)
                } else {
                    format!("{:.0} tok/min", f)
                }
            },
            _ => "—".to_string(),
        }
    }
}

pub struct OutputVelocity;
impl Metric for OutputVelocity {
    fn id(&self) -> &str {
        "output_velocity"
    }
    fn name(&self) -> &str {
        "Output Velocity"
    }
    fn description(&self) -> &str {
        "Output tokens generated per minute"
    }
    fn category(&self) -> MetricCategory {
        MetricCategory::Velocity
    }
    fn unit(&self) -> &str {
        "tok/min"
    }
    fn compute(&self, ctx: &MetricContext) -> MetricValue {
        let mins = session_duration_mins(ctx);
        if mins <= 0.0 {
            return MetricValue::Unavailable;
        }
        let total: i64 = ctx.turns.iter().map(|t| t.output_tokens).sum();
        MetricValue::Float(total as f64 / mins)
    }
}

pub struct TurnsPerHour;
impl Metric for TurnsPerHour {
    fn id(&self) -> &str {
        "turns_per_hour"
    }
    fn name(&self) -> &str {
        "Turns/Hour"
    }
    fn description(&self) -> &str {
        "Session turn rate per hour"
    }
    fn category(&self) -> MetricCategory {
        MetricCategory::Velocity
    }
    fn unit(&self) -> &str {
        "turns/h"
    }
    fn compute(&self, ctx: &MetricContext) -> MetricValue {
        let mins = session_duration_mins(ctx);
        if mins <= 0.0 {
            return MetricValue::Unavailable;
        }
        MetricValue::Float(ctx.turns.len() as f64 / mins * 60.0)
    }
}

pub struct AvgTurnLatencyMs;
impl Metric for AvgTurnLatencyMs {
    fn id(&self) -> &str {
        "avg_turn_latency_ms"
    }
    fn name(&self) -> &str {
        "Avg Latency"
    }
    fn description(&self) -> &str {
        "Average turn processing time in milliseconds"
    }
    fn category(&self) -> MetricCategory {
        MetricCategory::Velocity
    }
    fn unit(&self) -> &str {
        "ms"
    }
    fn compute(&self, ctx: &MetricContext) -> MetricValue {
        let durations: Vec<f64> = ctx
            .turns
            .iter()
            .filter_map(|t| t.duration_ms)
            .map(|d| d as f64)
            .collect();
        if durations.is_empty() {
            return MetricValue::Unavailable;
        }
        MetricValue::Duration(durations.iter().sum::<f64>() / durations.len() as f64)
    }
}

pub struct P95TurnLatencyMs;
impl Metric for P95TurnLatencyMs {
    fn id(&self) -> &str {
        "p95_turn_latency_ms"
    }
    fn name(&self) -> &str {
        "P95 Latency"
    }
    fn description(&self) -> &str {
        "95th percentile turn latency in milliseconds"
    }
    fn category(&self) -> MetricCategory {
        MetricCategory::Velocity
    }
    fn unit(&self) -> &str {
        "ms"
    }
    fn compute(&self, ctx: &MetricContext) -> MetricValue {
        let mut durations: Vec<f64> = ctx
            .turns
            .iter()
            .filter_map(|t| t.duration_ms)
            .map(|d| d as f64)
            .collect();
        if durations.is_empty() {
            return MetricValue::Unavailable;
        }
        durations.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let idx = ((durations.len() as f64 * 0.95) as usize).min(durations.len() - 1);
        MetricValue::Duration(durations[idx])
    }
}
