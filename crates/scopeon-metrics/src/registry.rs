use crate::builtin;
use crate::metric::{Metric, MetricCategory, MetricContext, MetricValue};

pub struct MetricRegistry {
    metrics: Vec<Box<dyn Metric>>,
}

impl Default for MetricRegistry {
    fn default() -> Self {
        let mut r = MetricRegistry {
            metrics: Vec::new(),
        };
        // Cache
        r.register(Box::new(builtin::cache::CacheHitRate));
        r.register(Box::new(builtin::cache::CacheRoi));
        r.register(Box::new(builtin::cache::CacheSavingsUsd));
        r.register(Box::new(builtin::cache::CacheWarmupTurns));
        // Cost
        r.register(Box::new(builtin::cost::TotalCostUsd));
        r.register(Box::new(builtin::cost::CostPerTurn));
        r.register(Box::new(builtin::cost::CostPerOutputToken));
        r.register(Box::new(builtin::cost::ProjectedDailyCost));
        // Velocity
        r.register(Box::new(builtin::velocity::TokenVelocity));
        r.register(Box::new(builtin::velocity::OutputVelocity));
        r.register(Box::new(builtin::velocity::TurnsPerHour));
        r.register(Box::new(builtin::velocity::AvgTurnLatencyMs));
        r.register(Box::new(builtin::velocity::P95TurnLatencyMs));
        // Quality
        r.register(Box::new(builtin::quality::ThinkingRatio));
        r.register(Box::new(builtin::quality::ContextEfficiency));
        r.register(Box::new(builtin::quality::McpDensity));
        // Pattern
        r.register(Box::new(builtin::pattern::ContextGrowthRate));
        r.register(Box::new(builtin::pattern::TurnSizeCv));
        r.register(Box::new(builtin::pattern::SessionDepth));
        r.register(Box::new(builtin::pattern::SessionDurationMins));
        // New cache metrics
        r.register(Box::new(builtin::cache::CompactionCount));
        r.register(Box::new(builtin::cache::ContextPressurePct));
        // New cost metric
        r.register(Box::new(builtin::cost::BurnRatePerHour));
        // New pattern metric
        r.register(Box::new(builtin::pattern::WasteScore));
        r
    }
}

impl MetricRegistry {
    pub fn register(&mut self, metric: Box<dyn Metric>) {
        self.metrics.push(metric);
    }

    pub fn get(&self, id: &str) -> Option<&dyn Metric> {
        self.metrics
            .iter()
            .find(|m| m.id() == id)
            .map(|m| m.as_ref())
    }

    pub fn compute_all(&self, ctx: &MetricContext) -> Vec<(String, MetricValue)> {
        self.metrics
            .iter()
            .map(|m| (m.id().to_string(), m.compute(ctx)))
            .collect()
    }

    pub fn by_category(&self, cat: MetricCategory) -> Vec<&dyn Metric> {
        self.metrics
            .iter()
            .filter(|m| m.category() == cat)
            .map(|m| m.as_ref())
            .collect()
    }

    pub fn all(&self) -> &[Box<dyn Metric>] {
        &self.metrics
    }
}
