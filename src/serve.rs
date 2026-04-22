/// `scopeon serve` — Privacy-Filtered HTTP API + WebSocket Dashboard
///
/// Implements TRIZ Solution 2 (Privacy-Preserving Team Relay) and Solution 9
/// (WebSocket Browser Dashboard). Resolves TC-B1 (Privacy vs. Team Visibility)
/// by exposing only an aggregated/anonymized projection of local data on the LAN.
///
/// # Data Tiers (developer-controlled via `--tier`)
///
/// | Tier | Exposed Data |
/// |------|-------------|
/// | 0    | Health only (`GET /health`) |
/// | 1    | Aggregate stats (cost totals, cache hit rate, session count) |
/// | 2    | Per-session metadata (no prompt content) |
/// | 3    | Full metrics (all available fields) |
///
/// # Usage
///
/// ```sh
/// scopeon serve                      # tier 1, port 7771
/// scopeon serve --port 8080          # custom port
/// scopeon serve --tier 2             # expose per-session metadata
/// scopeon serve --tier 0             # health-check only (most private)
/// scopeon serve --lan --tier 1 --secret team-token   # LAN-safe aggregate sharing
/// ```
///
/// # Team Usage
///
/// Each team member runs `scopeon serve --lan --tier 1 --secret <token>` on their machine.
/// A central dashboard (web app or `scopeon status --remote http://...`) polls
/// all instances and renders the aggregated team view.
/// Raw prompts, file paths, and session content NEVER leave the machine.
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Result;
use axum::{
    extract::{
        ws::{Message, WebSocket},
        Query, State, WebSocketUpgrade,
    },
    http::{header, HeaderName, HeaderValue, StatusCode},
    response::{sse::Event, IntoResponse},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::broadcast;
use tokio_stream::StreamExt as _;
use tower_http::cors::CorsLayer;

use scopeon_core::user_config::UserConfig;
use scopeon_core::{derive_hook_effects, interaction_token_total, provider_capabilities, Database};

/// Embedded single-file dashboard served at `GET /`.
static DASHBOARD_HTML: &str = include_str!("dashboard.html");

/// Shared application state threaded through all HTTP handlers.
#[derive(Clone)]
struct ServeState {
    db: Arc<Mutex<Database>>,
    tier: u8,
    started_at: Instant,
    /// Optional shared secret for tiered endpoints.
    /// When set, callers must supply `x-scopeon-token: <secret>` header.
    secret: Option<String>,
    /// Broadcast channel for WebSocket snapshot pushes.
    /// Snapshot task publishes; each WebSocket connection subscribes.
    ws_tx: broadcast::Sender<String>,
    /// §1.2: UserConfig loaded once at startup to avoid repeated disk reads on every heartbeat.
    config: Arc<UserConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct SessionDetailQuery {
    session_id: Option<String>,
    limit: Option<usize>,
}

/// Start the HTTP server.
pub async fn run_serve(
    db: Arc<Mutex<Database>>,
    port: u16,
    tier: u8,
    lan: bool,
    secret: Option<String>,
) -> Result<()> {
    let secret = secret.and_then(|raw| {
        let trimmed = raw.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    });
    if lan && tier >= 1 && secret.is_none() {
        anyhow::bail!(
            "LAN mode with tier {} requires --secret <token> so metrics are not exposed to the whole network",
            tier
        );
    }

    // Broadcast channel: snapshot task → all active WebSocket clients.
    // Capacity of 8 means up to 8 snapshots can be queued before slow clients
    // start receiving `RecvError::Lagged` and are silently skipped.
    let (ws_tx, _) = broadcast::channel::<String>(8);
    let ws_tx_snap = ws_tx.clone();

    let state = ServeState {
        db: db.clone(),
        tier,
        started_at: Instant::now(),
        secret: secret.clone(),
        ws_tx,
        config: Arc::new(UserConfig::load()),
    };

    // Background task: periodically build a snapshot and broadcast to WS clients.
    let db_snap = db.clone();
    let config_snap = state.config.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            // Skip DB acquisition entirely if no WebSocket clients are listening.
            if ws_tx_snap.receiver_count() == 0 {
                continue;
            }
            let snap = match db_snap.lock() {
                Ok(g) => build_ws_snapshot(&g, &config_snap),
                Err(_) => continue,
            };
            if let Ok(json) = serde_json::to_string(&snap) {
                // Ignore error if no subscribers (no WS clients connected).
                let _ = ws_tx_snap.send(json);
            }
        }
    });

    // CORS — §2.2: In LAN mode the dashboard is served from http://<machine-ip>:<port>;
    // the browser's Origin header will be that IP address, not localhost. Using a
    // localhost-only allowlist silently blocks all LAN dashboard API calls.
    // Use permissive() in LAN mode; auth is enforced separately via x-scopeon-token.
    // In localhost-only mode, allow both bare hostname and port-specific origins for
    // clients that send a port in their Origin header (some browsers/tools do).
    let auth_header = HeaderName::from_static("x-scopeon-token");
    let cors = if lan {
        tower_http::cors::CorsLayer::permissive()
    } else {
        CorsLayer::new()
            .allow_origin([
                "http://localhost"
                    .parse::<axum::http::HeaderValue>()
                    .unwrap(),
                format!("http://localhost:{port}").parse().unwrap(),
                "http://127.0.0.1".parse().unwrap(),
                format!("http://127.0.0.1:{port}").parse().unwrap(),
            ])
            .allow_methods([axum::http::Method::GET])
            .allow_headers([header::CONTENT_TYPE, auth_header])
    };

    let app = Router::new()
        .route("/", get(handle_dashboard))
        .route("/health", get(handle_health))
        .route("/metrics", get(handle_prometheus))
        .route("/api/v1/stats", get(handle_stats))
        .route("/api/v1/budget", get(handle_budget))
        .route("/api/v1/sessions", get(handle_sessions))
        .route("/api/v1/context", get(handle_context))
        .route("/api/v1/interactions", get(handle_interactions))
        .route("/api/v1/tasks", get(handle_tasks))
        .route(
            "/api/v1/provider-capabilities",
            get(handle_provider_capabilities),
        )
        .route("/ws/v1/metrics", get(handle_ws))
        .route("/sse/v1/status", get(handle_sse_status))
        .layer(cors)
        .with_state(state);

    // Bind to 127.0.0.1 by default (localhost only) for privacy.
    // Use --lan to bind to 0.0.0.0 for intentional LAN team sharing.
    let bind_ip: [u8; 4] = if lan { [0, 0, 0, 0] } else { [127, 0, 0, 1] };
    let addr: SocketAddr = (bind_ip, port).into();
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    let bind_display = if lan {
        "0.0.0.0 (LAN mode)"
    } else {
        "127.0.0.1 (localhost only)"
    };

    tracing::info!(
        "Scopeon HTTP API listening on http://{}  (tier {}, bind {})",
        addr,
        tier,
        bind_display
    );
    eprintln!("\n🔬 Scopeon serve — Privacy-Filtered API + Live Dashboard");
    eprintln!("   Dashboard: http://localhost:{port}");
    eprintln!("   Bind:      {bind_display}");
    eprintln!("   Tier: {tier} ({})", tier_description(tier));
    if secret.is_some() {
        eprintln!("   Auth:      x-scopeon-token required for tiered endpoints");
    }
    eprintln!("   Endpoints:");
    eprintln!("     GET /                    — browser dashboard (WebSocket live)");
    eprintln!("     GET /health              — always available");
    eprintln!("     GET /metrics             — Prometheus text exposition (tier 1+)");
    if tier >= 1 {
        eprintln!("     GET /api/v1/stats        — aggregate token & cost totals");
        eprintln!("     GET /api/v1/budget        — daily/weekly/monthly spend");
        eprintln!("     WS  /ws/v1/metrics        — live WebSocket metrics stream");
        eprintln!("     GET /sse/v1/status        — IDE status stream (SSE, tier 1+)");
    }
    if tier >= 2 {
        eprintln!("     GET /api/v1/context       — context pressure (latest session)");
        eprintln!("     GET /api/v1/sessions      — per-session metadata");
    }
    if tier >= 3 {
        eprintln!("     GET /api/v1/interactions  — detailed interaction provenance");
        eprintln!("     GET /api/v1/tasks         — task and subagent history");
        eprintln!("     GET /api/v1/provider-capabilities — provider provenance support matrix");
    }
    eprintln!();

    axum::serve(listener, app).await?;
    Ok(())
}

