# Changelog

All notable changes to Scopeon are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Scopeon follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

### S-8 — Phase 2: OTLP/HTTP JSON Push Exporter (planned, zero new deps)

- `[telemetry]` config section: `otlp_endpoint`, `otlp_interval_secs`, `otlp_headers`.
- New `src/otlp.rs`: `build_otlp_metrics_json(snap: &MetricSnapshot) -> String` serialises
  `MetricSnapshot` into OTLP MetricsData JSON using only `serde_json` (already present).
  Maps `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`,
  `ai.usage.cache_read_tokens`, `ai.cost.usd_today`, `ai.context.fill_pct`, etc.
- Background alert task extended: fires `do_http_post(otlp_endpoint, json)` every
  `otlp_interval_secs` via system `curl` (already used for webhook delivery). Zero overhead
  when `otlp_endpoint` is unset (PC-2 resolved by separation by condition).
- `GET /otlp/v1/metrics` pull endpoint on `scopeon serve` for pull-mode collectors.

### S-8 — Phase 3: Trace Export (planned, zero new deps)

- `scopeon export --format otlp-json` — offline/CI mode; reads SQLite, serialises all
  sessions/turns as OTLP TraceData JSON (sessions = root spans, turns = child spans,
  token counts as `gen_ai.*` span attributes). Pipeable to any OTLP endpoint via curl.
- Optional continuous trace push: completed sessions flushed as OTLP spans to configured
  `otlp_endpoint` alongside metrics push.

---

## [0.7.1] — 2026-04-20

### Added

- **GitHub Copilot CLI MCP integration** — `scopeon init-copilot` writes Scopeon as an MCP
  server into `~/.copilot/mcp-config.json` atomically (backup → tmp → rename).
  Preserves all existing MCP servers; creates the file if absent.
  Copilot can then call `get_token_usage`, `get_session_summary`, `get_cache_efficiency`,
  `get_history`, and `compare_sessions` without any token cost per JSON-RPC §4.
- **`scopeon onboard` auto-detection** — when Copilot CLI is detected the onboarding wizard
  now prompts to configure the MCP server automatically (same flow as Claude Code).
- Two unit tests for `cmd_init_copilot`: create-from-scratch and preserve-existing-servers.

### Fixed

- Install instructions updated to use `--git` variants until the crate is published on
  crates.io: `cargo binstall --git https://github.com/sorunokoe/Scopeon scopeon`
  and `cargo install --git https://github.com/sorunokoe/Scopeon`.

---

## [0.7.0] — 2026-04-17

This release delivers OpenTelemetry integration documentation (S-8 Phase 1), the Copilot CLI
provider parser, semantic turn typing, adaptive compaction advisory, ambient MCP status push,
IDE SSE stream, git-native team cost ledger, self-calibrating health scores, Zen Mode, temporal
replay, full mouse navigation, natural-language session filter, anomaly cards, Prometheus label
dimensions, and a wide range of TUI and accuracy improvements.

### S-8 — Zero-Dependency OpenTelemetry Export: Phase 1 (TRIZ PC-1 resolution)

**TRIZ analysis**: Users want AI cost/token metrics in Grafana, Datadog, Honeycomb, and any
OTel-compatible backend. The naive path — embedding the `opentelemetry-sdk` crate — introduces
6–15 MB of transitive dependencies (tonic, prost, h2) that violate Scopeon's local-first,
zero-friction contract.

**Physical Contradiction (PC-1)**: Scopeon MUST include OTel protocol support (enterprise
integration) AND MUST NOT include OTel libraries (lightweight, local-first). Resolved via
separation by structure (OTel Collector as external mediator) and by condition (zero overhead
when no endpoint is configured).

**Ideal Final Result (IFR)**: The Prometheus `/metrics` endpoint already present in
`scopeon serve` (v0.5.0) IS the OTel interface — any OTel Collector with a Prometheus receiver
bridges it to any backend today. The function is delivered by the system already present.

**Vepol resolution** (Standard 1.2.1 — Mediator):
`S1 (Scopeon) →[Prometheus]→ S3 (OTel Collector) →[OTLP]→ S2 (Backend)` — Collector is the
mediating substance; Scopeon never speaks gRPC/protobuf.

**40 Principles applied**: #1 Segmentation, #2 Taking Out, #6 Universality,
#22 Turn Harm into Benefit, #24 Mediator, #28 Replace Mechanism.

