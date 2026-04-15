# Contributing to Scopeon

First off — **thank you**. Every contribution makes Scopeon more useful for the entire AI-developer community.

Scopeon is dual-licensed under **MIT OR Apache-2.0**. By contributing you agree your work is available under the same terms — no CLA, no paperwork.

---

## Table of contents

- [Code of conduct](#code-of-conduct)
- [Getting started](#getting-started)
- [Project structure](#project-structure)
- [Development workflow](#development-workflow)
- [Adding a new provider](#adding-a-new-provider)
- [Adding an MCP tool](#adding-an-mcp-tool)
- [Adding a new alert type](#adding-a-new-alert-type)
- [Code style & conventions](#code-style--conventions)
- [Testing](#testing)
- [Submitting a pull request](#submitting-a-pull-request)
- [Reporting bugs](#reporting-bugs)
- [Proposing features](#proposing-features)

---

## Code of conduct

Be kind. Assume good faith. Disagreements on technical direction are fine — personal attacks are not. We reserve the right to remove comments or contributions that violate this principle.

---

## Getting started

### Prerequisites

| Tool | Version | Install |
|---|---|---|
| Rust | ≥ 1.86 | [rustup.rs](https://rustup.rs) |
| Git | any | [git-scm.com](https://git-scm.com) |

macOS 12+ or Linux (glibc 2.31+). Windows builds via cross-compilation in CI — local dev on Windows is not yet validated.

### Clone and build

```bash
git clone https://github.com/scopeon/scopeon
cd scopeon
cargo build                    # dev build (fast)
cargo build --release          # optimised build (used for releases)
```

A `Makefile` wraps common tasks so you do not have to remember flags:

```bash
make          # fmt-check + clippy + test (same as CI)
make build    # dev build
make test     # run all tests
make clippy   # clippy -D warnings
make fmt      # auto-format in place
make install  # install binary to ~/.cargo/bin
make docs     # build and open rustdoc
make check    # full pre-push check (fmt + clippy + test + audit)
```

See ARCHITECTURE.md for a full codebase map (data flow, schema, key files).

### Run from source

```bash
cargo run -- status            # quick inline stats, no TUI
cargo run -- start             # full TUI (reads provider log files)
cargo run -- serve             # browser dashboard on http://localhost:7771
cargo run -- export --format csv --days 30
```

### Run tests

```bash
cargo test --workspace
```

All tests are **offline** — they use in-memory or temp-file SQLite databases. Claude Code, Copilot, or any other AI agent does not need to be installed. 139 tests, all pass, all offline.

### Lint and format

```bash
cargo clippy --workspace -- -D warnings    # must be zero warnings
cargo fmt --all                            # auto-format
cargo fmt --all -- --check                 # CI check (no modification)
```

---

## Project structure

```
scopeon/                       ← CLI binary
  src/
    main.rs                    ← entry point, subcommand dispatch, tag CLI
    ci.rs                      ← `scopeon ci` snapshot + report
    serve.rs                   ← `scopeon serve` HTTP API + WebSocket dashboard
    onboarding.rs              ← `scopeon init` MCP config wizard
    shell_hook.rs              ← `scopeon shell-hook` prompt integration
    dashboard.html             ← embedded single-file browser dashboard
crates/
  scopeon-core/                ← models, SQLite schema (4 migrations, WAL), cost engine
    src/
      db.rs                    ← database: all queries, migrations, WAL + read-pool
      models.rs                ← Session, Turn, ToolCall, GlobalStats, DailyRollup
      cost.rs                  ← per-model pricing, cache savings calculation
      context.rs               ← context window sizes per model, fill % calculation
      tags.rs                  ← branch_to_tag() — auto-tag from git branch prefix
      user_config.rs           ← UserConfig, BudgetConfig, AlertsConfig, WebhookConfig
  scopeon-collector/           ← JSONL parsers + file watcher
    src/
      providers/               ← one file per AI provider (claude, copilot, aider, …)
      watcher.rs               ← FSEvents/inotify file watcher + byte-offset tracking
  scopeon-mcp/                 ← MCP JSON-RPC 2.0 server
    src/
      server.rs                ← all 14 tools, MetricSnapshot, push notifications, webhooks
  scopeon-tui/                 ← Ratatui 6-tab dashboard
    src/
      app.rs                   ← App state, refresh loop, adaptive thresholds, predictions
      ui.rs                    ← rendering entry point, status bar
      theme.rs                 ← colour themes
      views/                   ← one file per tab (dashboard, sessions, insights, …)
  scopeon-metrics/             ← metric computation (no I/O, pure functions)
    src/
      health.rs                ← health score (0–100)
      waste.rs                 ← waste signals, compute_with_thresholds()
      suggestions.rs           ← actionable suggestions, cross-session intelligence
      thresholds.rs            ← UserThresholds, P10/P90 adaptive percentile engine
.github/
  workflows/ci.yml             ← CI pipeline (format, clippy, tests, MSRV, audit)
  workflows/release.yml        ← cross-platform binary release (v* tags)
  examples/ai-cost-gate.yml   ← example PR cost gate workflow
```

### Where to start for common contributions

| Goal | Where to look |
|---|---|
| New AI provider | `crates/scopeon-collector/src/providers/` |
| New MCP tool | `crates/scopeon-mcp/src/server.rs` |
| New TUI tab or panel | `crates/scopeon-tui/src/views/` |
| New cost model / pricing | `crates/scopeon-core/src/cost.rs` |
| New metric or waste signal | `crates/scopeon-metrics/src/waste.rs` |
| New suggestion rule | `crates/scopeon-metrics/src/suggestions.rs` |
| New alert type | `crates/scopeon-mcp/src/server.rs` (`AlertKind` + `check_alerts`) |
| New CLI subcommand | `src/main.rs` |
| Database schema change | `crates/scopeon-core/src/db.rs` (add a migration at the end of `MIGRATIONS`) |
| Browser dashboard changes | `src/dashboard.html` |

---

## Development workflow

### Running against your real data

```bash
# Uses your real ~/.claude/ logs and ~/.scopeon/scopeon.db
cargo run -- start

# Just status (fast, no TUI)
cargo run -- status

# Debug logging
RUST_LOG=scopeon=debug cargo run -- start
```

### Iterating on TUI layout

The TUI reads from SQLite — you do not need live log files. Just run `cargo run -- start` and it will use whatever data is already in `~/.scopeon/scopeon.db`.

### Iterating on the MCP server

```bash
# Start the MCP server over stdio
cargo run -- mcp

# Test individual JSON-RPC calls manually
echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_context_pressure","arguments":{}}}' \
  | cargo run -- mcp
```

### Iterating on the HTTP API and WebSocket dashboard

```bash
cargo run -- serve --port 7771 --tier 3
# In another terminal:
curl http://localhost:7771/health
curl http://localhost:7771/api/v1/stats
# Open http://localhost:7771 in a browser for the live WebSocket dashboard
```

### Adding a webhook receiver for local testing

```bash
# Simple HTTP echo server (requires Python)
python3 -m http.server 9999

# Configure in ~/.scopeon/config.toml:
# [[alerts.webhooks]]
# url = "http://localhost:9999/webhook"
# events = []
```

---

## Adding a new provider

Providers live in `crates/scopeon-collector/src/providers/`. Each provider implements the `Provider` trait:

```rust
pub trait Provider: Send + Sync {
    /// Human-readable name shown in the TUI Providers tab.
    fn name(&self) -> &str;

    /// Return true if this provider's log files are present on disk.
    fn is_available(&self) -> bool;

    /// Directories to watch for file-change events (FSEvents / inotify).
    fn watch_paths(&self) -> Vec<PathBuf>;

    /// Walk all log files and persist new turns to the database.
    /// Must be idempotent — byte offsets prevent double-counting.
    fn scan(&self, db: &Database) -> Result<()>;
}
```

**Step-by-step:**

1. **Create `crates/scopeon-collector/src/providers/my_tool.rs`** implementing the trait above.
2. **Map the log format** to the shared models in `scopeon-core`:
   - `Session` — one per conversation
   - `Turn` — one per request/response pair (contains all token counts)
   - `ToolCall` — one per MCP/function tool use within a turn
3. **Register the provider** in `src/main.rs` inside `build_providers()`.
4. **Update the providers table** in `README.md`.
5. **Write at least one test** that parses a sample log line and asserts the correct `Turn` fields.

---

## Adding an MCP tool

MCP tools are registered in `crates/scopeon-mcp/src/server.rs`.

1. **Add the tool to `list_tools()`** — name, description, and JSON schema for parameters.
2. **Add a match arm in `dispatch()`** — call your DB query or MetricSnapshot field.
3. **If the result is cacheable**, add the field to `MetricSnapshot` and populate it in `refresh_snapshot()`.
4. **Write a doc comment** on the match arm explaining what the tool returns and when it is useful.

---

## Adding a new alert type

Proactive push notifications are defined in `crates/scopeon-mcp/src/server.rs`.

1. **Add a variant to `AlertKind`** — derive `Debug, Clone, PartialEq, Eq, Hash`.
2. **Add the detection condition in `check_alerts()`** — read the relevant field from `MetricSnapshot` and call `debounce.should_fire(&AlertKind::MyAlert)`.
3. **Enqueue the alert** via `tx.try_send(Alert { payload: json!({...}) })` with a `"method": "notifications/scopeon/alert"` payload.
4. **Document the event type string** in the webhook configuration docs in `README.md` and `user_config.rs`.

---

## Code style & conventions

These are enforced by Clippy (`-D warnings`) and code review.

| Convention | Rule |
|---|---|
| **Error handling** | Use `anyhow` for application-level errors, `thiserror` for library errors. Never `unwrap()` in library code — use `?` and return `Result`. |
| **No panics** | `panic!`, `unwrap()`, `expect()` are allowed only in tests and in documented invariant scenarios (add a `// SAFETY:` comment). |
| **No unsafe** | The codebase is `#![deny(unsafe_code)]`. Open an issue before adding any unsafe code. |
| **One canonical formula** | Do not inline calculations that already have a function. Use `scopeon_core::cache_hit_rate()` everywhere. |
| **Database queries** | Use indexes. Check with `EXPLAIN QUERY PLAN`. Avoid full table scans in hot paths. |
| **Logging** | `tracing::debug!` for per-turn detail, `tracing::info!` for startup events, `tracing::warn!` for recoverable errors, `tracing::error!` for data-integrity failures. |
| **Comments** | Comment *why*, not *what*. Explain intent, edge cases, and invariants. |
| **Clippy** | Zero warnings. `#[allow(...)]` requires a doc comment explaining why. |
| **Range checks** | Use `.contains()` over manual `>=` / `<` pairs (clippy `manual_range_contains`). |

---

## Testing

All tests live in `#[cfg(test)]` modules in the same file as the code they test.

```bash
cargo test --workspace                  # run all tests (131 tests, all offline)
cargo test -p scopeon-core              # run tests for one crate
cargo test cache_hit_rate               # run tests matching a name
cargo test -- --nocapture               # show println! output
```

### What to test

- **Pure functions** (cost calculations, context pressure, cache hit rate): exhaustive unit tests with representative inputs and edge cases.
- **Database functions**: use `Database::open_in_memory()` — no temp files needed. All 4 migrations run automatically on in-memory DBs.
- **Parsers**: include at least one test with a real-format sample log line.
- **MCP dispatch**: verify that each tool returns a valid JSON-RPC result structure.
- **Threshold calculations**: include tests for the P10/P90 percentile boundary cases (< 7 days → defaults, ≥ 7 days → computed).
- **Alert conditions**: test `check_alerts` with snapshots at boundary values (79%, 80%, 95%, 96%).
- **Prefix ordering**: `cost.rs` and `context.rs` each have a test asserting that no more-specific model prefix is shadowed by a less-specific one. Maintain this property when adding new models.

---

## Submitting a pull request

1. **Fork** the repository and create a branch: `git checkout -b feat/my-feature`
2. Make your changes and **write tests**.
3. **Run the full local suite:**
   ```bash
   make check   # fmt + clippy + tests + audit
   ```
4. **Add a CHANGELOG entry** under `[Unreleased]` in `CHANGELOG.md`.
5. **Open a PR** with a clear description of what changed and why.

The CI pipeline (defined in `.github/workflows/ci.yml`) runs automatically on every PR:

| Job | Blocks merge? |
|-----|--------------|
| Format check | ✅ Yes |
| Clippy | ✅ Yes |
| Tests (Linux, macOS, Windows) | ✅ Yes |
| MSRV build (Rust 1.86) | ✅ Yes |
| Release build | ✅ Yes |
| Docs (cargo doc) | ✅ Yes |
| Security audit | Advisory only |
| cargo-deny | Advisory only |
| Coverage | Advisory only |

**Draft PRs** skip CI entirely to save runner minutes. Click "Ready for review" to trigger CI.

### Commit message style

```
feat: add support for Windsurf provider
fix: prevent negative token offset wrapping to u64::MAX
perf: use dedicated read-only connection in snapshot task
docs: add webhook escalation guide to CONTRIBUTING.md
chore(deps): update tokio to 1.44
```

Subject line 72 characters or fewer. Reference issues where relevant (`Closes #42`).

---

## Branch protection setup (for repo maintainers)

To enforce CI on every PR, configure branch protection in
**Settings → Branches → Add branch protection rule** for `main`:

| Setting | Value |
|---------|-------|
| Require a pull request before merging | ✅ |
| Required approvals | 1 |
| Dismiss stale pull request approvals when new commits are pushed | ✅ |
| Require status checks to pass before merging | ✅ |
| Required status check | **CI pass** |
| Require branches to be up to date before merging | ✅ |
| Do not allow bypassing the above settings | ✅ (for non-admins) |

The single **"CI pass"** check is the only one to add to required status checks. It
acts as a fan-in: it only turns green when all mandatory jobs (fmt, clippy, test on 3
platforms, MSRV, release-build, docs) have passed. This gives you one clean name in the
UI instead of configuring every individual job.

---

## Reporting bugs

Open a [bug report](https://github.com/scopeon/scopeon/issues/new?template=bug_report.md) with:

- **Scopeon version**: `scopeon --version`
- **OS**: `sw_vers` (macOS) or `uname -r` (Linux)
- **AI provider** you are using
- **Steps to reproduce**
- **Expected vs actual** behaviour
- **Debug log** (very helpful): `RUST_LOG=scopeon=debug scopeon start 2>&1 | head -100`

---

## Proposing features

Open a [feature request](https://github.com/scopeon/scopeon/issues/new?template=feature_request.md) and describe:

- The problem you are solving
- How you imagine the feature working
- Alternatives you considered

For larger changes (new crate, major refactor, new CLI subcommand), open the issue first and discuss the approach before writing code. This saves everyone time.

---

## License

By contributing, you agree that your contributions will be licensed under the same **MIT OR Apache-2.0** terms as the rest of the project. No contributor license agreement is required.


---

## Table of contents

- [Code of conduct](#code-of-conduct)
- [Getting started](#getting-started)
- [Project structure](#project-structure)
- [Development workflow](#development-workflow)
- [Adding a new provider](#adding-a-new-provider)
- [Adding an MCP tool](#adding-an-mcp-tool)
- [Code style & conventions](#code-style--conventions)
- [Testing](#testing)
- [Submitting a pull request](#submitting-a-pull-request)
- [Reporting bugs](#reporting-bugs)
- [Proposing features](#proposing-features)

---

## Code of conduct

Be kind. Assume good faith. Disagreements on technical direction are fine — personal attacks are not. We reserve the right to remove comments or contributions that violate this principle.

---

## Getting started

### Prerequisites

| Tool | Version | Install |
|---|---|---|
| Rust | ≥ 1.86 | [rustup.rs](https://rustup.rs) |
| Git | any | [git-scm.com](https://git-scm.com) |

macOS 12+ or Linux (glibc 2.31+). Windows builds via cross-compilation in CI — local dev on Windows is not yet validated.

### Clone and build

```bash
git clone https://github.com/scopeon/scopeon
cd scopeon
cargo build                    # dev build (fast)
cargo build --release          # optimised build (used for releases)
```

### Run from source

```bash
cargo run -- status            # quick inline stats, no TUI
cargo run -- start             # full TUI (reads provider log files)
cargo run -- export --format csv --days 30
```

### Run tests

```bash
cargo test --workspace
```

All tests are **offline** — they use in-memory or temp-file SQLite databases. Claude Code, Copilot, or any other AI agent does not need to be installed.

### Lint and format

```bash
cargo clippy --workspace -- -D warnings    # must be zero warnings
cargo fmt --all                            # auto-format
cargo fmt --all -- --check                 # CI check (no modification)
```

---

## Project structure

```
scopeon/                    <- CLI binary (src/main.rs, src/ci.rs, src/serve.rs)
crates/
  scopeon-core/             <- models, SQLite schema + migrations, cost engine  <- start here
  scopeon-collector/        <- JSONL parsers + provider abstraction + file watcher
  scopeon-mcp/              <- MCP server (JSON-RPC 2.0 over stdio, 12 tools + pre-comp engine)
  scopeon-tui/              <- Ratatui dashboard (6 tabs, adaptive refresh state machine)
  scopeon-metrics/          <- Metric registry, waste signals, health score, suggestions
.github/
  workflows/ci.yml          <- CI pipeline (format, clippy, test, MSRV, release build, audit)
  workflows/release.yml     <- Cross-platform binary release (triggered on v* tags)
  examples/ai-cost-gate.yml <- Example: PR cost gate workflow
  ISSUE_TEMPLATE/           <- Bug and feature request templates
  PULL_REQUEST_TEMPLATE.md  <- PR checklist
```

### Where to start for common contributions

| Goal | Where to look |
|---|---|
| New AI provider | `crates/scopeon-collector/src/providers/` |
| New MCP tool | `crates/scopeon-mcp/src/server.rs` |
| New TUI tab or panel | `crates/scopeon-tui/src/` |
| New cost model / pricing | `crates/scopeon-core/src/cost.rs` |
| New metric or waste signal | `crates/scopeon-metrics/src/` |
| New CLI subcommand | `src/main.rs` |
| Database schema change | `crates/scopeon-core/src/db.rs` (add a migration) |

---

## Development workflow

### Running against your real data

```bash
# Uses your real ~/.claude/ logs and ~/.scopeon/scopeon.db
cargo run -- start

# Just status (fast, no TUI)
cargo run -- status

# Debug logging
RUST_LOG=scopeon=debug cargo run -- start
```

### Iterating on TUI layout

The TUI reads from SQLite — you do not need live log files. Just run `cargo run -- start` and it will use whatever data is already in `~/.scopeon/scopeon.db`.

### Iterating on the MCP server

```bash
# Start the MCP server over stdio
cargo run -- mcp

# Test individual JSON-RPC calls manually
echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_token_usage","arguments":{}}}' \
  | cargo run -- mcp
```

### Iterating on the HTTP API

```bash
cargo run -- serve --port 7771 --tier 3
# In another terminal:
curl http://localhost:7771/health
curl http://localhost:7771/api/v1/stats
```

---

## Adding a new provider

Providers live in `crates/scopeon-collector/src/providers/`. Each provider implements the `Provider` trait:

```rust
pub trait Provider: Send + Sync {
    /// Human-readable name shown in the TUI Providers tab.
    fn name(&self) -> &str;

    /// Return true if this provider's log files are present on disk.
    fn is_available(&self) -> bool;

    /// Directories to watch for file-change events (FSEvents / inotify).
    fn watch_paths(&self) -> Vec<PathBuf>;

    /// Walk all log files and persist new turns to the database.
    /// Must be idempotent — byte offsets prevent double-counting.
    fn scan(&self, db: &Database) -> Result<()>;
}
```

**Step-by-step:**

1. **Create `crates/scopeon-collector/src/providers/my_tool.rs`** implementing the trait above.
2. **Map the log format** to the shared models in `scopeon-core`:
   - `Session` — one per conversation
   - `Turn` — one per request/response pair (contains all token counts)
   - `ToolCall` — one per MCP/function tool use within a turn
3. **Register the provider** in `src/main.rs` inside `build_providers()`.
4. **Update the providers table** in `README.md`.
5. **Write at least one test** that parses a sample log line and asserts the correct `Turn` fields.

---

## Adding an MCP tool

MCP tools are registered in `crates/scopeon-mcp/src/server.rs`.

1. **Add the tool to `list_tools()`** — name, description, and JSON schema for parameters.
2. **Add a match arm in `dispatch()`** — call your DB query or MetricSnapshot field.
3. **If the result is cacheable**, add the field to `MetricSnapshot` and populate it in `refresh_snapshot()`.
4. **Write a doc comment** on the match arm explaining what the tool returns and when it is useful.

---

## Code style & conventions

These are enforced by Clippy (`-D warnings`) and code review.

| Convention | Rule |
|---|---|
| **Error handling** | Use `anyhow` for application-level errors, `thiserror` for library errors. Never `unwrap()` in library code — use `?` and return `Result`. |
| **No panics** | `panic!`, `unwrap()`, `expect()` are allowed only in tests and in documented invariant scenarios (add a `// SAFETY:` comment). |
| **No unsafe** | The codebase is `#![deny(unsafe_code)]`. Open an issue before adding any unsafe code. |
| **One canonical formula** | Do not inline calculations that already have a function. Use `scopeon_core::cache_hit_rate()` everywhere. |
| **Database queries** | Use indexes. Check with `EXPLAIN QUERY PLAN`. Avoid full table scans in hot paths. |
| **Logging** | `tracing::debug!` for per-turn detail, `tracing::info!` for startup events, `tracing::warn!` for recoverable errors, `tracing::error!` for data-integrity failures. |
| **Comments** | Comment *why*, not *what*. Explain intent, edge cases, and invariants. |

---

## Testing

All tests live in `#[cfg(test)]` modules in the same file as the code they test.

```bash
cargo test --workspace                  # run all tests
cargo test -p scopeon-core              # run tests for one crate
cargo test cache_hit_rate               # run tests matching a name
```

### What to test

- **Pure functions** (cost calculations, context pressure, cache hit rate): exhaustive unit tests with representative inputs and edge cases.
- **Database functions**: use `Database::open_in_memory()` — no temp files needed.
- **Parsers**: include at least one test with a real-format sample log line.
- **MCP dispatch**: verify that each tool returns a valid JSON-RPC result structure.
- **Prefix ordering**: `cost.rs` and `context.rs` each have a test asserting that no more-specific model prefix is shadowed by a less-specific one. Maintain this property when adding new models.

---

## Submitting a pull request

1. **Fork** the repository and create a branch: `git checkout -b feat/my-feature`
2. Make your changes and **write tests**.
3. **Run the full local suite:**
   ```bash
   cargo fmt --all
   cargo clippy --workspace -- -D warnings
   cargo test --workspace
   ```
4. **Add a CHANGELOG entry** under `[Unreleased]` in `CHANGELOG.md`.
5. **Open a PR** with a clear description of what changed and why.

The CI pipeline runs format check, Clippy, tests on Linux + macOS, MSRV build, release build, and security audit. All jobs must pass. The single required check for branch protection is **"CI pass"**.

### Commit message style

```
feat: add support for Windsurf provider
fix: prevent negative token offset wrapping to u64::MAX
perf: cache context_window_for_model results in a HashMap
docs: add provider integration guide to CONTRIBUTING.md
```

Subject line 72 characters or fewer. Reference issues where relevant (`Closes #42`).

---

## Reporting bugs

Open a [bug report](https://github.com/scopeon/scopeon/issues/new?template=bug_report.md) with:

- **Scopeon version**: `scopeon --version`
- **OS**: `sw_vers` (macOS) or `uname -r` (Linux)
- **AI provider** you are using
- **Steps to reproduce**
- **Expected vs actual** behaviour
- **Debug log** (very helpful): `RUST_LOG=scopeon=debug scopeon start 2>&1 | head -100`

---

## Proposing features

Open a [feature request](https://github.com/scopeon/scopeon/issues/new?template=feature_request.md) and describe:

- The problem you are solving
- How you imagine the feature working
- Alternatives you considered

For larger changes (new crate, major refactor, new CLI subcommand), open the issue first and discuss the approach before writing code. This saves everyone time.

---

## License

By contributing, you agree that your contributions will be licensed under the same **MIT OR Apache-2.0** terms as the rest of the project. No contributor license agreement is required.

