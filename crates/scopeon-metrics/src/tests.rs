use super::*;
use metric::{MetricContext, MetricValue};
use scopeon_core::Turn;

fn make_turn(
    id: &str,
    input: i64,
    cache_read: i64,
    cache_write: i64,
    output: i64,
    thinking: i64,
    mcp: i64,
    cost: f64,
    ts: i64,
    dur: Option<i64>,
) -> Turn {
    Turn {
        id: id.to_string(),
        session_id: "s1".to_string(),
        turn_index: 0,
        timestamp: ts,
        duration_ms: dur,
        input_tokens: input,
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
        cache_write_5m_tokens: cache_write,
        cache_write_1h_tokens: 0,
        output_tokens: output,
        thinking_tokens: thinking,
        mcp_call_count: mcp,
        mcp_input_token_est: 0,
        text_output_tokens: output - thinking,
        model: "claude-opus-4-5-20251101".to_string(),
        service_tier: "standard".to_string(),
        estimated_cost_usd: cost,
        is_compaction_event: false,
    }
}

fn ctx<'a>(turns: &'a [Turn]) -> MetricContext<'a> {
    MetricContext {
        turns,
        session: None,
        daily_rollups: &[],
        provider_name: "claude-code",
        tool_calls: &[],
    }
}

#[test]
fn test_cache_hit_rate_basic() {
    // input=100, cache_read=500, cache_write=200
    // canonical formula: cache_read / (input + cache_read + cache_write) = 500/800 = 62.5%
    let turns = vec![make_turn("t1", 100, 500, 200, 50, 10, 0, 0.01, 0, None)];
    let val = builtin::cache::CacheHitRate.compute(&ctx(&turns));
    if let MetricValue::Percentage(p) = val {
        assert!((p - 62.5).abs() < 0.01);
    } else {
        panic!("expected Percentage");
    }
}

#[test]
fn test_cache_hit_rate_empty() {
    let val = builtin::cache::CacheHitRate.compute(&ctx(&[]));
    assert_eq!(val, MetricValue::Unavailable);
}

#[test]
fn test_total_cost() {
    let turns = vec![
        make_turn("t1", 100, 0, 0, 50, 0, 0, 0.01, 0, None),
        make_turn("t2", 100, 0, 0, 50, 0, 0, 0.02, 1000, None),
    ];
    let val = builtin::cost::TotalCostUsd.compute(&ctx(&turns));
    if let MetricValue::Currency(c) = val {
        assert!((c - 0.03).abs() < 1e-9);
    } else {
        panic!("expected Currency");
    }
}

#[test]
fn test_thinking_ratio() {
    let turns = vec![make_turn("t1", 100, 0, 0, 100, 40, 0, 0.01, 0, None)];
    let val = builtin::quality::ThinkingRatio.compute(&ctx(&turns));
    if let MetricValue::Percentage(p) = val {
        assert!((p - 40.0).abs() < 1e-9);
    } else {
        panic!("expected Percentage");
    }
}

#[test]
fn test_session_depth() {
    let turns = vec![
        make_turn("t1", 100, 0, 0, 50, 0, 0, 0.01, 0, None),
        make_turn("t2", 100, 0, 0, 50, 0, 0, 0.01, 1000, None),
        make_turn("t3", 100, 0, 0, 50, 0, 0, 0.01, 2000, None),
    ];
    let val = builtin::pattern::SessionDepth.compute(&ctx(&turns));
    assert_eq!(val, MetricValue::Count(3));
}

#[test]
fn test_avg_latency() {
    let turns = vec![
        make_turn("t1", 100, 0, 0, 50, 0, 0, 0.01, 0, Some(1000)),
        make_turn("t2", 100, 0, 0, 50, 0, 0, 0.01, 1000, Some(2000)),
    ];
    let val = builtin::velocity::AvgTurnLatencyMs.compute(&ctx(&turns));
    if let MetricValue::Duration(d) = val {
        assert!((d - 1500.0).abs() < 1e-9);
    } else {
        panic!("expected Duration");
    }
}