- New `docs/opentelemetry.md` — complete OTel integration guide covering three paths: Prometheus
  bridge (zero config), OTLP/HTTP push (Phase 2, planned), and trace export (Phase 3, planned).
  Includes quick-start YAML for Grafana Cloud, Datadog, Honeycomb, and self-hosted Prometheus.
- `docs/features.md` updated with OpenTelemetry row in Team & Integration table.
- Binary size delta: 0 KB. New Cargo dependencies: zero.

### S-7 — Optimal Compaction Advisory (TRIZ PC-5 resolution)

- New `CompactionAdvisory` alert kind fires **before** the context crisis, in the 55–79% fill window.
- `compaction_advisory_score()` combines fill percentage, fill acceleration (rate-of-change of consecutive slopes), and inverse cache-write fraction to pinpoint the optimal compact moment.
- Score > 0.65 triggers a `notifications/scopeon/alert` push notification of type `compaction_advisory` with `should_compact: true`, `fill_pct`, and `advisory_score` — **no token cost** per JSON-RPC §4.
- 60-second debounce per the existing `AlertDebounce` infrastructure.
- Fill history (last 5 samples) tracked in a `VecDeque<f64>` inside the background task — zero allocations at steady state.

### S-3 — Zero-Token Ambient Status Push (TRIZ IFR)

- Every 30 seconds when context is below 80% fill, the MCP background task emits a `notifications/scopeon/status` push notification with method type `ambient_status`.
- Payload: `fill_pct`, `predicted_turns_remaining`, `daily_cost_usd`, `cache_hit_rate_pct`, `should_compact`, and a lightweight `health_score_proxy`.
- Ambient push is **periodic** (not debounced like alerts) — no cooldown gate, no agent poll needed.
- Agents subscribed to MCP notifications get a free continuous status update at ~30 s cadence.

### S-4 — IDE SSE Status Endpoint (TRIZ TC resolution)

- New `GET /sse/v1/status` route on the HTTP server (tier 1+, same auth as other tiered endpoints).
- Returns a persistent `text/event-stream` response: compact JSON events every ~2 s with `fill_pct`, `daily_cost_usd`, `cache_hit_rate_pct`, `predicted_turns_remaining`, `should_compact`.
- Powered by the existing broadcast channel — **zero additional DB queries** per connected client.
- Keep-alive pings prevent proxy timeouts. Lagged clients silently skip stale frames.
- Added to startup banner: `GET /sse/v1/status — IDE status stream (SSE, tier 1+)`.
- Dependency: `tokio-stream 0.1` added to binary crate for `UnboundedReceiverStream`.

### S-2 — Git-Native Team AI Cost Ledger (TRIZ resource mobilisation)

- New `scopeon team [--days N]` command aggregates `AI-Cost:` trailers from `git log` history.
- Reads git commit history locally — no cloud, no data sharing, works with the existing `git-hook install` integration.
- Groups commits by author email; outputs a Markdown table with columns: Author · Commits · AI Commits · Total Cost · Avg/Commit · Tokens.
- Prints summary line with total spend and percentage of AI-assisted commits.
- Includes a tip when no trailers are found (guides user to `scopeon init`).
- Graceful error when run outside a git repository or when git is not in `PATH`.

### S-5 — Self-Calibrating Adaptive Health Score (TRIZ PC-1 resolution)

- New `ProjectProfile` enum (`CacheHeavy`, `Exploration`, `ToolHeavy`, `Balanced`) inferred from session telemetry.
- `classify_project_profile()` uses cache intensity, output/input ratio (thinking proxy), and MCP call density to pick the right profile.
- `WeightSet::for_profile()` returns profile-tuned weights (e.g. `CacheHeavy` → cache 40 pts, `Exploration` → waste 35 pts) that still sum to 100.
- New `compute_health_score_adaptive()` returns `(f64, AdaptiveHealthBreakdown)` — backward-compatible with existing callers via `compute_health_score` which is unchanged.
- `AdaptiveHealthBreakdown` exposes `profile_label()` for display in the TUI Insights tab.
- All new types re-exported from `scopeon-metrics` crate root.

### Narrative Intelligence Header (Dashboard)

- `draw_kpi_strip()` now rotates through natural-language insight sentences instead of raw KPI chips.
- Messages are prioritised: cache bust anomalies → top waste suggestion → context pressure warning → daily cost summary with trend → EOD pace warning.
- Falls back to classic KPI chips when no actionable insights are available.
- `build_narrative_messages()` helper generates sentences referencing `app.suggestions`, `cache_bust_drop`, `trend_cost_pct`, and `daily_projected_eod`.