fn tier_description(tier: u8) -> &'static str {
    match tier {
        0 => "health-check only",
        1 => "aggregate stats",
        2 => "per-session metadata",
        _ => "full metrics",
    }
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// `GET /` — serve the embedded single-file browser dashboard.
async fn handle_dashboard() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        )],
        DASHBOARD_HTML,
    )
}

/// `GET /metrics` — Prometheus text-format exposition endpoint.
///
/// §2.3: /metrics exposes aggregate cost and session data — equivalent to tier-1 data.
/// Apply a tier-0 gate: return 403 with a hint when the server is configured to
/// expose health only. This maintains the privacy guarantee even when serving LAN.
async fn handle_prometheus(
    State(state): State<ServeState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    if let Some(response) = tier_access_guard(&state, &headers, 1) {
        return response;
    }
    let daily_usd_limit = state.config.budget.daily_usd;
    let result = with_db(&state.db, move |db| {
        let global = db.get_global_stats()?;
        let rollups = db.get_daily_rollups(7)?;
        let sessions = db.list_sessions(1)?;
        let latest_session = sessions.first();

        // Context pressure from the latest session's last turn (§1.1: targeted query, no full scan).
        let ctx_pct = if let Some(session) = latest_session {
            db.get_last_turn_for_session(&session.id)
                .ok()
                .flatten()
                .map(|t| {
                    // §8.2: use stored context window if available (parsed from JSONL max_tokens)
                    let (pct, _) = scopeon_core::context_pressure_with_window(
                        &session.model,
                        t.input_tokens + t.cache_read_tokens,
                        session.context_window_tokens,
                    );
                    pct
                })
                .unwrap_or(0.0)
        } else {
            0.0
        };

        // rollups is in ascending chronological order (oldest first, today last).
        let today_cost: f64 = rollups.last().map(|r| r.estimated_cost_usd).unwrap_or(0.0);
        let week_cost: f64 = rollups
            .iter()
            .rev()
            .take(7)
            .map(|r| r.estimated_cost_usd)
            .sum();

        // Canonical cache hit rate: read / (input + read + write).
        let cache_hit = scopeon_core::cache_hit_rate(
            global.total_input_tokens,
            global.total_cache_read_tokens,
            global.total_cache_write_tokens,
        );

        // §1.2: Use cached config from ServeState — no disk read per heartbeat.
        let daily_limit = daily_usd_limit;
        let daily_used_pct = if daily_limit > 0.0 {
            (today_cost / daily_limit * 100.0).clamp(0.0, 200.0)
        } else {
            0.0
        };

        Ok(format!(
            "# HELP scopeon_context_fill_pct Context window fill percentage (0-100)\n\
             # TYPE scopeon_context_fill_pct gauge\n\
             scopeon_context_fill_pct {ctx_pct:.2}\n\
             # HELP scopeon_cost_usd_today Estimated cost in USD for today\n\
             # TYPE scopeon_cost_usd_today gauge\n\
             scopeon_cost_usd_today {today_cost:.6}\n\
             # HELP scopeon_cost_usd_week Estimated cost in USD for the last 7 days\n\
             # TYPE scopeon_cost_usd_week gauge\n\
             scopeon_cost_usd_week {week_cost:.6}\n\
             # HELP scopeon_cache_hit_rate Cache hit ratio (0.0-1.0) across all sessions\n\
             # TYPE scopeon_cache_hit_rate gauge\n\
             scopeon_cache_hit_rate {cache_hit:.4}\n\
             # HELP scopeon_budget_daily_used_pct Percentage of daily budget consumed\n\
             # TYPE scopeon_budget_daily_used_pct gauge\n\
             scopeon_budget_daily_used_pct {daily_used_pct:.2}\n\
             # HELP scopeon_total_sessions Total number of AI sessions recorded\n\
             # TYPE scopeon_total_sessions counter\n\
             scopeon_total_sessions {}\n\
             # HELP scopeon_total_turns Total number of turns recorded\n\
             # TYPE scopeon_total_turns counter\n\
             scopeon_total_turns {}\n\
             # HELP scopeon_total_cost_usd Lifetime total cost in USD\n\
             # TYPE scopeon_total_cost_usd counter\n\
             scopeon_total_cost_usd {:.6}\n\
             # HELP scopeon_cache_savings_usd Lifetime cache savings in USD\n\
             # TYPE scopeon_cache_savings_usd counter\n\
             scopeon_cache_savings_usd {:.6}\n",
            global.total_sessions,
            global.total_turns,
            global.estimated_cost_usd,
            global.cache_savings_usd,
        ))
    })
    .await;

    match result {
        Ok(body) => (
            StatusCode::OK,
            [(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
            )],
            body,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"))],
            format!("# ERROR: {}\n", e),
        )
            .into_response(),
    }
}

