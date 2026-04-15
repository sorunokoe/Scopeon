# Scopeon Architecture

This document is for contributors who want to understand how the codebase works before
making changes. It covers the crate layout, data flow, and patterns you'll encounter
throughout the code.

---

## Crates at a glance

```
scopeon (binary)
    scopeon-core        - data models, SQLite, cost engine, config
    scopeon-collector   - JSONL parsers, file watcher, provider abstraction
    scopeon-metrics     - pure-function metric computation (no I/O)
    scopeon-tui         - Ratatui 6-tab terminal dashboard
    scopeon-mcp         - MCP JSON-RPC 2.0 server (stdio)
```

The dependency graph is a DAG — no crate depends on the binary, and `scopeon-metrics`
has no I/O so it can be tested with pure unit tests.

```
scopeon (bin)
    +-- scopeon-tui       --> scopeon-core
    +-- scopeon-mcp       --> scopeon-core
    +-- scopeon-collector --> scopeon-core
    +-- scopeon-metrics   --> scopeon-core
    +-- (direct: serve.rs, ci.rs, digest.rs, ...)
```

---

## Data flow

```
  AI agent log files (JSONL / plain text)
          |
          v
  scopeon-collector
  +-------------------------------------------+
  |  FileWatcher (notify)                      |
  |    watches provider dirs for new bytes     |
  |         |                                  |
  |         v                                  |
  |  Provider::parse_incremental()             |
  |    reads new bytes, emits Session + Turns  |
  |         |                                  |
  |         v                                  |
  |  Database::upsert_session()                |
  |  Database::upsert_turns()                  |
  +-------------------------------------------+
          |
          v  SQLite (~/.scopeon/scopeon.db)
          |
          +---> scopeon-tui   (reads DB every refresh tick)
          +---> scopeon-mcp   (reads DB per tool call)
          +---> scopeon serve (reads DB per HTTP/WS request)
```

The SQLite file is the **single source of truth**. Nothing persists in memory beyond
the current refresh cycle. The file watcher writes; everything else reads.

---

## SQLite schema

Four tables, seven migrations (additive only — ALTER TABLE ADD COLUMN or CREATE TABLE,
never destructive). Migrations run at startup via `rusqlite_migration`.

```
sessions
  id                    TEXT PK   - opaque session ID from the agent
  provider              TEXT      - "claude-code", "copilot", "aider", ...
  model                 TEXT      - model string as reported by the agent
  git_branch            TEXT      - git branch at session start (if detectable)
  started_at            INTEGER   - Unix timestamp
  last_active_at        INTEGER
  total_input_tokens    INTEGER
  total_cache_read      INTEGER
  total_cache_write     INTEGER
  total_output_tokens   INTEGER
  total_cost_usd        REAL
  context_window_tokens INTEGER   - per-session window size from JSONL maxTokens

turns
  id                    INTEGER PK autoincrement
  session_id            TEXT FK -> sessions.id
  turn_index            INTEGER
  input_tokens          INTEGER
  cache_read_tokens     INTEGER
  cache_write_tokens    INTEGER
  thinking_tokens       INTEGER
  output_tokens         INTEGER
  mcp_call_count        INTEGER
  cost_usd              REAL
  timestamp             INTEGER

session_tags
  session_id            TEXT FK
  tag                   TEXT

daily_rollup
  date                  TEXT PK   - "YYYY-MM-DD"
  total_cost_usd        REAL
  total_tokens          INTEGER
  session_count         INTEGER
  cache_savings_usd     REAL
  health_score_avg      REAL
```

---

## `scopeon-core`

**`db.rs`** is the heart. All SQL lives here.

- `Database` wraps a `Mutex<Connection>` (write) plus up to 4 read-only connections.
  Read-heavy paths (HTTP API, WS snapshot) use the read pool.
- `with_db()` in `serve.rs` wraps DB operations in `tokio::task::spawn_blocking` so
  SQLite never blocks the Tokio executor.
- Migrations are a `&[&str]` constant at the top of the file. Add new ones at the end.
  Never modify existing entries — they have already run on users' databases.

**`cost.rs`** — per-model pricing table (sorted prefix array). `pricing_for()` does
prefix-matched lookup. Unknown models log a deduplicated toast warning. Cost estimates
are always prefixed with `~$` to signal they are approximations.

