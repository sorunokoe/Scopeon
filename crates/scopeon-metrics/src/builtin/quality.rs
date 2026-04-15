use crate::metric::{Metric, MetricCategory, MetricContext, MetricValue};

pub struct ThinkingRatio;
impl Metric for ThinkingRatio {
    fn id(&self) -> &str {
        "thinking_ratio"
    }
    fn name(&self) -> &str {
        "Thinking Ratio"
    }
    fn description(&self) -> &str {
        "Thinking tokens as % of output — higher means more reasoning"
    }
    fn category(&self) -> MetricCategory {
        MetricCategory::Quality
    }
    fn unit(&self) -> &str {
        "%"
    }
    fn compute(&self, ctx: &MetricContext) -> MetricValue {
        if ctx.turns.is_empty() {
            return MetricValue::Unavailable;
        }
        let thinking: i64 = ctx.turns.iter().map(|t| t.thinking_tokens).sum();
        let output: i64 = ctx.turns.iter().map(|t| t.output_tokens).sum();
        if output == 0 {
            return MetricValue::Unavailable;
        }
        MetricValue::Percentage(thinking as f64 / output as f64 * 100.0)
    }
    fn format(&self, value: &MetricValue) -> String {
        match value {
            MetricValue::Percentage(p) => format!("{:.0}%", p),
            _ => "—".to_string(),
        }
    }
}

pub struct ContextEfficiency;
impl Metric for ContextEfficiency {
    fn id(&self) -> &str {
        "context_efficiency"
    }
    fn name(&self) -> &str {
        "Context Efficiency"
    }
    fn description(&self) -> &str {
        "Output tokens per token invested (input + cache read)"
    }
    fn category(&self) -> MetricCategory {
        MetricCategory::Quality
    }
    fn unit(&self) -> &str {
        "%"
    }
    fn compute(&self, ctx: &MetricContext) -> MetricValue {
        if ctx.turns.is_empty() {
            return MetricValue::Unavailable;
        }
        let output: i64 = ctx.turns.iter().map(|t| t.output_tokens).sum();
        let invested: i64 = ctx
            .turns
            .iter()
            .map(|t| t.input_tokens + t.cache_read_tokens)
            .sum();
        if invested == 0 {
            return MetricValue::Unavailable;
        }
        MetricValue::Percentage(output as f64 / invested as f64 * 100.0)
    }
}

pub struct McpDensity;
impl Metric for McpDensity {
    fn id(&self) -> &str {
        "mcp_density"
    }
    fn name(&self) -> &str {
        "MCP Density"
    }
    fn description(&self) -> &str {
        "Average MCP tool calls per turn"
    }
    fn category(&self) -> MetricCategory {
        MetricCategory::Quality
    }
    fn unit(&self) -> &str {
        "calls/turn"
    }
    fn compute(&self, ctx: &MetricContext) -> MetricValue {
        if ctx.turns.is_empty() {
            return MetricValue::Unavailable;
        }
        let mcp: i64 = ctx.turns.iter().map(|t| t.mcp_call_count).sum();
        MetricValue::Float(mcp as f64 / ctx.turns.len() as f64)
    }
}