/// `GET /health` — always available regardless of tier.
/// Returns: `{ status, version, uptime_secs, tier }`
async fn handle_health(State(state): State<ServeState>) -> impl IntoResponse {
    let uptime = state.started_at.elapsed().as_secs();
    json_response(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_secs": uptime,
        "tier": state.tier,
    }))
}

fn resolve_session_id_http(db: &Database, session_id: Option<&str>) -> anyhow::Result<String> {
    match session_id {
        Some(id) => Ok(id.to_string()),
        None => db
            .get_latest_session_id()?
            .ok_or_else(|| anyhow::anyhow!("No sessions found.")),
    }
}

/// `GET /api/v1/stats` — aggregate token & cost totals (tier ≥ 1).
async fn handle_stats(
    State(state): State<ServeState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    if let Some(response) = tier_access_guard(&state, &headers, 1) {
        return response;
    }
    let result = with_db(&state.db, move |db| {
        db.get_global_stats().map(|stats| {
            json!({
                "total_sessions":       stats.total_sessions,
                "total_turns":          stats.total_turns,
                "total_input_tokens":   stats.total_input_tokens,
                "total_output_tokens":  stats.total_output_tokens,
                "total_cache_read_tokens":  stats.total_cache_read_tokens,
                "total_cache_write_tokens": stats.total_cache_write_tokens,
                "estimated_cost_usd":   stats.estimated_cost_usd,
                "cache_savings_usd":    stats.cache_savings_usd,
                "cache_hit_rate":       stats.cache_hit_rate,
            })
        })
    })
    .await;
    match result {
        Ok(v) => json_response(v),
        Err(e) => error_response(e),
    }
}