**`context.rs`** — maps model name prefixes to context window sizes.
`context_pressure_with_window(model, input_tokens, stored_window)` prefers the
per-session `context_window_tokens` stored in the DB over the prefix table, so
non-standard deployments work correctly.

**`models.rs`** — `Session`, `Turn`, `ToolCall`, `GlobalStats`, `DailyRollup`. Plain
structs with serde derives, no business logic.

---

## `scopeon-collector`

### Provider abstraction

Every AI agent source implements the `Provider` trait:

```rust
pub trait Provider: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn watch_paths(&self) -> Vec<PathBuf>;
    fn parse_file_incremental(
        &self,
        path: &Path,
        from_byte: u64,
    ) -> Result<Vec<IncrementalResult>>;
}
```

`parse_file_incremental` receives the byte offset of the last read — it only parses
bytes written since the last call. This is how near-zero CPU overhead is achieved on
idle: only new bytes are processed.

Providers live in `crates/scopeon-collector/src/providers/`. To add a new provider:
1. Create `providers/myprovider.rs` implementing `Provider`.
2. Register it in `providers/mod.rs`.
3. Follow the complete guide in `CONTRIBUTING.md`.

### File watcher

`watcher.rs` uses the `notify` crate (FSEvents on macOS, inotify on Linux). On each
file event:
1. Read new bytes from the stored byte offset.
2. Call `provider.parse_file_incremental()`.
3. Upsert the resulting `Session` + `Turn`s into SQLite.
4. Run tool-call pattern inference and auto-tag if no manual tag exists.
5. Advance the stored byte offset.

---

## `scopeon-metrics`

All functions here are **pure** — no I/O, no global state. They take DB query result
structs and return metric structs. This makes them fast to unit-test in isolation.

| Module | Responsibility |
|--------|---------------|
| `health.rs` | Health score 0-100 (cache 30 + context 25 + cost 25 + waste 20) |
| `waste.rs` | Severity-weighted waste signals (Critical / Warning / Info) |
| `suggestions.rs` | Actionable text suggestions from cross-session intelligence |
| `thresholds.rs` | Adaptive P10/P90 percentile thresholds from 90-day history |
| `metric.rs` | `MetricSnapshot` - the aggregated struct passed to TUI and MCP |
| `builtin/` | Per-axis modules (cache, cost, velocity, quality, pattern) |

The exact health score formula is documented in the `health.rs` module-level doc
comment with full math notation.

---

## `scopeon-tui`

A **6-tab Ratatui dashboard** driven by an adaptive 500ms-4s refresh loop.

```
App (app.rs)
  refresh()       - reads DB, computes metrics, updates App state
  handle_event()  - keyboard and mouse input -> state transitions

ui.rs - top-level render() - draws status bar, tab bar, active tab

views/
  dashboard.rs   - Tab 1: KPI strip, turn table, cache + context gauges
  sessions.rs    - Tab 2: session list with natural-language filter
  insights.rs    - Tab 3: health breakdown, anomaly cards, suggestions
  budget.rs      - Tab 4: daily/weekly/monthly cost bars, forecasts
  providers.rs   - Tab 5: per-provider session counts and costs
  agents.rs      - Tab 6: live agent monitoring (per-session focus)
```

**Refresh rate adaptation:** context pressure > 80% doubles the refresh rate to ~500ms;
idle periods back off to 4s. `pulse_phase` drives the animated context gauge.

**Zen mode:** pressing `z` collapses the TUI to a single ambient status line. It
auto-exits when context or budget pressure becomes urgent.

**Mouse support:** click-to-select and double-click-to-open in the sessions tab. Mouse
events map pixel coordinates to list rows using Ratatui `Rect` layout values.

---

## `scopeon-mcp`

**JSON-RPC 2.0 over stdin/stdout.** Stdout is protected by
`Arc<tokio::sync::Mutex<BufWriter<Stdout>>>` (type alias `SharedWriter`) to prevent
interleaved output from concurrent tasks.

14 tools exposed to the agent:

