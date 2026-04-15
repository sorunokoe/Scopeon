use crate::metric::{Metric, MetricCategory, MetricContext, MetricValue};

pub struct TotalCostUsd;
impl Metric for TotalCostUsd {
    fn id(&self) -> &str {
        "total_cost_usd"
    }
    fn name(&self) -> &str {
        "Total Cost"
    }
    fn description(&self) -> &str {
        "Total estimated session cost in USD"
    }
    fn category(&self) -> MetricCategory {
        MetricCategory::Cost
    }
    fn unit(&self) -> &str {
        "USD"
    }
    fn compute(&self, ctx: &MetricContext) -> MetricValue {
        if ctx.turns.is_empty() {
            return MetricValue::Unavailable;
        }
        MetricValue::Currency(ctx.turns.iter().map(|t| t.estimated_cost_usd).sum())
    }
}

pub struct CostPerTurn;
impl Metric for CostPerTurn {
    fn id(&self) -> &str {
        "cost_per_turn"
    }
    fn name(&self) -> &str {
        "Cost/Turn"
    }
    fn description(&self) -> &str {
        "Average cost per turn in USD"
    }
    fn category(&self) -> MetricCategory {
        MetricCategory::Cost
    }
    fn unit(&self) -> &str {
        "USD"
    }
    fn compute(&self, ctx: &MetricContext) -> MetricValue {
        if ctx.turns.is_empty() {
            return MetricValue::Unavailable;
        }
        let total: f64 = ctx.turns.iter().map(|t| t.estimated_cost_usd).sum();
        MetricValue::Currency(total / ctx.turns.len() as f64)
    }
}

pub struct CostPerOutputToken;
impl Metric for CostPerOutputToken {
    fn id(&self) -> &str {
        "cost_per_output_token"
    }
    fn name(&self) -> &str {
        "Cost/1k Out"
    }
    fn description(&self) -> &str {
        "Cost per 1k output tokens in USD"
    }
    fn category(&self) -> MetricCategory {
        MetricCategory::Cost
    }
    fn unit(&self) -> &str {
        "USD/1k"
    }
    fn compute(&self, ctx: &MetricContext) -> MetricValue {
        if ctx.turns.is_empty() {
            return MetricValue::Unavailable;
        }
        let total_cost: f64 = ctx.turns.iter().map(|t| t.estimated_cost_usd).sum();
        let total_output: i64 = ctx.turns.iter().map(|t| t.output_tokens).sum();
        if total_output == 0 {
            return MetricValue::Unavailable;
        }
        MetricValue::Currency(total_cost / total_output as f64 * 1000.0)
    }
}

pub struct ProjectedDailyCost;
impl Metric for ProjectedDailyCost {
    fn id(&self) -> &str {
        "projected_daily_cost"
    }
    fn name(&self) -> &str {
        "Projected Daily"
    }
    fn description(&self) -> &str {
        "Extrapolated 24h burn rate at current session pace"
    }
    fn category(&self) -> MetricCategory {
        MetricCategory::Cost
    }
    fn unit(&self) -> &str {
        "USD"
    }
    fn compute(&self, ctx: &MetricContext) -> MetricValue {
        if ctx.turns.len() < 2 {
            return MetricValue::Unavailable;
        }
        let first = ctx.turns.iter().map(|t| t.timestamp).min().unwrap_or(0);
        let last = ctx.turns.iter().map(|t| t.timestamp).max().unwrap_or(0);
        let elapsed_ms = (last - first) as f64;
        if elapsed_ms <= 0.0 {
            return MetricValue::Unavailable;
        }
        let total_cost: f64 = ctx.turns.iter().map(|t| t.estimated_cost_usd).sum();
        let ms_per_day = 24.0 * 60.0 * 60.0 * 1000.0;
        MetricValue::Currency(total_cost / elapsed_ms * ms_per_day)
    }
}

pub struct BurnRatePerHour;
impl Metric for BurnRatePerHour {
    fn id(&self) -> &str {
        "burn_rate_per_hour"
    }
    fn name(&self) -> &str {
        "Burn Rate"
    }
    fn description(&self) -> &str {
        "Estimated cost per hour at current session pace"
    }
    fn category(&self) -> MetricCategory {
        MetricCategory::Cost
    }
    fn unit(&self) -> &str {
        "USD/h"
    }
    fn compute(&self, ctx: &MetricContext) -> MetricValue {
        if ctx.turns.len() < 2 {
            return MetricValue::Unavailable;
        }
        let first = ctx.turns.iter().map(|t| t.timestamp).min().unwrap_or(0);
        let last = ctx.turns.iter().map(|t| t.timestamp).max().unwrap_or(0);
        let elapsed_ms = (last - first) as f64;
        if elapsed_ms <= 0.0 {
            return MetricValue::Unavailable;
        }
        let total_cost: f64 = ctx.turns.iter().map(|t| t.estimated_cost_usd).sum();
        let ms_per_hour = 60.0 * 60.0 * 1000.0;
        MetricValue::Currency(total_cost / elapsed_ms * ms_per_hour)
    }
}