/// `GET /api/v1/budget` — daily/weekly/monthly spend (tier ≥ 1).
async fn handle_budget(
    State(state): State<ServeState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    if let Some(response) = tier_access_guard(&state, &headers, 1) {
        return response;
    }
    let result = with_db(&state.db, move |db| {
        db.get_daily_rollups(30).map(|rollups| {
            // rollups is oldest-first; today is .last().
            let today_cost: f64 = rollups.last().map(|r| r.estimated_cost_usd).unwrap_or(0.0);
            let week_cost: f64 = rollups
                .iter()
                .rev()
                .take(7)
                .map(|r| r.estimated_cost_usd)
                .sum();
            let month_cost: f64 = rollups.iter().map(|r| r.estimated_cost_usd).sum();
            json!({
                "daily_cost_usd":   today_cost,
                "weekly_cost_usd":  week_cost,
                "monthly_cost_usd": month_cost,
                "daily_rollups_30d": rollups.iter().take(30).map(|r| json!({
                    "date":              r.date,
                    "cost_usd":          r.estimated_cost_usd,
                    "sessions":          r.session_count,
                    "turns":             r.turn_count,
                    "input_tokens":      r.total_input_tokens,
                    "cache_read_tokens": r.total_cache_read_tokens,
                })).collect::<Vec<_>>(),
            })
        })
    })
    .await;
    match result {
        Ok(v) => json_response(v),
        Err(e) => error_response(e),
    }
}

/// `GET /api/v1/sessions` — recent session list (tier ≥ 2).
/// No prompt content exposed — only metadata: id, model, cost, timestamps.
/// Requires `x-scopeon-token` header if server was started with `--secret`.
async fn handle_sessions(
    State(state): State<ServeState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    if let Some(response) = tier_access_guard(&state, &headers, 2) {
        return response;
    }
    let result = with_db(&state.db, move |db| {
        db.list_sessions(20).map(|sessions| {
            let list: Vec<Value> = sessions
                .iter()
                .map(|s| {
                    json!({
                        "id":            s.id,
                        "provider":      s.provider,
                        "provider_version": s.provider_version,
                        "model":         s.model,
                        "project_name":  s.project_name,
                        "total_turns":   s.total_turns,
                        "started_at":    s.started_at,
                        "last_turn_at":  s.last_turn_at,
                        "is_subagent":   s.is_subagent,
                        // cost and git branch only in tier 2+
                        "git_branch":    s.git_branch,
                    })
                })
                .collect();
            json!({ "sessions": list })
        })
    })
    .await;
    match result {
        Ok(v) => json_response(v),
        Err(e) => error_response(e),
    }
}

/// `GET /api/v1/context` — latest session context pressure (tier ≥ 2).
/// Requires `x-scopeon-token` header if server was started with `--secret`.
async fn handle_context(
    State(state): State<ServeState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    if let Some(response) = tier_access_guard(&state, &headers, 2) {
        return response;
    }
    let result = with_db(&state.db, move |db| {
        db.list_sessions(1).map(|sessions| {
            if let Some(session) = sessions.first() {
                let model = &session.model;
                // §8.2: Use stored context window if available; fall back to model-prefix table.
                let window = session.context_window_tokens
                    .unwrap_or_else(|| scopeon_core::context_window_for_model(model));
                // §1.1: Use targeted single-row query instead of full session stats.
                let last_tokens = db
                    .get_last_turn_for_session(&session.id)
                    .ok()
                    .flatten()
                    .map(|t| t.input_tokens + t.cache_read_tokens)
                    .unwrap_or(0);
                let (fill_pct, remaining) = scopeon_core::context_pressure_with_window(
                    model,
                    last_tokens,
                    session.context_window_tokens,
                );
                json!({
                    "session_id":      session.id,
                    "model":           model,
                    "context_window":  window,
                    "fill_pct":        fill_pct,
                    "tokens_remaining": remaining,
                    "pressure_level":  if fill_pct >= 95.0 { "critical" } else if fill_pct >= 80.0 { "high" } else { "normal" },
                })
            } else {
                json!({ "message": "No sessions found" })
            }
        })
    }).await;
    match result {
        Ok(v) => json_response(v),
        Err(e) => error_response(e),
    }
}