### Zen Mode — Ambient Single-Line UI

- Press `z`/`Z` to collapse the entire TUI to a single ambient status line centered vertically.
- Shows: health score · context % · daily cost · cache %.
- Auto-exits when context ≥ 80% or budget ≥ 90% of daily limit (urgency overrides zen).
- Auto-restores zen after 3 consecutive refresh cycles below the threshold (pressure cleared).
- `pulse_phase` field drives animated leading-edge character.
- Help overlay updated with `z` key binding.

### Anomaly Cards in Insights Tab

- `draw_suggestions()` now renders each suggestion as a bordered card widget (instead of a flat text list).
- Each card: severity-coloured border + title, body text wrapped to card width.
- Cards dynamically divide available height (up to 4 per screen).

### Natural Language Session Filter

- Sessions filter (`/`) now accepts structured predicates in addition to plain text search:
  - `cost>N` / `cost<N` — filter by session cost in USD
  - `cache>N` / `cache<N` — filter by cache hit rate %
  - `tag:X` — filter by tag label
  - `model:X` — filter by model substring
  - `today` — sessions with activity today
  - `anomaly` — sessions flagged as anomalies
- Backward-compatible: unknown predicates fall through to substring search.
- Filter bar shows parse errors in amber when a structured predicate has an invalid value (e.g. `cost>abc │ ✗ expected a number`).
- Filter hint line updated to show the available predicates.

### `scopeon digest` — Weekly Markdown Report

- New CLI command `scopeon digest [--days N]` (default 7 days).
- Outputs a full Markdown digest: Executive Summary table, Daily Breakdown, Cost by Model, Cost by Tag, Recent Sessions, and Optimization Recommendations.
- Pipe to file or clipboard: `scopeon digest > weekly-ai-report.md`
- Recommendations are generated from cache hit rate, cost/turn ratio, and MCP call density.

### Context Pressure Heartbeat

- Context window gauge now pulses between `█` and `▉` leading-edge characters.
- Pulse rate scales with context pressure (calm → breathing → urgent).
- `pulse_phase: f64` field on `App` tracks the animation phase, reset on each refresh tick.

### End-of-Day Spend Projection

- Budget tab now shows a second projection row: EOD extrapolation based on current hourly rate.
- Displays: `EOD: $X.XX of $Y.YY daily limit · at $Z.ZZZ/hr pace`
- Amber warning (`⚠ APPROACHING LIMIT`) when on pace to exceed 90% of daily limit.
- `daily_hourly_rate` and `daily_projected_eod` fields added to `BudgetState`, computed each refresh.

### Overhead Transparency

- Status bar now appends `[◈ X.XMB]` showing Scopeon's own RSS memory footprint.
- Implemented via `/proc/self/status` (Linux) and `task_info` Mach API (macOS).
- Reinforces the "zero overhead" trust story: users can see exactly how much memory Scopeon uses.

### Temporal Replay Mode (Session Detail)

- Press `→`/`l` in fullscreen session detail to enter temporal replay — scrubs through turns one at a time.
- Press `←`/`h` to step backwards; first `←` from turn 0 exits replay.
- Each replayed turn shows a snapshot panel: turn #, context %, input/cache/output tokens, turn cost, cumulative cost to that point.
- The highlighted turn row is visually reversed in the turn table.
- Turn table title updates to show `← → replay` hint when in replay mode.
- Help overlay updated with `→ / ←` scrub keybinding.

### Full Mouse Navigation (Sessions Tab)

- Left-clicking a session row in the Sessions list now selects that session.
- Double-clicking the same row (clicking a row that is already selected) opens the fullscreen detail view.
- Tab-bar clicks reset `replay_turn_idx` to prevent stale replay state when switching tabs.

### Health Score Breakdown in Insights Tab

- Health gauge now shows per-component score breakdown: Cache (max 30), Context (max 25), Cost efficiency (max 25), Waste (max 20).
- Color-coded: green ≥ 80%, amber ≥ 50%, red below 50% of maximum.

### Fixed: "By Task Type" Always Empty

- `get_cost_by_tag_days()` was querying the legacy `sessions.tag` single-tag column instead of the `session_tags` table written by `scopeon tag set`.
- Fixed to join `session_tags → turns`, matching the pattern of `get_cost_by_tag()`.
- Empty-state hint updated: shows `scopeon tag set --session <id> feat-auth` instead of opaque "No task data".