#[test]
fn test_registry_default_has_all_metrics() {
    let r = MetricRegistry::default();
    assert!(r.get("cache_hit_rate").is_some());
    assert!(r.get("total_cost_usd").is_some());
    assert!(r.get("token_velocity").is_some());
    assert!(r.get("thinking_ratio").is_some());
    assert!(r.get("session_depth").is_some());
}

// ── Waste detection tests ─────────────────────────────────────────────────────

use crate::waste::{WasteKind, WasteReport};
use scopeon_core::ToolCall;

fn make_tool_call(name: &str, input_chars: i64) -> ToolCall {
    ToolCall {
        id: format!("tc-{}", name),
        session_id: "s1".to_string(),
        turn_id: "t1".to_string(),
        tool_name: name.to_string(),
        input_size_chars: input_chars,
        input_hash: input_chars as u64, // deterministic for tests
        timestamp: 0,
    }
}

fn ctx_with_tools<'a>(turns: &'a [Turn], tools: &'a [ToolCall]) -> MetricContext<'a> {
    MetricContext {
        turns,
        session: None,
        daily_rollups: &[],
        provider_name: "claude-code",
        tool_calls: tools,
    }
}

#[test]
fn test_waste_score_zero_for_clean_session() {
    // Good cache hit rate, low MCP density, no thinking waste → score should be 0
    let turns = vec![
        make_turn("t1", 100, 500, 200, 50, 5, 1, 0.01, 0, None),
        make_turn("t2", 100, 500, 200, 50, 5, 1, 0.01, 1000, None),
        make_turn("t3", 100, 500, 200, 50, 5, 1, 0.01, 2000, None),
        make_turn("t4", 100, 500, 200, 50, 5, 1, 0.01, 3000, None),
    ];
    let report = WasteReport::compute(&ctx_with_tools(&turns, &[]));
    assert_eq!(
        report.waste_score, 0.0,
        "clean session should have zero waste score"
    );
    assert!(report.signals.is_empty());
}

#[test]
fn test_waste_detects_redundant_tool_calls() {
    let turns = vec![make_turn("t1", 100, 0, 0, 50, 0, 2, 0.01, 0, None)];
    // Same tool name + same input size called twice = redundant
    let tools = vec![
        make_tool_call("bash", 42),
        make_tool_call("bash", 42), // exact duplicate
    ];
    let report = WasteReport::compute(&ctx_with_tools(&turns, &tools));
    let has_redundant = report.signals.iter().any(|s| {
        matches!(&s.kind, WasteKind::RedundantToolCalls { tool_name, .. } if tool_name == "bash")
    });
    assert!(
        has_redundant,
        "duplicate tool call should produce RedundantToolCalls signal"
    );
    assert!(report.waste_score > 0.0);
}

#[test]
fn test_waste_detects_thinking_waste() {
    // thinking_tokens >> output_tokens (ratio > 2.0)
    let turns = vec![make_turn("t1", 100, 0, 0, 100, 600, 0, 0.01, 0, None)];
    let report = WasteReport::compute(&ctx_with_tools(&turns, &[]));
    let has_thinking = report
        .signals
        .iter()
        .any(|s| matches!(&s.kind, WasteKind::ThinkingWaste { .. }));
    assert!(
        has_thinking,
        "high thinking:output ratio should produce ThinkingWaste signal"
    );
}

#[test]
fn test_waste_detects_cold_cache_after_turn_3() {
    // After turn 3: cache_read near-zero = cold cache
    let turns: Vec<Turn> = (0..6)
        .map(|i| {
            make_turn(
                &format!("t{}", i),
                1000,
                0,
                0,
                50,
                0,
                0,
                0.01,
                i * 1000,
                None,
            )
        })
        .collect();
    let report = WasteReport::compute(&ctx_with_tools(&turns, &[]));
    let has_cold = report
        .signals
        .iter()
        .any(|s| matches!(&s.kind, WasteKind::ColdCacheSession { .. }));
    assert!(
        has_cold,
        "zero cache reads after turn 3 should produce ColdCacheSession signal"
    );
}