/// `GET /api/v1/interactions` — detailed interaction provenance for a session (tier ≥ 3).
async fn handle_interactions(
    State(state): State<ServeState>,
    Query(query): Query<SessionDetailQuery>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    if let Some(response) = tier_access_guard(&state, &headers, 3) {
        return response;
    }
    let result = with_db(&state.db, move |db| {
        let sid = resolve_session_id_http(db, query.session_id.as_deref())?;
        let session = db
            .get_session(&sid)?
            .ok_or_else(|| anyhow::anyhow!("Session metadata unavailable."))?;
        let capabilities = provider_capabilities(&session.provider);
        let events = db.list_interaction_events_for_session(&sid, query.limit.unwrap_or(100))?;
        let hook_effects = derive_hook_effects(&events);
        let payload: Vec<Value> = events
            .iter()
            .map(|event| {
                json!({
                    "id": event.id,
                    "timestamp": event.timestamp,
                    "turn_id": event.turn_id,
                    "task_run_id": event.task_run_id,
                    "correlation_id": event.correlation_id,
                    "parent_id": event.parent_id,
                    "kind": event.kind,
                    "phase": event.phase,
                    "name": event.name,
                    "display_name": event.display_name,
                    "provider": event.provider,
                    "status": event.status,
                    "success": event.success,
                    "hook_type": event.hook_type,
                    "agent_type": event.agent_type,
                    "execution_mode": event.execution_mode,
                    "model": event.model,
                    "mcp": {
                        "server": event.mcp_server,
                        "tool": event.mcp_tool,
                    },
                    "sizes": {
                        "input_chars": event.input_size_chars,
                        "output_chars": event.output_size_chars,
                        "prompt_chars": event.prompt_size_chars,
                        "summary_chars": event.summary_size_chars,
                    },
                    "tokens": {
                        "total": event.total_tokens,
                        "estimated_input": event.estimated_input_tokens,
                        "estimated_output": event.estimated_output_tokens,
                        "attributed_total": interaction_token_total(event),
                        "confidence": event.confidence,
                    },
                    "duration_ms": event.duration_ms,
                    "tool_calls": event.total_tool_calls,
                    "hook_effect": hook_effects.get(&event.id),
                })
            })
            .collect();
        Ok(json!({
            "session_id": sid,
            "provider": {
                "name": session.provider,
                "version": session.provider_version,
                "model": session.model,
            },
            "capabilities": capabilities,
            "hook_effect_summary": {
                "modified": hook_effects.values().filter(|effect| effect.as_str() == "modified").count(),
                "blocked": hook_effects.values().filter(|effect| effect.as_str() == "blocked").count(),
                "pass_through": hook_effects.values().filter(|effect| effect.as_str() == "pass_through").count(),
            },
            "count": payload.len(),
            "events": payload,
        }))
    })
    .await;
    match result {
        Ok(v) => json_response(v),
        Err(e) => error_response(e),
    }
}

/// `GET /api/v1/tasks` — task and subagent history for a session (tier ≥ 3).
async fn handle_tasks(
    State(state): State<ServeState>,
    Query(query): Query<SessionDetailQuery>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    if let Some(response) = tier_access_guard(&state, &headers, 3) {
        return response;
    }
    let result = with_db(&state.db, move |db| {
        let sid = resolve_session_id_http(db, query.session_id.as_deref())?;
        let session = db
            .get_session(&sid)?
            .ok_or_else(|| anyhow::anyhow!("Session metadata unavailable."))?;
        let capabilities = provider_capabilities(&session.provider);
        let tasks = db.list_recent_task_runs_for_session(&sid, query.limit.unwrap_or(25))?;
        let events = db.list_interaction_events_for_session(&sid, 10_000)?;

        let payload: Vec<Value> = tasks
            .iter()
            .map(|task| {
                let task_events: Vec<_> = events
                    .iter()
                    .filter(|event| event.task_run_id.as_deref() == Some(task.id.as_str()))
                    .collect();
                let mut by_kind = std::collections::BTreeMap::<String, usize>::new();
                let mut tools_used = std::collections::BTreeSet::<String>::new();
                let mut mcp_tools_used = std::collections::BTreeSet::<String>::new();

                for event in &task_events {
                    *by_kind.entry(event.kind.clone()).or_default() += 1;
                    if matches!(event.kind.as_str(), "tool" | "task" | "skill") {
                        tools_used.insert(event.name.clone());
                    }
                    if event.kind == "mcp" {
                        let label = match (&event.mcp_server, &event.mcp_tool) {
                            (Some(server), Some(tool)) => format!("{server}.{tool}"),
                            _ => event.name.clone(),
                        };
                        mcp_tools_used.insert(label);
                    }
                }

                json!({
                    "id": task.id,
                    "correlation_id": task.correlation_id,
                    "name": task.name,
                    "display_name": task.display_name,
                    "agent_type": task.agent_type,
                    "execution_mode": task.execution_mode,
                    "requested_model": task.requested_model,
                    "actual_model": task.actual_model,
                    "started_at": task.started_at,
                    "completed_at": task.completed_at,
                    "duration_ms": task.duration_ms,
                    "success": task.success,
                    "tokens": {
                        "total": task.total_tokens,
                        "confidence": task.confidence,
                    },
                    "tool_calls": task.total_tool_calls,
                    "payload_sizes": {
                        "description_chars": task.description_size_chars,
                        "prompt_chars": task.prompt_size_chars,
                        "summary_chars": task.summary_size_chars,
                    },
                    "interaction_summary": {
                        "count": task_events.len(),
                        "by_kind": by_kind,
                        "tools_used": tools_used.into_iter().collect::<Vec<_>>(),
                        "mcp_tools_used": mcp_tools_used.into_iter().collect::<Vec<_>>(),
                    },
                })
            })
            .collect();

        Ok(json!({
            "session_id": sid,
            "provider": {
                "name": session.provider,
                "version": session.provider_version,
                "model": session.model,
            },
            "capabilities": capabilities,
            "count": payload.len(),
            "tasks": payload,
        }))
    })
    .await;
    match result {
        Ok(v) => json_response(v),
        Err(e) => error_response(e),
    }
}

