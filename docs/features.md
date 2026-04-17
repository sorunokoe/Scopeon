# Features

## 📊 Core Observability

| Capability | Detail |
|---|---|
| **Token breakdown per turn** | Input · cache reads (↓ cheap) · cache writes (↑ one-time) · thinking · output · MCP calls — one line per turn |
| **Prompt cache intelligence** | Hit-rate gauge, tokens saved, USD saved vs. uncached, per-model pricing |
| **Predictive context countdown** | Linear trend on last 10 turns → *"~12 turns remaining"* before context exhaustion |
| **Context window pressure** | Fill % bar per model (green → yellow → red), alerts at 80% and 95%, adaptive TUI refresh |
| **Cost estimation** | Per-model pricing applied to every turn; daily / weekly / monthly totals |
| **Budget guardrails** | Configurable USD limits with progress bars and alert banners |

## 🤖 Agent Intelligence

| Capability | Detail |
|---|---|
| **MCP integration** | 14 tools callable inside the agent — let it self-monitor and self-optimize |
| **Proactive push alerts** | MCP server sends JSON-RPC notifications when context > 80%, budget > 90%, ≤5 turns left, or compaction detected — without polling |
| **Webhook escalation** | HTTP POST to Slack/Discord/custom on any alert type — configurable per event |
| **Adaptive thresholds** | Your own P10/P90 percentile thresholds computed from 90 days of history — not hard-coded |
| **Waste analysis** | Severity-weighted signals (Critical/Warning/Info) + actionable suggestions from cross-session intelligence |

## 🏷️ Organization

| Capability | Detail |
|---|---|
| **Session tagging** | `scopeon tag set <id> feature research` — attribute costs to business categories |
| **Auto git branch tags** | `feat/` → `[feature]`, `fix/` → `[bugfix]` displayed in Sessions tab automatically |
| **Cost by tag** | `get_cost_by_tag` MCP tool — "how much did the authentication feature cost in AI?" |
| **Multi-agent tree** | Visualize parent ↔ sub-agent cost hierarchies with per-node totals |

## ⌨️ Shell & Git

| Capability | Detail |
|---|---|
| **Shell prompt integration** | `scopeon shell-hook` injects `$SCOPEON_STATUS` into every shell prompt — health score, context fill, daily cost |
| **Git commit trailer** | `scopeon git-hook install` appends an `AI-Cost:` line to every commit message — visible in `git log` forever |
| **Interactive onboarding** | `scopeon onboard` auto-detects installed AI tools and configures MCP + shell integration in one wizard |
| **Health diagnostics** | `scopeon doctor` prints memory usage, DB stats, provider availability, and overhead proof |
| **Shields.io badges** | `scopeon badge` generates live daily-cost and cache-rate badge URLs for your project README |
| **Weekly digest** | `scopeon digest` produces a Markdown report ready to share or post to Slack/Discord |

## 🔭 Advanced TUI

| Capability | Detail |
|---|---|
| **Temporal Replay** | Press `→`/`←` in session detail to scrub through every turn — see context fill, cost, and tokens at any point in history |
| **Zen Mode** | Press `z` to collapse the entire TUI to a single ambient line: `health · context% · daily cost · cache%`. Auto-exits at context ≥ 80% |
| **Natural-language filter** | Press `/` in Sessions — supports `cost>5`, `cache<20`, `tag:feature`, `model:sonnet`, `today`, `anomaly` predicates |
| **Narrative insights header** | Status bar rotates natural-language sentences instead of raw KPI chips |
| **Anomaly cards** | Insights tab renders severity-bordered cards with actionable titles |
| **End-of-day projection** | Budget tab shows `EOD: $X.XX of $Y.YY · at $Z.ZZZ/hr` — warns at 90% pace |
| **Overhead transparency** | Status bar appends `[◈ X.XMB]` showing Scopeon's own RSS memory footprint |

## 🌐 Team & Integration

| Capability | Detail |
|---|---|
| **Browser dashboard** | `scopeon serve` → `http://localhost:7771` — live WebSocket charts, zero npm |
| **CI cost gate** | `scopeon ci report --fail-on-cost-delta 50` — fail PRs when AI cost spikes |
| **Privacy-filtered HTTP API** | Four tiers: health-only to full metrics |
| **Export** | JSON, CSV, or OTLP JSON for external analysis and data pipelines |
| **OpenTelemetry export** | Prometheus bridge (zero code), OTLP/HTTP push, or `scopeon export --format otlp-json` — feeds Grafana, Datadog, Honeycomb, any OTel backend. [→ guide](opentelemetry.md) |
| **Reprice** | Recalculate all historical costs after a provider price change in seconds |

## ⚙️ Engineering

| Capability | Detail |
|---|---|
| **Pre-computed metrics** | Background snapshot serves most MCP reads with zero DB queries once warm |
| **Read-connection pool** | WAL-mode SQLite + dedicated read-only snapshot path to reduce writer contention |
| **Multi-provider support** | Claude Code · GitHub Copilot CLI · Aider · Cursor · Gemini CLI · Ollama · Generic OpenAI |
| **Local-first, no cloud backend** | No accounts or hosted service — data stays on your machine |