#[test]
fn test_waste_score_severity_weighting() {
    // Critical signals should score 40 each; ensure cap at 100
    // Trigger ThinkingWaste (Warning=20) + ColdCache (Warning=20) = 40
    let turns: Vec<Turn> = (0..6)
        .map(|i| {
            make_turn(
                &format!("t{}", i),
                1000,
                0,
                0,
                50,
                300,
                0,
                0.01,
                i * 1000,
                None,
            )
        })
        .collect();
    let report = WasteReport::compute(&ctx_with_tools(&turns, &[]));
    assert!(
        report.waste_score <= 100.0,
        "waste_score must never exceed 100"
    );
    assert!(
        report.waste_score > 0.0,
        "multiple signals should produce positive score"
    );
}

#[test]
fn test_waste_score_capped_at_100() {
    // Many critical signals — score must cap at 100
    let turns: Vec<Turn> = (0..10)
        .map(|i| {
            make_turn(
                &format!("t{}", i),
                1000,
                0,
                0,
                50,
                5000,
                0,
                0.01,
                i * 1000,
                None,
            )
        })
        .collect();
    let many_tools: Vec<ToolCall> = (0..20)
        .map(|i| make_tool_call("bash", i)) // varied input sizes so not all redundant, but some are
        .collect();
    let report = WasteReport::compute(&ctx_with_tools(&turns, &many_tools));
    assert!(
        report.waste_score <= 100.0,
        "waste_score must be capped at 100"
    );
}

// ── Health score tests ────────────────────────────────────────────────────────

use crate::health::{compute_health_score, health_trend};

#[test]
fn test_health_score_empty_session_returns_neutral() {
    // No turns → all sub-scores return neutral values → should be around 55
    let empty_ctx = MetricContext {
        turns: &[],
        session: None,
        daily_rollups: &[],
        provider_name: "claude-code",
        tool_calls: &[],
    };
    let waste = WasteReport::compute(&empty_ctx);
    let score = compute_health_score(&empty_ctx, &waste);
    // cache=15 (neutral), context=25 (no data), cost=15 (neutral), waste=20 (no signals)
    assert!(
        score >= 50.0 && score <= 80.0,
        "empty session health should be in neutral range, got {}",
        score
    );
}

#[test]
fn test_health_score_perfect_cache_high_score() {
    // 90%+ cache hit rate should produce cache=30 pts
    let turns = vec![
        make_turn("t1", 100, 900, 0, 50, 0, 0, 0.01, 0, None),
        make_turn("t2", 100, 900, 0, 50, 0, 0, 0.01, 1000, None),
    ];
    let ctx = MetricContext {
        turns: &turns,
        session: None,
        daily_rollups: &[],
        provider_name: "claude-code",
        tool_calls: &[],
    };
    let waste = WasteReport::compute(&ctx);
    let score = compute_health_score(&ctx, &waste);
    assert!(
        score >= 70.0,
        "high cache hit rate should produce high health score, got {}",
        score
    );
}

#[test]
fn test_health_score_is_bounded_0_to_100() {
    // Both extremes: no data and rich data should stay in [0, 100]
    let turns = vec![make_turn("t1", 1, 0, 0, 1, 0, 0, 0.0001, 0, None)];
    let ctx = MetricContext {
        turns: &turns,
        session: None,
        daily_rollups: &[],
        provider_name: "claude-code",
        tool_calls: &[],
    };
    let waste = WasteReport::compute(&ctx);
    let score = compute_health_score(&ctx, &waste);
    assert!(
        (0.0..=100.0).contains(&score),
        "health score must be in [0, 100], got {}",
        score
    );
}

#[test]
fn test_health_trend_positive() {
    assert!((health_trend(80.0, 60.0) - 20.0).abs() < 1e-9);
}

#[test]
fn test_health_trend_negative() {
    assert!((health_trend(40.0, 75.0) - (-35.0)).abs() < 1e-9);
}

#[test]
fn test_health_trend_no_change() {
    assert_eq!(health_trend(55.0, 55.0), 0.0);
}