### Fixed: Session list truncation indicator

- Session list header now shows "N — showing 200 most recent" when the list is at its 200-session display cap, so users know older sessions exist.

### Fixed: Memory safety in `scopeon doctor`

- Replaced undefined-behaviour memory cast in `doctor.rs` with proper `#[repr(C)]` struct for macOS `MACH_TASK_BASIC_INFO`.

### Fixed: Cache savings calculation

- Cache savings in `scopeon digest` now use exact per-model pricing (including write overhead) instead of a fixed 0.9 coefficient.

### Fixed: Session replay direction

- Temporal replay now advances oldest → newest (pressing `→` moves forward in time, as expected).

### Fixed: Unknown model pricing warning

- When Scopeon encounters a model with no pricing data, it now shows a toast in the TUI and a section in `scopeon doctor` instead of silently using Sonnet pricing.

### Fixed: Daily cost Z-score partial-day false alerts

- Anomaly detection now uses only completed days (not today's partial data) when computing the distribution, preventing morning false positives.

### Fixed: Warmup period now proportional to session length

- The warmup exclusion period for anomaly detection now scales with session length instead of using a hard-coded threshold of 5 turns.

### Fixed: Config validation on load

- Invalid config values (zero budget, invalid thresholds, zero refresh interval) are now caught and corrected at startup with a warning log instead of silently misbehaving.

---

## [0.6.0] — 2026-04-15

This release focuses on **accuracy, personalization, and developer trust** — fixing inflated metrics, adding self-diagnostics, enabling cross-model cost comparison, and personalizing anomaly thresholds to each user's spending patterns.

### Added: Honor CLAUDE_CONFIG_DIR Environment Variable

- `ClaudeCodeProvider::new()` now checks `CLAUDE_CONFIG_DIR` env var first, then falls back to `~/.claude`, then `/nonexistent`.
- Enables multi-profile setups and corporate deployments with non-standard Claude paths.
- `description()` updated to mention the env var override.

### Fixed: Streaming Turn Count Inflation

- Claude Code emits 2–4 JSONL records per assistant turn with the same `message.id` and progressively higher `output_tokens` (streaming chunks).
- `parse_file_incremental()` now tracks `HashMap<String, usize>` (msg_id → index) per batch. Duplicate `msg_id` → update existing `Turn` in-place (last-write-wins) instead of creating new turns.
- Fixes 2.7× inflated session turn counts and incorrect context pressure estimates.

### Added: Shadow Pricing — Cross-Model Cost Comparison

- `shadow_cost()` in `crates/scopeon-core/src/cost.rs` computes what a session would cost on a different model.
- Session detail view now shows "If Haiku: $X.XX" and "If Sonnet: $X.XX" rows (hidden when already on that model).
- Zero extra API calls — uses the in-memory pricing table.

### Added: `scopeon doctor` Command

- New `scopeon doctor` subcommand with plain-text health diagnostics (exit 0 = healthy, exit 1 = issues).
- Sections: Runtime (RSS memory, binary path, DB size/path), Providers (availability check), Data (session/turn counts, total cost, cache savings), Config (CLAUDE_CONFIG_DIR, budget limits), RAM comparison vs Node.js, Health summary.
- macOS memory via `task_info` extern C (no extra crates), Linux via `/proc/self/status`.

### Added: Personalized Anomaly Thresholds

- `compute_suggestions()` now includes three-tier personalized anomaly detection:
  - Tier 1 (< 7 days data): existing hard-coded thresholds.
  - Tier 2 (7–90 days): P90 label with per-user daily cost stats.
  - Tier 3 (> 90 days): Z-score (|z| > 2.0) with "Unusually Expensive Day" / "Low-Cost Day" alerts.
- No more one-size-fits-all thresholds that are always wrong for extreme users.

### Added: Cache Health Vital Sign

- `db.get_cache_efficiency_trend()` computes per-turn cache efficiency for the live session.
- `app.budget.cache_bust_drop` is populated in `refresh()` when the last 3-turn efficiency average drops below 50% of the prior 7-turn average.
- Dashboard "Insights" section shows `⚡ Cache efficiency dropped X% — possible MCP tool reorder or --resume bug` at the top of suggestions when a cache bust is detected.

### Added: Bayesian Cold-Start Turn Countdown

- `db.get_median_tokens_per_turn(days: 90)` provides a 90-day historical median as a Bayesian prior.
- `predict_turns_remaining_bayesian()` blends prior (100% at turn 0) with session regression (100% at turn 10+).
- Shows "~N turns remaining" from turn 0 instead of "N/A" during cold start.

### Added: Cost Attribution by Task Type

- `db.get_cost_by_tag_days(days: 30)` returns cost/count breakdown by auto-detected task type.
- New "By Task Type" section in Budget tab (bar chart, right-aligned dollar amounts, session counts).
- `BudgetState.cost_by_tag` field populated in `refresh()`.

### Fixed: By Model section now shows all model usage

- `get_cost_by_model()` no longer filters out zero-cost entries; shows model usage even for turns without pricing data.

---

## [0.5.0] — 2026-04-14

This release focuses on **observability, resilience, and developer workflow integration** — adding HTTPS webhook support, sub-millisecond shell hook latency, Prometheus metrics, budget forecasting, health score history, and auto-tagging from tool call patterns.

### Added: HTTPS Webhooks via System curl

- `do_http_post()` in `crates/scopeon-mcp/src/server.rs` replaced with `tokio::process::Command::new("curl")`.
- Supports all URL schemes: HTTP, HTTPS, WebSockets redirect. Works with Slack, Discord, PagerDuty, GitHub webhooks out of the box.
- curl `--retry 2 --retry-delay 1 --max-time 10` args for reliability. HTTP response code captured via `-w %{http_code}`.

### Added: Sub-millisecond Shell Hook Latency

- TUI `refresh()` now atomically writes `~/.cache/scopeon/status` (write→tmp→rename) on every tick.
- Shell hooks (bash/zsh/fish) updated to `cat ~/.cache/scopeon/status` instead of forking a subprocess.
- Falls back to `scopeon shell-status` (DB query) when the file doesn't exist (first run / TUI not active).
- `cmd_shell_status` also opportunistically writes the file when it computes metrics (populates cache for next prompt).
- `status_file_path()` and `write_status_file()` added to `src/shell_hook.rs` as public API.

### Added: Prometheus `/metrics` Endpoint

- `GET /metrics` route added to `scopeon serve` HTTP server (no tier restriction — local scraping).
- Hand-rolled Prometheus text/plain v0.0.4 format (no SDK dependency).
- Exposes: `scopeon_context_fill_pct`, `scopeon_cost_usd_today`, `scopeon_cost_usd_week`, `scopeon_cache_hit_rate`, `scopeon_budget_daily_used_pct`, `scopeon_total_sessions`, `scopeon_total_turns`, `scopeon_total_cost_usd`, `scopeon_cache_savings_usd`.
- Startup eprintln now lists `/metrics` route.

### Added: Budget Exhaustion Forecast

- `predict_days_until_monthly_limit()` in `app.rs` — linear regression on last 7 daily costs.
- New field `predicted_days_until_monthly_limit: Option<f64>` on `BudgetState`.
- Budget tab projection strip shows "~N days until monthly limit" with urgency coloring (red < 7d, yellow < 14d).

### Added: Health Score Trend Storage

- **Migration M0005**: `ALTER TABLE daily_rollup ADD COLUMN health_score_avg REAL NOT NULL DEFAULT 0.0`.
- `Database::update_today_health_score(score: f64)` — blends new score into today's daily_rollup row (70/30 EWA).
- Called in `app.rs` refresh() immediately after `compute_health_score()`.
- `DailyRollup` struct gains `health_score_avg: f64` field.
- Enables health trend sparkline and future ML-based health forecasting.

### Added: DB Retention / Auto-Archive

- `[storage] retain_days: Option<u64>` added to `UserConfig` (default: `None` = keep all data).
- New `StorageConfig` struct exported from `scopeon-core`.
- `Database::delete_turns_older_than(days)` — deletes old turns + orphaned sessions, refreshes daily rollup, returns count.
- On startup: if `retain_days` is set, purge is run and count is logged at INFO level.

### Added: Tool-Call Pattern Auto-Tagging

- `infer_tag_from_tool_calls(calls: &[ToolCall]) -> Option<&'static str>` in `crates/scopeon-core/src/tags.rs`.
  - ≥ 3 web-search/browser calls → `"research"`.
  - ≥ 5 bash/grep/find calls → `"debugging"`.
  - ≥ 3 write + ≥ 3 read calls → `"refactoring"`.
- `watcher.rs::process_file()` applies tag inference after session upsert (only when no manual tag is set).
- Falls back to `branch_to_tag()` if no tool pattern matches.
- 4 new unit tests in `tags.rs`: research/debugging/refactoring/no-match.

### Changed

- `[workspace.package] version` bumped `0.4.0` → `0.5.0`.
- `StorageConfig` exported from `scopeon_core` public API.
- `infer_tag_from_tool_calls` exported from `scopeon_core` public API.

---

## [0.4.0] — 2026-04-13

This release applies a **TRIZ-inspired v2 analysis** — 10 inventive solutions framed around the contradictions and undesirable effects identified in v0.3.0. The references below document the design intent behind each change rather than a formal standalone TRIZ artifact.

### Added: Cross-Session Intelligence in Suggestions

- `compute_suggestions()` now uses `GlobalStats` to detect below-average cache hit rate compared to the user's own historical mean from `daily_rollup`.
- Three new cross-session suggestion rules: below-average cache efficiency, above-average input token consumption, high cost-per-turn relative to historical mean.
- All cross-session rules are guarded by `global.total_turns >= 10` (minimum history for statistical reliability).

### Added: Auto-Tag from Git Branch Prefix

- `crates/scopeon-core/src/tags.rs` — new `branch_to_tag(branch: &str) -> Option<&'static str>` function.
  - Maps `feat/` → `"feature"`, `fix/` → `"bugfix"`, `hotfix/` → `"bugfix"`, `refactor/` → `"refactor"`, `chore/` → `"chore"`, `docs/` → `"docs"`, `test/` → `"test"`.
  - Returns `None` for `main`, `master`, `develop`, `trunk` (no badge shown).
- Sessions tab shows auto-detected branch tags as `[feature]`, `[bugfix]`, etc. suffixes.

### Added: Predictive Context Countdown

- `predict_turns_remaining_from_turns()` in `app.rs` — least-squares linear regression on last ≤10 turns' token consumption. Returns `None` if slope ≤ 0 or fewer than 3 data points.
- `BudgetState.predicted_turns_remaining: Option<i64>` field exposed in TUI.
- Status bar now shows `Ctx ████ 73% ~12t` — predicted turns remaining inline with the fill bar.
- Alert at 80%+ context includes `(~12 turns left)` in the banner text.
- `get_context_pressure` MCP tool response includes `predicted_turns_remaining`.
- `predict_turns_remaining()` in `server.rs` provides the same calculation for MCP responses.

### Added: Proactive MCP Push Notifications

- `AlertKind` enum: `ContextCrisis`, `ContextWarning`, `BudgetWarning`, `LowTurnsLeft`, `CompactionDetected`.
- `AlertDebounce` struct — per-kind cooldown (60 seconds default) prevents notification spam.
- `check_alerts(snap, debounce, tx)` — inspects each fresh snapshot for alert conditions and enqueues `Alert` values via `tokio::sync::mpsc::channel`.
- `run_mcp_server()` main loop now uses `tokio::select!` on incoming JSON-RPC requests and outgoing alert channel. Alert payloads are written as JSON-RPC notifications (no `id` field — per §4 of JSON-RPC 2.0 spec, fire-and-forget).
- Alert triggers: context ≥ 95% (crisis), context 80–94% (warning), daily spend > 90% of limit (budget warning), predicted turns ≤ 5 (low turns), compaction event detected.

### Added: Adaptive Percentile Threshold Engine

- `crates/scopeon-metrics/src/thresholds.rs` — new `UserThresholds` struct with `Default` impl (conservative hard-coded values) and `from_daily_data(&[(f64,f64,f64)])` factory.
- `UserThresholds::from_daily_data` requires ≥7 days of history; otherwise returns defaults.
- Computes P10 of daily cache hit rate → `cold_cache_pct`; P90 of thinking/output ratios → `thinking_ratio_warn` (critical = 2× warn); fixed `context_bloat_multiplier = 2.0`.
- `db.get_threshold_data()` queries last 90 days of `daily_rollup` for threshold computation.
- `WasteReport::compute_with_thresholds(ctx, thresholds)` — waste signals now use adaptive thresholds. `compute()` delegates to `compute_with_thresholds` with defaults.
- `App.user_thresholds: UserThresholds` refreshed each cycle in `app.refresh()`.
- MCP `handle_get_optimization_suggestions` also uses adaptive thresholds.

### Added: Webhook Escalation Chain

- `WebhookConfig { url: String, events: Vec<String> }` and `webhooks: Vec<WebhookConfig>` field added to `AlertsConfig` in `user_config.rs`.
- `fire_webhooks(config, alert_type, payload)` — dispatches async tokio tasks for each configured webhook. Failures are logged as warnings and never propagate to the caller.
- `do_http_post(url, body)` — minimal HTTP/1.1 POST over raw `tokio::net::TcpStream`. No additional crate dependencies. Supports `http://` URLs.
- Webhooks fire alongside every MCP push notification; per-event type filtering; empty events list = all events.
- `UserConfig` loaded at `run_mcp_server()` startup; checked in the alert dispatch branch.

### Added: Session Tagging CLI + DB Migration

- `M0004` migration: `session_tags(session_id TEXT, tag TEXT, PRIMARY KEY(session_id,tag))` table + index on `tag`.
- `db.set_session_tags(session_id, tags)` — atomically replaces all tags (DELETE + INSERT in transaction).
- `db.get_session_tags(session_id) -> Vec<String>`.
- `db.get_sessions_by_tag(tag) -> Vec<Session>`.
- `db.get_cost_by_tag(tag) -> (total_cost, session_count, turn_count)`.
- `scopeon tag set <session-id> [tags...]` — set (or clear) tags on a session.
- `scopeon tag show <session-id>` — list tags on a session.
- `scopeon tag list <tag>` — find all sessions with a given tag.
- MCP tools added: `set_session_tags`, `get_cost_by_tag`.

### Added: WebSocket Browser Dashboard

- `axum` `ws` feature added to workspace dependencies.
- `GET /` — serves the embedded single-file `src/dashboard.html` (no npm, no CDN, no build step).
- `GET /ws/v1/metrics` — real-time WebSocket endpoint. `tokio::sync::broadcast` channel from background snapshot task to all connected clients.
- Background task publishes a `build_ws_snapshot` payload every 2 seconds.
- Dashboard features: context pressure bar, daily cost vs. budget gauge, cache hit rate gauge, token usage breakdown, 30-day cost history bars, live stats (turns, total cost, savings), WebSocket auto-reconnect, push alert banners.
- `ServeState` gains `ws_tx: broadcast::Sender<String>` field.
- Dashboard URL printed at startup: `http://localhost:7771`.

### Added: Read-Connection Pool (WAL complement)

- `Database.db_path: Option<PathBuf>` — stores filesystem path for on-disk databases.
- `Database::open_readonly(path)` — opens a read-only connection with `PRAGMA query_only=ON`. WAL mode allows concurrent reads without blocking the writer.
- `Database::path() -> Option<&Path>` accessor.
- MCP snapshot task now opens a dedicated read-only `Database` connection. On each refresh it opens a fresh read connection — zero mutex contention with the watcher's write connection.
- Falls back to shared mutex for in-memory databases (tests).

### Changed

- `scopeon serve` startup banner updated to show dashboard URL and WebSocket endpoint.
- `run_mcp_server()` loads `UserConfig` once at startup (used for webhook dispatch).
- `AlertsConfig` in `user_config.rs` gains `webhooks: Vec<WebhookConfig>` field with `#[serde(default)]`.

### Fixed — Round 6 peer review

- `Alert.kind` field removed (redundant — type is embedded in payload `params.type`).
- `fill_pct >= 80.0 && fill_pct < 95.0` → `(80.0..95.0).contains(&fill_pct)` (clippy `manual_range_contains`).
- `Cargo.toml` `[workspace.package]` and `[profile.release]` section headers preserved correctly after dependency additions.

---

## [0.3.0] — 2026-04-13

### Added — TRIZ Phase 1: Pre-Computation Engine

- `MetricSnapshot` struct in `scopeon-mcp` caches all 10 frequently-queried MCP metrics
  in a `RwLock`-protected in-memory snapshot.
- Background Tokio task refreshes the snapshot at an adaptive rate:
  5 s (IDLE, context < 50 %) → 1 s (ACTIVE, 50–80 %) → 200 ms (CRISIS, > 80 %).
- Every cacheable tool call now reads from the snapshot with **zero database queries**.
  Only `compare_sessions` (which requires specific session IDs) always queries live.

### Added — TRIZ Phase 2: Adaptive TUI State Machine

- TUI `refresh_interval` now adjusts dynamically based on context pressure at the end
  of every `app.refresh()` call:
  - < 50 % fill → 2 s interval (idle, save CPU)
  - 50 – 80 % fill → 500 ms (active, timely updates)
  - > 80 % fill → 100 ms (crisis, near-real-time)

### Added — TRIZ Phase 3: CI Cost Gate (`scopeon ci`)

- New `scopeon ci snapshot` subcommand — captures a point-in-time JSON snapshot of
  AI usage metrics (cost, cache hit rate, context peak, avg tokens/turn, session/turn counts).
- New `scopeon ci report` subcommand — compares current metrics against a baseline
  snapshot and prints a Markdown table with colour-coded emoji deltas (`🟢 🔴 ⬜`).
- `--fail-on-cost-delta <PCT>` flag — exits with code 1 if AI cost increased by more
  than the threshold; designed for use as a hard CI gate.
- `.github/examples/ai-cost-gate.yml` — complete GitHub Actions workflow example
  for PR-level AI cost reporting.

### Added — TRIZ Phase 4: Team HTTP API (`scopeon serve`)

- New `scopeon serve` subcommand — starts a read-only axum HTTP server on `0.0.0.0:7771`.
- Five endpoints: `GET /health`, `GET /api/v1/stats`, `GET /api/v1/budget`,
  `GET /api/v1/sessions`, `GET /api/v1/context`.
- Four-tier privacy system: health-only → aggregate stats → per-session metadata → full metrics.
- CORS headers via `tower-http` for browser dashboard consumption.
- All data is read-only; the server never writes to the database.

### Added — TRIZ Phase 5: Binary Distribution

- `.github/workflows/release.yml` — 5-platform cross-compilation matrix (mac-arm64, mac-x86, linux-x86, linux-arm64, windows).
- `install.sh` — `curl | sh` installer that auto-detects OS and CPU architecture.
- `[package.metadata.binstall]` — enables `cargo binstall scopeon` without compilation.
- `SHA256SUMS.txt` generated alongside release binaries.

### Fixed — Round 5 peer review

- `lock_db()` helper returns `Result<MutexGuard>` instead of silently recovering from mutex poisoning.
- `watcher.rs` spawn_blocking closure: mutex poison → log error + return.
- `server.rs` dispatch: mutex poison → return JSON-RPC error `-32603`.
- Added `// SAFETY:` comment on `unchecked_transaction` in `db.rs`.
- `COMPACTION_MIN_PREV_TOKENS` and `COMPACTION_DROP_THRESHOLD` promoted to named constants.

---

## [0.2.0] — 2026-04-12

### Added

- GitHub Copilot CLI provider parser.
- Gemini CLI provider with Gemini 1.5 / 2.0 / 2.5 pricing.
- Generic OpenAI-compatible provider (configurable log paths).
- `scopeon export` command — JSON and CSV output.
- `scopeon reprice` command — recalculate historical costs after price changes.
- Multi-agent tree view (Agents tab, tab 6).
- Health score (0–100) composite metric per session.
- Waste analysis engine with severity-weighted signals.
- Optimisation suggestions derived from waste signals.

### Fixed

- Compaction detection now requires `COMPACTION_MIN_PREV_TOKENS = 50,000` to avoid false positives on short follow-up messages.
- MCP tool response latency reduced from ~10 ms to < 1 ms via inline caching.

---

## [0.1.0] — 2026-04-10

### Added

- Initial release with Claude Code provider support.
- SQLite storage with WAL mode and 3 migrations.
- Ratatui 6-tab TUI dashboard: Dashboard, Sessions, Insights, Budget, Providers, Agents.
- MCP server over stdio with 10 tools: `get_token_usage`, `get_session_summary`, `get_cache_efficiency`, `get_history`, `compare_sessions`, `get_context_pressure`, `get_budget_status`, `get_optimization_suggestions`, `suggest_compact`, `get_project_stats`.
- `scopeon init` — auto-configure Claude Code MCP integration.
- Prompt cache intelligence: hit rate, tokens saved, USD saved.
- Context window pressure bar with per-model window sizes.
- Budget guardrails: daily/weekly/monthly limits with alert banners.
- Byte-offset tracking for incremental log parsing (no double-counting on restart).
- Aider and Cursor provider parsers.
- Ollama provider (local API polling, free — no cost tracking).

---

[Unreleased]: https://github.com/sorunokoe/Scopeon/compare/v0.7.1...HEAD
[0.7.1]: https://github.com/sorunokoe/Scopeon/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/sorunokoe/Scopeon/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/sorunokoe/Scopeon/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/sorunokoe/Scopeon/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/sorunokoe/Scopeon/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/sorunokoe/Scopeon/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/sorunokoe/Scopeon/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/sorunokoe/Scopeon/releases/tag/v0.1.0