/// `GET /api/v1/provider-capabilities` — provider provenance support matrix (tier ≥ 3).
async fn handle_provider_capabilities(
    State(state): State<ServeState>,
    Query(query): Query<SessionDetailQuery>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    if let Some(response) = tier_access_guard(&state, &headers, 3) {
        return response;
    }
    let result = with_db(&state.db, move |db| {
        let sid = resolve_session_id_http(db, query.session_id.as_deref())?;
        let session = db
            .get_session(&sid)?
            .ok_or_else(|| anyhow::anyhow!("Session metadata unavailable."))?;
        let capabilities = provider_capabilities(&session.provider);
        Ok(json!({
            "session_id": sid,
            "provider": {
                "name": session.provider,
                "version": session.provider_version,
                "model": session.model,
            },
            "support_summary": {
                "exact": capabilities.iter().filter(|c| c.level == "exact").count(),
                "estimated": capabilities.iter().filter(|c| c.level == "estimated").count(),
                "unsupported": capabilities.iter().filter(|c| c.level == "unsupported").count(),
            },
            "capabilities": capabilities,
        }))
    })
    .await;
    match result {
        Ok(v) => json_response(v),
        Err(e) => error_response(e),
    }
}

// ── WebSocket handler ─────────────────────────────────────────────────────────

/// `GET /ws/v1/metrics` — real-time WebSocket stream.
///
/// On connect, immediately sends one snapshot. Thereafter broadcasts every
/// ~2 seconds when the background snapshot task publishes. Uses the same tier
/// model as the REST endpoints (min tier 1).
async fn handle_ws(
    ws: WebSocketUpgrade,
    State(state): State<ServeState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    if let Some(response) = ws_access_guard(&state, &headers) {
        return response;
    }
    ws.on_upgrade(move |socket| ws_handler(socket, state))
}

async fn ws_handler(mut socket: WebSocket, state: ServeState) {
    // Send one immediate snapshot on connect.
    let initial = {
        match state.db.lock() {
            Ok(g) => {
                serde_json::to_string(&build_ws_snapshot(&g, &state.config)).unwrap_or_default()
            },
            Err(_) => return,
        }
    };
    if socket.send(Message::Text(initial.into())).await.is_err() {
        return;
    }

    let mut rx = state.ws_tx.subscribe();
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Ok(json) => {
                    if socket.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            },
            // Handle client-initiated close or ping/pong.
            client_msg = socket.recv() => {
                match client_msg {
                    None | Some(Err(_)) => break, // connection closed
                    Some(Ok(Message::Close(_))) => break,
                    Some(Ok(Message::Ping(b))) => {
                        let _ = socket.send(Message::Pong(b)).await;
                    }
                    _ => {} // text/binary from client ignored
                }
            }
        }
    }
}

/// `GET /sse/v1/status` — Server-Sent Events stream for IDE extensions and lightweight clients.
///
/// S-4 (TRIZ): resolves TC-B3 (Observability vs. Agent Token Budget) by delivering
/// real-time context-pressure status to IDEs using SSE — a persistent HTTP connection
/// that sends compact events without the agent spending any tokens to poll.
///
/// Each event is a compact JSON object (fill_pct, daily_cost_usd, cache_hit_rate_pct,
/// should_compact, predicted_turns_remaining). Powered by the same broadcast channel
/// as the WebSocket endpoint — zero additional DB queries after initial connection.
///
/// Requires tier ≥ 1. Respects the same `x-scopeon-token` auth header as other
/// tiered endpoints.
async fn handle_sse_status(
    State(state): State<ServeState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    if let Some(response) = tier_access_guard(&state, &headers, 1) {
        return response;
    }

    let mut rx = state.ws_tx.subscribe();

    // Build a compact status object from the full WS snapshot.
    fn extract_status(snap: &Value) -> Value {
        let data = snap.get("data").unwrap_or(snap);
        let fill_pct = data
            .pointer("/context_pressure/fill_pct")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let daily_cost = data
            .pointer("/budget_status/daily_spent")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let cache_hit = data
            .pointer("/cache_efficiency/cache_hit_rate")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let predicted = data
            .pointer("/context_pressure/predicted_turns_remaining")
            .and_then(Value::as_i64);
        let should_compact = fill_pct >= 80.0;
        serde_json::json!({
            "fill_pct": (fill_pct * 10.0).round() / 10.0,
            "daily_cost_usd": (daily_cost * 10000.0).round() / 10000.0,
            "cache_hit_rate_pct": (cache_hit * 10.0).round() / 10.0,
            "predicted_turns_remaining": predicted,
            "should_compact": should_compact,
        })
    }

    // Send one immediate event on connect using the current WS broadcast value.
    let initial_json = match state.db.lock() {
        Ok(g) => {
            let snap = build_ws_snapshot(&g, &state.config);
            serde_json::to_string(&extract_status(&snap)).unwrap_or_default()
        },
        Err(_) => String::new(),
    };

    // Channel to pipe events into the SSE body stream.
    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

    if !initial_json.is_empty() {
        let _ = event_tx.send(initial_json);
    }

    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(json_str) => {
                    if let Ok(snap) = serde_json::from_str::<Value>(&json_str) {
                        let status =
                            serde_json::to_string(&extract_status(&snap)).unwrap_or_default();
                        if event_tx.send(status).is_err() {
                            break;
                        }
                    }
                },
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Convert the unbounded receiver into a byte stream formatted as SSE.
    let stream = tokio_stream::wrappers::UnboundedReceiverStream::new(event_rx)
        .map(|data| Ok::<Event, std::convert::Infallible>(Event::default().data(data)));

    axum::response::sse::Sse::new(stream)
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response()
}

