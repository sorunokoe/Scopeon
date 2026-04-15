<div align="center">

<h1>
  <img src="https://raw.githubusercontent.com/scopeon/scopeon/main/assets/logo.svg" alt="Scopeon" width="48" height="48" />
  <br/>Scopeon
</h1>

```
  ◈  ╔═╗╔═╗╔═╗╔═╗╔═╗╔═╗╔╗╔
     ╚═╗║  ║ ║╠═╝║╣ ║ ║║║║
     ╚═╝╚═╝╚═╝╩  ╚═╝╚═╝╝╚╝
     AI Context Observability
     for Claude Code & friends
```

**The AI context observatory — for every coding agent, every token, every dollar.**

[![CI](https://github.com/scopeon/scopeon/actions/workflows/ci.yml/badge.svg)](https://github.com/scopeon/scopeon/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/scopeon)](https://crates.io/crates/scopeon)
[![Downloads](https://img.shields.io/crates/d/scopeon)](https://crates.io/crates/scopeon)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](LICENSE-MIT)
[![MSRV: 1.86](https://img.shields.io/badge/MSRV-1.86-orange)](https://blog.rust-lang.org/2025/02/20/Rust-1.86.0.html)

[**Install**](#installation) · [**Quick Start**](#quick-start) · [**Features**](#features) · [**TUI**](#tui-dashboard) · [**Browser Dashboard**](#browser-dashboard) · [**MCP Tools**](#mcp-tools) · [**CI Gate**](#ci-integration) · [**Shell & Git**](#shell--git-integration) · [**Team Mode**](#team-mode) · [**Config**](#configuration) · [**Contributing**](#contributing)

---

*You fire up Claude Code and start building. An hour later: **"Context window full."** You have no idea what burned it — was it the MCP tools? The thinking budget? Yesterday's file edits? You're flying blind on a meter that costs real money.*

**Scopeon gives you the instrument panel.**

```
╔══════════════════════════════════ Scopeon v0.6.0 ═══════════════════════════════════════╗
║  Health 91/100  ▪  Cost today $2.41  ▪  Cache 68.3%  ▪  Ctx 73% ████████░░ ~12t [!]   ║
╠════════════╦════════════╦═══════════╦═══════════╦═════════════╦════════════════════════╣
║ Dashboard  ║  Sessions  ║ Insights  ║  Budget   ║  Providers  ║        Agents          ║
╠════════════╩════════════╩═══════════╩═══════════╩═════════════╩════════════════════════╣
║                                                                                          ║
║  TURN BREAKDOWN — last 5 turns                                                           ║
║  ┌──────┬────────────┬───────────┬───────────┬──────────┬─────────┬──────────┐          ║
║  │ Turn │ Input      │ Cache↓    │ Cache↑    │ Thinking │ Output  │ Cost     │          ║
║  ├──────┼────────────┼───────────┼───────────┼──────────┼─────────┼──────────┤          ║
║  │  142 │  48,291    │  112,443  │      0    │  12,800  │  2,847  │ $0.182   │          ║
║  │  141 │  46,102    │  115,201  │      0    │   8,192  │  1,923  │ $0.154   │          ║
║  │  140 │  44,887    │  110,988  │  24,601   │  16,384  │  3,211  │ $0.231   │          ║
║  └──────┴────────────┴───────────┴───────────┴──────────┴─────────┴──────────┘          ║
║                                                                                          ║
║  PROMPT CACHE ████████████████████████░░░░░░  68.3% hit rate  saved $1.82 today        ║
║  CONTEXT      █████████████████████████████████████░░░░░░  73.1% used  ~12 turns left  ║
╚══════════════════════════════════════════════════════════════════════════════════════════╝
```

</div>

---

## Why Scopeon?

AI coding agents are powerful but opaque. Scopeon makes them transparent:

| Without Scopeon | With Scopeon |
|---|---|
| "Why is this session so expensive?" | Turn-by-turn cost breakdown with waste signals |
| "Is the prompt cache actually working?" | Hit-rate gauge, USD saved, optimization suggestions |
| "How close am I to the context limit?" | Real-time fill bar + *"~12 turns remaining"* prediction |
| "Which project costs the most?" | Per-project / per-branch cost breakdown |
| "Did my optimization actually help?" | `compare_sessions` before/after diff |
| "Can I gate AI cost in CI?" | `scopeon ci report --fail-on-cost-delta 50` |
| "How is my whole team using AI?" | `scopeon serve` — privacy-filtered team API |

---

## Features

### 📊 Core Observability

| Capability | Detail |
|---|---|
| **Token breakdown per turn** | Input · cache reads (↓ cheap) · cache writes (↑ one-time) · thinking · output · MCP calls — one line per turn |
| **Prompt cache intelligence** | Hit-rate gauge, tokens saved, USD saved vs. uncached, per-model pricing |
| **Predictive context countdown** | Linear trend on last 10 turns → *"~12 turns remaining"* before context exhaustion |
| **Context window pressure** | Fill % bar per model (green → yellow → red), alerts at 80% and 95%, adaptive TUI refresh |
| **Cost estimation** | Per-model pricing applied to every turn; daily / weekly / monthly totals |
| **Budget guardrails** | Configurable USD limits with progress bars and alert banners |

### 🤖 Agent Intelligence

| Capability | Detail |
|---|---|
| **MCP integration** | 14 tools callable inside the agent — let it self-monitor and self-optimize |
| **Proactive push alerts** | MCP server sends JSON-RPC notifications when context > 80%, budget > 90%, ≤5 turns left, or compaction detected — without polling |
| **Webhook escalation** | HTTP POST to Slack/Discord/custom on any alert type — configurable per event |
| **Adaptive thresholds** | Your own P10/P90 percentile thresholds computed from 90 days of history — not hard-coded |
| **Waste analysis** | Severity-weighted signals (Critical/Warning/Info) + actionable suggestions from cross-session intelligence |

### 🏷️ Organization

| Capability | Detail |
|---|---|
| **Session tagging** | `scopeon tag set <id> feature research` — attribute costs to business categories |
| **Auto git branch tags** | `feat/` → `[feature]`, `fix/` → `[bugfix]` displayed in Sessions tab automatically |
| **Cost by tag** | `get_cost_by_tag` MCP tool — "how much did the authentication feature cost in AI?" |
| **Multi-agent tree** | Visualize parent ↔ sub-agent cost hierarchies with per-node totals |

### ⌨️ Shell & Git
| Capability | Detail |
|---|---|
| **Shell prompt integration** | `scopeon shell-hook` injects `$SCOPEON_STATUS` into every shell prompt — see health score, context fill, and daily cost at a glance |
| **Git commit trailer** | `scopeon git-hook install` appends an `AI-Cost:` line to every commit message — cost visible in `git log` forever |
| **Interactive onboarding** | `scopeon onboard` auto-detects installed AI tools and configures MCP + shell integration in one wizard |
| **Health diagnostics** | `scopeon doctor` prints memory usage, DB stats, provider availability, and overhead proof |
| **Shields.io badges** | `scopeon badge` generates live daily-cost and cache-rate badge URLs for your project README |
| **Weekly digest** | `scopeon digest` produces a Markdown report ready to share or post to Slack/Discord |

### 🔭 Advanced TUI

| Capability | Detail |
|---|---|
| **Temporal Replay** | Press `→`/`←` in session detail to scrub through every turn — see context fill, cost, and tokens at any point in history |
| **Zen Mode** | Press `z` to collapse the entire TUI to a single ambient line: `health · context% · daily cost · cache%`. Auto-exits at context ≥ 80% |
| **Natural-language filter** | Press `/` in Sessions — supports `cost>5`, `cache<20`, `tag:feature`, `model:sonnet`, `today`, `anomaly` predicates |
| **Narrative insights header** | Status bar rotates natural-language sentences: "Cache bust anomaly on turn 140 — $0.43 wasted" instead of raw KPI chips |
| **Anomaly cards** | Insights tab renders severity-bordered cards with actionable titles instead of a flat list |
| **End-of-day projection** | Budget tab shows `EOD: $X.XX of $Y.YY · at $Z.ZZZ/hr` — warns when hourly pace would exceed 90% of daily limit |
| **Overhead transparency** | Status bar appends `[◈ X.XMB]` showing Scopeon's own RSS memory footprint |

### 🌐 Team & Integration

| Capability | Detail |
|---|---|
| **Browser dashboard** | `scopeon serve` → open `http://localhost:7771` — live WebSocket charts, zero npm |
| **CI cost gate** | `scopeon ci report --fail-on-cost-delta 50` — fail PRs when AI cost spikes |
| **Privacy-filtered HTTP API** | Four tiers: health-only to full metrics — share locally or behind a trusted reverse proxy |
| **Export** | JSON or CSV for external analysis and data pipelines |
| **Reprice** | Recalculate all historical costs after a provider price change in seconds |

### ⚙️ Engineering

| Capability | Detail |
|---|---|
| **Pre-computed metrics** | Background snapshot serves most default MCP reads with zero DB queries once warm |
| **Read-connection pool** | WAL-mode SQLite + dedicated read-only snapshot path to reduce writer contention |
| **Multi-provider support** | Claude Code · GitHub Copilot CLI · Aider · Cursor · Gemini CLI · Ollama · Generic OpenAI |
| **Local-first, no cloud backend** | No accounts or hosted service. Data stays on your machine unless you opt into LAN serving or webhooks. |

---

## Installation

### Fastest: `cargo binstall` (pre-built binary, no compilation)

```bash
cargo install cargo-binstall   # one-time
cargo binstall scopeon
```

### `curl` one-liner (macOS & Linux)

```bash
curl -fsSL https://raw.githubusercontent.com/scopeon/scopeon/main/install.sh | sh
```

Auto-detects your OS and CPU architecture, installs to `~/.local/bin`.

### From source

```bash
cargo install scopeon
```

**Requirements:** Rust 1.86+ · macOS 12+ or Linux (glibc 2.31+)

### Manual download

Pre-built binaries for every platform on [GitHub Releases](https://github.com/scopeon/scopeon/releases):

| Platform | Asset |
|---|---|
| macOS Apple Silicon (M1–M4) | `scopeon-vX.Y.Z-aarch64-apple-darwin.tar.gz` |
| macOS Intel | `scopeon-vX.Y.Z-x86_64-apple-darwin.tar.gz` |
| Linux x86\_64 | `scopeon-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz` |
| Linux ARM64 | `scopeon-vX.Y.Z-aarch64-unknown-linux-gnu.tar.gz` |
| Windows x86\_64 | `scopeon-vX.Y.Z-x86_64-pc-windows-msvc.zip` |

```bash
tar xzf scopeon-*.tar.gz && mv scopeon ~/.local/bin/
```

---

## Quick Start

### 0 — Interactive onboarding (recommended for first-time users)

```bash
scopeon onboard
```

Auto-detects every AI tool installed on your machine, configures MCP for Claude Code, and sets up shell integration in one wizard. Skip steps 1–2 below if you use this.

### 1 — Open the TUI dashboard

```bash
scopeon
```

That's it. Scopeon discovers your AI log files automatically and opens the dashboard.

### 2 — Connect to Claude Code (MCP integration)

```bash
scopeon init
```

This writes the MCP server config to `~/.claude/settings.json`. After this, Claude Code exposes all 14 Scopeon tools in its own context — the agent can call `get_context_pressure` before starting a long task, `get_budget_status` to stay within limits, and receive **proactive push notifications** when context is running low.

### 3 — Open the browser dashboard

```bash
scopeon serve
# → open http://localhost:7771
```

Live WebSocket charts update every 2 seconds. No npm, no CDN, no build step.

### 4 — Quick inline stats (no TUI)

```bash
scopeon status
```

```
◈ Scopeon v0.6.0 — health 91/100

Active session    abc12345  (claude-code · [feature])
  Turns           142
  Context fill    73.1%  ██████████████████████████████░░░  ~12 turns left
  This session    $2.84

Today
  Cost            $2.84  of $5.00 limit  ████████████████████████░░░░░░  56.8%
  Cache           68.3%  saved $1.82 vs. uncached
  Turns           142
  Tokens          1,847,291

[◈ 12.3MB]  No warnings.
```

### 5 — Diagnose issues

```bash
scopeon doctor
```

Prints memory footprint, DB stats, provider log paths, and whether each provider is being tracked correctly. Run this if sessions aren't appearing.

### 4 — All commands

```bash
# ── Core ─────────────────────────────────────────────────────────────
scopeon                        # start daemon + open TUI (default)
scopeon start                  # same as above (explicit)
scopeon tui                    # open TUI only (no file watching)
scopeon mcp                    # run MCP server over stdio
scopeon status                 # quick inline stats, no TUI
scopeon serve                  # start browser dashboard + HTTP API
scopeon serve --port 8080 --tier 2

# ── Session management ────────────────────────────────────────────────
scopeon tag set <session-id> feature auth   # tag a session
scopeon tag show <session-id>
scopeon tag list feature

# ── Data management ───────────────────────────────────────────────────
scopeon export --format json --days 30
scopeon export --format csv  --output report.csv
scopeon reprice               # recalculate costs after a provider price change

# ── Reports & sharing ─────────────────────────────────────────────────
scopeon digest                # 7-day Markdown report to stdout
scopeon digest --days 30 --post-to-slack <webhook-url>
scopeon badge                 # shields.io badge Markdown snippets
scopeon badge --format url    # raw badge URLs

# ── CI cost gate ──────────────────────────────────────────────────────
scopeon ci snapshot --output baseline.json
scopeon ci report   --baseline baseline.json
scopeon ci report   --baseline baseline.json --fail-on-cost-delta 50

# ── Shell & Git integration ───────────────────────────────────────────
scopeon shell-hook            # emit shell prompt hook (bash/zsh/fish)
scopeon git-hook install      # add AI-Cost trailer to git commits
scopeon git-hook uninstall    # remove the hook

# ── Setup & diagnostics ───────────────────────────────────────────────
scopeon onboard               # interactive setup wizard
scopeon init                  # configure Claude Code MCP integration
scopeon doctor                # health diagnostics + overhead proof
```

---

## TUI Dashboard

Six keyboard-navigable tabs — press a number to jump directly.

| Tab | Key | What you see |
|---|---|---|
| **Dashboard** | `1` | Narrative health header · active session · token breakdown · cache gauge · context bar |
| **Sessions** | `2` | Session list with git-branch tags · filter predicates · per-turn drill-down · temporal replay |
| **Insights** | `3` | Severity-bordered anomaly cards · cross-session suggestions · trend chart |
| **Budget** | `4` | Daily/weekly/monthly spend vs. limits · EOD projection · adaptive threshold bands |
| **Providers** | `5` | Status · session count · last-seen per provider |
| **Agents** | `6` | Multi-agent tree with depth indentation · per-agent cost and token totals |

### Keyboard shortcuts

| Key | Action |
|---|---|
| `1` – `6` / `Tab` | Switch tab |
| `↑` / `k` | Scroll up |
| `↓` / `j` | Scroll down |
| `Enter` | Open session detail / fullscreen |
| `→` / `l` | Replay: step forward one turn |
| `←` / `h` | Replay: step backward (first `←` exits replay) |
| `/` | Open session filter bar |
| `z` / `Z` | Toggle Zen mode (ambient single-line display) |
| `r` | Force refresh |
| `?` | Help overlay |
| `q` | Quit |

### Session filter predicates

Press `/` in the Sessions tab to filter with plain text **or** structured predicates:

```
/cost>5          sessions that cost more than $5
/cache<20        sessions with cache hit rate below 20%
/tag:feature     sessions tagged "feature"
/model:sonnet    sessions using a model containing "sonnet"
/today           sessions active today
/anomaly         sessions flagged by the waste-analysis engine
```

Predicates can be combined with text search — unknown tokens fall through to substring match. Invalid values (e.g. `cost>abc`) display an amber parse-error hint.

### Zen Mode

Press `z` anywhere in the TUI to collapse the dashboard to a single ambient line:

```
              ⬡87  ·  Ctx 73% ████████░░  ·  $2.41  ·  Cache 68%
```

Zen auto-exits when context ≥ 80% or daily budget ≥ 90% — urgency always wins. It auto-restores after pressure clears.

### Temporal Replay

In the Sessions tab, open a session with `Enter` then press `→` to scrub through its turns one by one. Each step shows a snapshot panel:

```
┌─ Replay — turn 140 of 142 ─────────────────────────────────────────┐
│  Context     73.1% ████████████████████████████████████░░░          │
│  Input       44,887 tok   Cache↓  110,988 tok   Cache↑  24,601 tok  │
│  Thinking    16,384 tok   Output    3,211 tok                        │
│  Turn cost   $0.231        Cumulative  $28.42                        │
│                                           ← step back  → step fwd   │
└─────────────────────────────────────────────────────────────────────┘
```

### Adaptive refresh

The TUI dynamically adjusts its refresh rate so data is current when it matters:

| Context fill | Refresh interval | State |
|---|---|---|
| < 50% | Every 2 s | Idle |
| 50 – 80% | Every 500 ms | Active |
| > 80% | Every 100 ms | Crisis |

---

## Browser Dashboard

`scopeon serve` starts a local HTTP server with a **live WebSocket browser dashboard**.

```bash
scopeon serve             # port 7771, tier 1 (aggregate stats)
scopeon serve --tier 3    # full metrics including per-turn detail
```

Open **http://localhost:7771** in any browser. The dashboard shows:

- 🟢 **Context pressure** — fill bar + predicted turns remaining
- 💰 **Daily cost** — spend vs. budget gauge
- ⚡ **Cache efficiency** — hit rate and USD saved
- 📊 **Token usage** — input, output, cache, thinking breakdown
- 📅 **30-day cost history** — bar chart with hover detail
- 🔴 **Live alerts** — push banners for context crisis, budget warnings

Updates every 2 seconds via WebSocket — no page refresh needed. Zero npm, zero CDN, zero build step. The entire dashboard is a single `include_str!`-embedded HTML file.

### Privacy tiers

Control exactly what data the server exposes:

| Tier | Flag | Data exposed |
|---|---|---|
| **0** | `--tier 0` | `/health` only — just "is it running?" |
| **1** | `--tier 1` | Aggregate totals: cost, cache rate, turn count **(default)** |
| **2** | `--tier 2` | Per-session metadata (no prompt content) |
| **3** | `--tier 3` | Full metrics including per-turn breakdown |

---

## MCP Tools

After `scopeon init`, these tools are callable from inside the agent's own context. The agent can self-monitor, act on data, and receive proactive push notifications — **without using any token budget for polling**.

| Tool | Description |
|---|---|
| `get_token_usage` | Current session token breakdown: input, output, cache reads/writes, thinking, MCP |
| `get_session_summary` | Per-turn stats for the current or a named session |
| `get_cache_efficiency` | Prompt cache hit rate, tokens saved, USD saved vs. no-cache |
| `get_history` | Daily rollup for the last N days |
| `compare_sessions` | Before/after token and cost comparison — measure optimization impact |
| `get_context_pressure` | Context fill %, tokens remaining, `predicted_turns_remaining`, `should_compact` |
| `get_budget_status` | Current spend vs. daily / weekly / monthly limits |
| `get_optimization_suggestions` | Actionable suggestions from the adaptive waste-analysis engine |
| `suggest_compact` | Boolean: should the agent call `/compact` right now? |
| `get_project_stats` | Cost and cache breakdown by project and branch |
| `list_sessions` | Recent sessions with cost and cache metadata |
| `get_agent_tree` | Multi-agent hierarchy with per-agent cost totals |
| `set_session_tags` | Tag the current session for cost attribution (`feature`, `research`, `debug`) |
| `get_cost_by_tag` | Total cost grouped by tag — "how much did auth work cost in AI?" |

### Proactive push notifications

The MCP server sends JSON-RPC **notifications** (no `id` field = zero token cost, per JSON-RPC 2.0 §4) to Claude Code's MCP client when:

| Condition | Alert type | Severity |
|---|---|---|
| Context ≥ 95% | `context_crisis` | Critical |
| Context 80–94% | `context_warning` | Warning |
| Daily spend > 90% of limit | `budget_warning` | Warning / Critical |
| Predicted turns remaining ≤ 5 | `low_turns_left` | Warning |
| Auto-compaction detected | `compaction_detected` | Info |

Each alert kind has a **60-second cooldown** to prevent notification spam.

### Letting the agent manage its own context

Add this to your Claude Code system prompt:

```
Before any long task: call get_context_pressure.
If should_compact is true OR fill_pct > 85 OR predicted_turns_remaining < 10: call /compact.
After completing a task: call compare_sessions to report token efficiency.
When starting a new feature: call set_session_tags to attribute costs correctly.
```

---

## Webhook Escalation

Scopeon can POST alerts to any HTTP endpoint — Slack, Discord, PagerDuty, custom services.

```toml
# ~/.scopeon/config.toml

[[alerts.webhooks]]
url    = "https://hooks.slack.com/services/T.../B.../..."
events = ["context_crisis", "budget_warning"]

[[alerts.webhooks]]
url    = "https://discord.com/api/webhooks/..."
events = ["context_crisis"]

[[alerts.webhooks]]
url    = "http://localhost:9999/ai-alerts"
events = []   # empty = all event types
```

Alert payload (JSON POST body):

```json
{
  "method": "notifications/scopeon/alert",
  "params": {
    "type": "context_crisis",
    "severity": "critical",
    "message": "Context window 96.2% full — run /compact immediately",
    "fill_pct": 96.2,
    "should_compact": true
  }
}
```

---

## Session Tagging

Attribute AI costs to business features, projects, or work types.

```bash
# Tag a session (get session IDs from `scopeon status` or the Sessions tab)
scopeon tag set abc123 feature authentication

# View tags on a session
scopeon tag show abc123
# tags: feature, authentication

# Find all sessions with a tag
scopeon tag list feature

# Clear tags
scopeon tag set abc123   # empty = clear all tags
```

Or from inside Claude Code using the MCP tool:

```
set_session_tags(tags=["feature", "payment-integration"])
get_cost_by_tag(tag="feature")
# → { "tag": "feature", "total_cost_usd": 12.40, "sessions": 8, "turns": 234 }
```

---

## CI Integration

Gate pull requests on AI cost — catch regressions before they reach `main`.

### How it works

1. **On `main`** — save a baseline snapshot after merging.
2. **On each PR** — compare current AI usage against the baseline.
3. **Post as a PR comment** — reviewers see cost and cache changes at a glance.

### Usage

```bash
# Save baseline (run on main)
scopeon ci snapshot --output baseline.json

# Compare in CI
scopeon ci report --baseline baseline.json

# Hard gate: fail if AI cost increased by more than 50%
scopeon ci report --baseline baseline.json --fail-on-cost-delta 50
```

### GitHub Actions workflow

```yaml
# .github/workflows/ai-cost-gate.yml
name: AI Cost Gate

on:
  pull_request:
    branches: [main]

jobs:
  ai-cost-gate:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Scopeon
        run: curl -fsSL https://raw.githubusercontent.com/scopeon/scopeon/main/install.sh | sh

      - name: Download baseline
        run: gh release download --pattern baseline.json --dir .
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}

      - name: Generate cost report
        run: |
          ~/.local/bin/scopeon ci report \
            --baseline baseline.json \
            --fail-on-cost-delta 50 > cost-report.md

      - name: Post PR comment
        run: gh pr comment ${{ github.event.pull_request.number }} --body-file cost-report.md
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

Full example at [`.github/examples/ai-cost-gate.yml`](.github/examples/ai-cost-gate.yml).

### Example report output

```markdown
## 🔬 AI Cost Analysis

Comparing current (2026-04-13) to baseline (2026-03-01).

| Metric           | Baseline  | Current   | Delta       |
|------------------|-----------|-----------|-------------|
| Total cost       | $41.20    | $38.80    | -5.8%  🟢  |
| Cache hit rate   | 48.3%     | 68.3%     | +20.0% 🟢  |
| Context peak     | 91.2%     | 73.1%     | -18.1% 🟢  |
| Avg tokens/turn  | 12,400    | 8,468     | -31.7% 🟢  |
| Sessions         | 120       | 188       | —           |
| Turns            | 3,100     | 4,521     | —           |
```

---

## Team Mode

Run `scopeon serve` on each developer's machine. A shared dashboard can poll all instances for team-wide AI usage visibility — with zero cloud, zero data leaving the machine.

```bash
# On each developer machine:
scopeon serve --tier 1   # aggregate stats only (default)

# On the dashboard machine:
scopeon serve --tier 3   # full metrics, local network only
```

### REST endpoints

```
GET /health              → { status, version, uptime_secs, tier }         (all tiers)
GET /api/v1/stats        → token totals, cost totals, cache hit rate       (tier ≥ 1)
GET /api/v1/budget       → daily/weekly/monthly spend vs. limits           (tier ≥ 1)
GET /api/v1/sessions     → session list with cost and cache metadata       (tier ≥ 2)
GET /api/v1/context      → live context pressure for the active session    (tier ≥ 2)
WS  /ws/v1/metrics       → real-time WebSocket stream (2 s snapshots)      (tier ≥ 1)
```

All responses are JSON with CORS headers — ready to consume from any browser dashboard or monitoring tool.

---

## Shell & Git Integration

### Shell prompt status

Add live AI context stats to your shell prompt — health score, context fill, and daily cost appear on every command:

```bash
# Add to ~/.zshrc or ~/.bashrc
eval "$(scopeon shell-hook)"

# Then add $SCOPEON_STATUS to your prompt (RPROMPT in zsh, right-prompt in fish)
# Result on every prompt:
#   ⬡87  73%  $2.41
```

Fish shell:
```fish
# ~/.config/fish/config.fish
scopeon shell-hook --shell fish | source
```

The `⬡N` health score, context fill percentage, and daily cost update every time you draw the prompt. When context ≥ 80% the fill indicator turns amber; ≥ 95% turns red.

### Git commit cost trailer

Track exactly how much AI work went into each commit — visible forever in `git log`:

```bash
# Install the hook in your repository
scopeon git-hook install

# Every commit automatically gets an AI-Cost trailer:
# AI-Cost: $0.23 (14 turns, 182k tokens, 68% cache)
```

View it in git log:

```bash
git log --format="%h %s%n%b" | grep -A1 "AI-Cost"
# abc1234 feat: implement user authentication
# AI-Cost: $1.84 (42 turns, 891k tokens, 71% cache)
```

Remove the hook at any time: `scopeon git-hook uninstall`

### Weekly digest

Generate a Markdown report for sharing with your team or posting to Slack:

```bash
scopeon digest                                   # 7-day report to stdout
scopeon digest --days 30 > monthly-report.md
scopeon digest --post-to-slack <webhook-url>     # post directly to Slack
scopeon digest --post-to-discord <webhook-url>
```

```markdown
## AI Usage Digest — 2026-04-08 to 2026-04-15

### Executive Summary
| Metric | Value |
|---|---|
| Total cost | $18.40 |
| Sessions | 23 |
| Turns | 614 |
| Cache hit rate | 71.2% |
| Est. savings vs. no-cache | $12.30 |

### Top Optimization Recommendations
1. **High thinking ratio on model claude-sonnet** — consider capping thinking budget
2. **Cache bust on 4 sessions** — check system prompt stability
3. **3 sessions without tags** — tag work items for better cost attribution
```

### Badges

Add live AI usage badges to your project README:

```bash
scopeon badge   # outputs Markdown snippets
```

```markdown
![Daily AI Cost](https://img.shields.io/badge/AI_cost-$2.41%2Fday-blue)
![Cache Hit Rate](https://img.shields.io/badge/cache-68.3%25-brightgreen)
```

---

## Optimization Workflow

The core value of Scopeon is the **feedback loop**: try an optimization, measure the difference.

```bash
# 1. Capture state before trying something new
scopeon ci snapshot --output before.json

# ... make changes: tune system prompts, switch models, restructure cache headers ...

# 2. Capture after
scopeon ci snapshot --output after.json

# 3. See the delta
scopeon ci report --baseline before.json
```

Or directly from Claude Code:

```
compare_sessions(
  session_a = "abc123",   # session before optimization
  session_b = "def456"    # session after
)
```

```json
{
  "cache_hit_rate_delta_pct": 18.4,
  "cost_delta_usd": -0.73,
  "tokens_saved": 142800,
  "recommendation": "Cache efficiency improved significantly. Keep system prompt stable."
}
```

---

## Supported Providers

Scopeon discovers log files automatically — no configuration needed for standard install paths.

| Provider | Log location | Notes |
|---|---|---|
| **Claude Code** | `~/.claude/projects/**/*.jsonl` | Full breakdown: thinking, MCP, cache reads/writes |
| **GitHub Copilot CLI** | `~/.config/github-copilot/**/*.jsonl` | Token + cost tracking |
| **Aider** | `~/.aider/logs/*.jsonl` | Input / output token tracking |
| **Cursor** | `~/.cursor/logs/**/*.jsonl` | OpenAI-compatible format |
| **Gemini CLI** | `~/.gemini/logs/**/*.jsonl` | Gemini 1.5 / 2.0 / 2.5 pricing |
| **Ollama** | Local API polling | Free — no cost tracking |
| **Generic OpenAI** | Configurable paths | Set `generic_paths` in `~/.scopeon/config.toml` |

Want to add a provider? It takes ~50 lines of Rust. See [Adding a Provider](CONTRIBUTING.md#adding-a-new-provider).

---

## Configuration

`~/.scopeon/config.toml` — created automatically on first run.

```toml
[general]
refresh_interval_secs = 2        # TUI base refresh (adaptive override activates at >50% fill)
theme = "standard"               # "standard" | "high-contrast"

[providers]
enabled = [
  "claude-code", "copilot-cli", "aider",
  "cursor", "gemini-cli", "ollama"
]
generic_paths = []               # extra log directories for Generic OpenAI provider
generic_name  = "Custom Agent"

[budget]
daily_usd   = 5.0                # 0.0 = no limit; TUI shows progress bar + alert
weekly_usd  = 20.0
monthly_usd = 50.0

[alerts]
daily_cost_usd     = 5.0         # TUI banner when daily cost exceeds this
cache_hit_rate_min = 0.20        # warn when cache hit rate drops below 20%

# Webhook escalation — POST JSON alerts to any HTTP(S) endpoint
[[alerts.webhooks]]
url    = "https://hooks.slack.com/services/T.../B.../..."
events = ["context_crisis", "budget_warning"]  # empty = all events

# Custom model pricing overrides
[pricing]
# override_file = "~/.scopeon/my-pricing.toml"
```

### Environment variables

| Variable | Default | Description |
|---|---|---|
| `SCOPEON_DB` | `~/.scopeon/scopeon.db` | Override database path |
| `SCOPEON_CONFIG` | `~/.scopeon/config.toml` | Override config path |
| `RUST_LOG` | — | Set to `scopeon=debug` for verbose logs |

---

## Architecture

```
scopeon (CLI binary)
├── scopeon-core        Models · SQLite (7 migrations, WAL) · cost engine · context windows · tags
├── scopeon-collector   JSONL parsers (7 providers) · FSEvents/inotify watcher · byte-offset tracking
├── scopeon-mcp         JSON-RPC 2.0 over stdio · MetricSnapshot cache · push notifications · webhooks
├── scopeon-tui         Ratatui 6-tab dashboard · adaptive state machine · predictive turns display
└── scopeon-metrics     Health score · waste signals · adaptive thresholds · cross-session suggestions
```

### Pre-computation engine

The MCP server maintains a `MetricSnapshot` refreshed by a background task at an adaptive rate (5 s idle → 1 s active → 200 ms crisis). Once the snapshot is warm, cacheable MCP tools called with default arguments read from it with **zero database queries**. Parameterized queries and mutations still hit SQLite directly.

```
Agent context
    │  MCP call: get_context_pressure
    ▼
scopeon-mcp  ──read──▶  MetricSnapshot (Arc<RwLock>, ~200 ns)
                              │
                    ┌─────────┘  (refreshed 200 ms – 5 s)
                    ▼
              read-only connection  ──WAL──▶  ~/.scopeon/scopeon.db
              (no mutex contention                  (write: watcher)
               with the writer)
```

### Push notification flow

```
snapshot task  ──check_alerts──▶  mpsc channel  ──tokio::select!──▶  stdout (JSON-RPC notification)
                                                                              │
                                                                   fire_webhooks() (async, non-blocking)
```

---

## Data & Privacy

- **Local-first** — there is no mandatory cloud backend. Network exposure only happens if you opt into `scopeon serve` or configure webhooks.
- Reads token counts and costs from log files already written by the provider — never the actual prompt text or code.
- SQLite database at `~/.scopeon/scopeon.db` (WAL mode, auto-migrated on upgrade).
- Byte offsets tracked per file — restarts never double-count events.
- `scopeon serve` is read-only and never writes to the database.
- Webhooks are opt-in; payload contains only metric data (no prompts, no code) and can target HTTP(S) endpoints. Prefer HTTPS or a trusted local relay for sensitive environments.

---

## Frequently Asked Questions

**Does Scopeon work without Claude Code?**
Yes. All 7 providers work with the TUI, export, and CI commands. The MCP integration and push notifications are Claude Code-specific but entirely optional.

**Is my code or conversation content stored?**
No. Scopeon reads token *counts* and *costs* — not the actual prompt text or code. Your IP stays in the provider's log files (which the provider already wrote before Scopeon ran).

**Can I use Scopeon in a corporate environment?**
Yes. The tool is entirely local. `scopeon serve` is read-only and LAN-only. No data leaves the machine; no accounts or API keys are required.

**How does the predictive "turns remaining" work?**
Scopeon fits a least-squares linear trend to the last 10 turns' token consumption. If tokens per turn are growing, it extrapolates how many turns remain before the context window is full. The estimate appears as `~12t` in the status bar and as `predicted_turns_remaining` in the `get_context_pressure` MCP response.

**What are adaptive thresholds?**
If you have ≥7 days of history, Scopeon computes your personal P10/P90 percentiles for cache hit rate and thinking ratio from the last 90 days. Waste signals and suggestions use *your* baseline, not a one-size-fits-all hard-coded number.

**How do I reset the database?**
```bash
rm ~/.scopeon/scopeon.db
scopeon start          # fresh backfill from all provider logs
```

**How accurate are the cost estimates?**
Very accurate — token counts come directly from the provider's own telemetry. Pricing is sourced from provider rate cards and kept current. Run `scopeon reprice` after any price change to update historical data.

**My provider isn't listed.**
See [Adding a New Provider](CONTRIBUTING.md#adding-a-new-provider) — it typically takes ~50 lines of Rust and one PR.

---

## Contributing

Contributions are warmly welcomed — from bug fixes to new provider parsers to dashboard features.

- **[CONTRIBUTING.md](CONTRIBUTING.md)** — local setup, dev workflow, PR process
- **[ARCHITECTURE.md](ARCHITECTURE.md)** — codebase map: crate roles, data flow, schema, conventions

```bash
git clone https://github.com/scopeon/scopeon
cd scopeon
make          # fmt-check + clippy + test (same as CI)
make install  # install binary to ~/.cargo/bin
```

Every PR runs the full CI suite: format check, Clippy, tests on Linux + macOS, MSRV build, release build, docs build, and security audit. The CI badge must be green before merge.

---

## Changelog

See **[CHANGELOG.md](CHANGELOG.md)** for the full version history including the complete TRIZ v2 implementation details.

---

## License

Dual-licensed under **[MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE)** — use it however you like, commercially or privately, with no restrictions.

---

<div align="center">

Built with ❤️ in Rust · [Report a bug](https://github.com/scopeon/scopeon/issues/new?template=bug_report.md) · [Request a feature](https://github.com/scopeon/scopeon/issues/new?template=feature_request.md) · [Join the discussion](https://github.com/scopeon/scopeon/discussions)

*If Scopeon saved you money or context headaches, consider giving it a ⭐ — it helps others find the project.*

</div>

