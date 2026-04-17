use scopeon_core::{DailyRollup, InteractionEvent, Session, TaskRun, ToolCall, Turn};

#[derive(Debug, Clone, PartialEq)]
pub enum MetricValue {
    Count(i64),
    Float(f64),
    Percentage(f64),
    Currency(f64),
    Duration(f64),
    Text(String),
    Series(Vec<(String, f64)>),
    Unavailable,
}

pub struct MetricContext<'a> {
    pub turns: &'a [Turn],
    pub session: Option<&'a Session>,
    pub daily_rollups: &'a [DailyRollup],
    pub provider_name: &'a str,
    pub tool_calls: &'a [ToolCall],
    pub interaction_events: &'a [InteractionEvent],
    pub task_runs: &'a [TaskRun],
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MetricCategory {
    Cache,
    Cost,
    Velocity,
    Quality,
    Pattern,
    Session,
}

pub trait Metric: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn category(&self) -> MetricCategory;
    fn unit(&self) -> &str;
    fn compute(&self, ctx: &MetricContext) -> MetricValue;
    fn format(&self, value: &MetricValue) -> String {
        match value {
            MetricValue::Count(n) => format!("{}", n),
            MetricValue::Float(f) => format!("{:.2}", f),
            MetricValue::Percentage(p) => format!("{:.1}%", p),
            MetricValue::Currency(c) => format!("${:.4}", c),
            MetricValue::Duration(d) => format!("{:.0}ms", d),
            MetricValue::Text(t) => t.clone(),
            MetricValue::Series(_) => "[series]".to_string(),
            MetricValue::Unavailable => "—".to_string(),
        }
    }
}