/// Build the JSON payload sent over the WebSocket snapshot channel.
/// §1.2: config is passed from ServeState — no per-call disk reads.
fn build_ws_snapshot(db: &Database, config: &UserConfig) -> Value {
    let global = db.get_global_stats().ok();
    let rollups = db.get_daily_rollups(30).unwrap_or_default();
    let sessions = db.list_sessions(1).unwrap_or_default();

    let ctx = if let Some(s) = sessions.first() {
        let model = &s.model;
        // §8.2: Use stored context window if available; fall back to model-prefix table.
        let window = s
            .context_window_tokens
            .unwrap_or_else(|| scopeon_core::context_window_for_model(model));
        // §1.1: Use targeted single-row query for context pressure — avoids loading all turns.
        let last_tokens = db
            .get_last_turn_for_session(&s.id)
            .ok()
            .flatten()
            .map(|t| t.input_tokens + t.cache_read_tokens)
            .unwrap_or(0);
        let (fill_pct, remaining) =
            scopeon_core::context_pressure_with_window(model, last_tokens, s.context_window_tokens);
        json!({
            "model": model,
            "context_window": window,
            "fill_pct": fill_pct,
            "tokens_remaining": remaining,
            "predicted_turns_remaining": serde_json::Value::Null,
        })
    } else {
        json!({})
    };

    let today = chrono::Local::now().date_naive();
    let week_start = today
        - chrono::Duration::days(chrono::Datelike::weekday(&today).num_days_from_monday() as i64);
    let (daily_spent, weekly_spent, monthly_spent) =
        rollups
            .iter()
            .fold((0.0f64, 0.0f64, 0.0f64), |(d, w, m), r| {
                let cost = r.estimated_cost_usd;
                // Skip rows with corrupt cost values to prevent NaN/Inf poisoning
                // budget comparisons and alert thresholds.
                if !cost.is_finite() || cost < 0.0 {
                    return (d, w, m);
                }
                let date = chrono::NaiveDate::parse_from_str(&r.date, "%Y-%m-%d").unwrap_or(today);
                (
                    d + if date == today { cost } else { 0.0 },
                    w + if date >= week_start { cost } else { 0.0 },
                    m + cost,
                )
            });

    let budget = &config.budget;
    let daily_limit = budget.daily_usd;

    let tok = global
        .as_ref()
        .map(|g| {
            json!({
                "total_sessions": g.total_sessions,
                "total_turns": g.total_turns,
                "total_input_tokens": g.total_input_tokens,
                "total_output_tokens": g.total_output_tokens,
                "total_cache_read_tokens": g.total_cache_read_tokens,
                "total_cache_write_tokens": g.total_cache_write_tokens,
                "total_thinking_tokens": 0,
                "estimated_cost_usd": g.estimated_cost_usd,
            })
        })
        .unwrap_or(json!({}));

    let cache = global
        .as_ref()
        .map(|g| {
            json!({
                "cache_hit_rate": g.cache_hit_rate,
                "cache_savings_usd": g.cache_savings_usd,
            })
        })
        .unwrap_or(json!({}));

    let hist: Vec<Value> = rollups
        .iter()
        .map(|r| {
            json!({
                "date": r.date,
                "estimated_cost_usd": r.estimated_cost_usd,
            })
        })
        .collect();

    json!({
        "type": "snapshot",
        "version": env!("CARGO_PKG_VERSION"),
        "data": {
            "context_pressure": ctx,
            "budget_status": {
                "daily_spent": daily_spent,
                "daily_limit": daily_limit,
                "weekly_spent": weekly_spent,
                "monthly_spent": monthly_spent,
                "over_budget": daily_limit > 0.0 && daily_spent > daily_limit,
            },
            "token_usage": tok,
            "cache_efficiency": cache,
            "history_30d": hist,
        }
    })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// §3.2: Offload synchronous SQLite calls to the blocking thread pool so they
/// don't starve the Tokio async executor. All HTTP handlers call this.
async fn with_db<T, F>(db: &Arc<Mutex<Database>>, f: F) -> anyhow::Result<T>
where
    T: Send + 'static,
    F: FnOnce(&Database) -> anyhow::Result<T> + Send + 'static,
{
    let db = db.clone();
    tokio::task::spawn_blocking(move || {
        let guard = db
            .lock()
            .map_err(|_| anyhow::anyhow!("Database mutex poisoned"))?;
        f(&guard)
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking join error: {}", e))?
}

fn json_response(value: Value) -> axum::response::Response {
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )],
        Json(value),
    )
        .into_response()
}