| Tool | Returns |
|------|---------|
| `get_context_pressure` | fill %, tokens used/remaining, urgency level |
| `get_token_usage` | per-turn breakdown for the live session |
| `get_cost_summary` | today/week/month costs, budget status |
| `get_cache_efficiency` | hit rate, tokens saved, USD saved |
| `get_health_score` | 0-100 score with per-component breakdown |
| `list_sessions` | recent sessions with cost + cache summary |
| `get_session_detail` | full turn table for one session |
| `get_waste_signals` | actionable waste analysis |
| `suggest_compact` | recommendation to compact context |
| `compare_sessions` | before/after diff for optimization experiments |
| `get_optimization_suggestions` | cross-session suggestions |
| `get_daily_breakdown` | per-day cost trend |
| `get_prometheus_metrics` | raw Prometheus text format |
| `get_history` | multi-day token + cost history |

**Push notifications** (no polling needed): context > 80%, budget > 90%, 5 or fewer
turns remain, or a compaction event is detected.

---

## `scopeon` binary (`src/`)

| File | Subcommand(s) |
|------|--------------|
| `main.rs` | Dispatch, `tag` CLI |
| `serve.rs` | `scopeon serve` - HTTP API + WebSocket dashboard |
| `ci.rs` | `scopeon ci snapshot/report` |
| `digest.rs` | `scopeon digest` - Markdown weekly report |
| `shell_hook.rs` | `scopeon shell-hook / shell-status` |
| `git_hook.rs` | `scopeon git-hook` |
| `badge.rs` | `scopeon badge` |
| `onboarding.rs` | `scopeon init` |
| `doctor.rs` | `scopeon doctor` |
| `dashboard.html` | Embedded browser dashboard (served by `serve.rs`) |

`serve.rs` is the most complex file. It runs an Axum HTTP server with REST endpoints and
a WebSocket stream. `with_db()` wraps all DB access in `spawn_blocking`.

---

## Conventions

### Error handling

- Library crates use `Result<T, anyhow::Error>` or `thiserror`-derived enums.
- No `unwrap()` in library code. Use `?` or handle the error explicitly.
- The binary may use `anyhow::bail!` / `.context()` at the outermost call site.

### Logging

Use structured `tracing` macros:
```rust
tracing::warn!(model = %model, "Unknown model - falling back to Sonnet pricing");
```
`RUST_LOG=scopeon=debug` enables verbose output for contributors debugging locally.

### Tests

All tests are **offline** - `tempfile::tempdir()` or in-memory SQLite. No real agent
logs, no network, no filesystem side-effects beyond the temp directory.

Tests live inline (`#[cfg(test)] mod tests { ... }`). The `db.rs` file has `make_session()`
and `make_turn()` helpers - use those rather than constructing structs by hand.

### Adding a database migration

1. Open `crates/scopeon-core/src/db.rs` and find `MIGRATIONS: &[&str]`.
2. Append your SQL as a new entry. Never modify existing entries.
3. Update `row_to_session` / `row_to_turn` if you added columns (positional indexing).
4. Update `upsert_session` / `upsert_turn` to include the new column.
5. Add the field to the relevant struct in `models.rs`.

### Adding a pricing entry

Edit `PRICING` in `crates/scopeon-core/src/cost.rs`. Sorted longest-prefix-first.
Prices are in USD per million tokens.

### Adding a context window size

Edit `CONTEXT_WINDOWS` in `crates/scopeon-core/src/context.rs`. Same sort order.

---

## Quick-lookup

| Question | File |
|----------|------|
| How is the health score computed? | `crates/scopeon-metrics/src/health.rs` (module doc) |
| How is context pressure computed? | `crates/scopeon-core/src/context.rs` |
| How is cost estimated? | `crates/scopeon-core/src/cost.rs` |
| Where are model prices? | `crates/scopeon-core/src/cost.rs` - `PRICING` |
| Where are context window sizes? | `crates/scopeon-core/src/context.rs` - `CONTEXT_WINDOWS` |
| Where are DB migrations? | `crates/scopeon-core/src/db.rs` - `MIGRATIONS` |
| How does Claude Code provider parse JSONL? | `crates/scopeon-collector/src/providers/claude.rs` |
| How are MCP call counts tracked? | `crates/scopeon-collector/src/parser.rs` (only `mcp__` prefix) |
| How does the TUI refresh loop work? | `crates/scopeon-tui/src/app.rs` - `refresh()` |
| How does the HTTP API work? | `src/serve.rs` |
| How does the MCP server work? | `crates/scopeon-mcp/src/server.rs` |
| Where are CI/CD workflows? | `.github/workflows/` |