fn error_response(e: anyhow::Error) -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )],
        Json(json!({ "error": e.to_string() })),
    )
        .into_response()
}

fn tier_denied(required: u8) -> axum::response::Response {
    (
        StatusCode::FORBIDDEN,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )],
        Json(json!({
            "error": format!(
                "This endpoint requires tier {} or higher. \
                 Restart with `scopeon serve --tier {}`",
                required, required
            )
        })),
    )
        .into_response()
}

/// Returns a 401 Unauthorized response when a required `--secret` token is missing or wrong.
fn unauthorized_response() -> axum::response::Response {
    (
        StatusCode::UNAUTHORIZED,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )],
        Json(json!({
            "error": "Missing or invalid x-scopeon-token header. \
                      Supply the token configured with `scopeon serve --secret <token>`."
        })),
    )
        .into_response()
}

fn ws_access_guard(
    state: &ServeState,
    headers: &axum::http::HeaderMap,
) -> Option<axum::response::Response> {
    tier_access_guard(state, headers, 1)
}

fn tier_access_guard(
    state: &ServeState,
    headers: &axum::http::HeaderMap,
    required_tier: u8,
) -> Option<axum::response::Response> {
    if state.tier < required_tier {
        return Some(tier_denied(required_tier));
    }
    if !check_secret(state, headers, required_tier) {
        return Some(unauthorized_response());
    }
    None
}

/// Validates the `x-scopeon-token` header when a secret is configured.
/// Returns `true` if access is allowed (either no secret required, or correct token supplied).
fn check_secret(state: &ServeState, headers: &axum::http::HeaderMap, required_tier: u8) -> bool {
    if state.tier < required_tier {
        return false; // tier check handled separately
    }
    match &state.secret {
        None => true, // no secret configured — open access
        Some(expected) => headers
            .get("x-scopeon-token")
            .and_then(|v| v.to_str().ok())
            .map(|v| constant_time_eq(v, expected))
            .unwrap_or(false),
    }
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();
    let max_len = a_bytes.len().max(b_bytes.len());
    let mut diff: usize = a_bytes.len() ^ b_bytes.len();
    for i in 0..max_len {
        let lhs = *a_bytes.get(i).unwrap_or(&0);
        let rhs = *b_bytes.get(i).unwrap_or(&0);
        diff |= (lhs ^ rhs) as usize;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};

    fn test_state(tier: u8, secret: Option<&str>) -> ServeState {
        let (ws_tx, _) = broadcast::channel(1);
        ServeState {
            db: Arc::new(Mutex::new(Database::open_in_memory().unwrap())),
            tier,
            started_at: Instant::now(),
            secret: secret.map(str::to_string),
            ws_tx,
            config: Arc::new(UserConfig::default()),
        }
    }

    #[test]
    fn websocket_stream_requires_tier_one() {
        let denied = ws_access_guard(&test_state(0, None), &HeaderMap::new()).unwrap();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        assert!(ws_access_guard(&test_state(1, None), &HeaderMap::new()).is_none());
    }

    #[test]
    fn context_endpoint_requires_tier_two() {
        let denied = tier_access_guard(&test_state(1, None), &HeaderMap::new(), 2).unwrap();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn tier_one_endpoint_requires_matching_secret_when_configured() {
        let state = test_state(1, Some("secret-token"));

        let unauthorized = tier_access_guard(&state, &HeaderMap::new(), 1).unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let mut headers = HeaderMap::new();
        headers.insert("x-scopeon-token", HeaderValue::from_static("secret-token"));
        assert!(tier_access_guard(&state, &headers, 1).is_none());
    }

    #[test]
    fn context_endpoint_requires_matching_secret_when_configured() {
        let state = test_state(2, Some("secret-token"));

        let unauthorized = tier_access_guard(&state, &HeaderMap::new(), 2).unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let mut headers = HeaderMap::new();
        headers.insert("x-scopeon-token", HeaderValue::from_static("secret-token"));
        assert!(tier_access_guard(&state, &headers, 2).is_none());
    }
}
