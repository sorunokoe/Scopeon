# Team Mode

Scopeon has two complementary team features:

- **`scopeon serve`** — privacy-filtered HTTP API each developer runs locally; teammates poll it for aggregate metrics.
- **`scopeon team`** — reads `AI-Cost:` git trailers and prints a per-author cost breakdown straight from your repo history.

## `scopeon team` — Git-Native Cost Ledger

Aggregate AI spend from your commit history without any server or cloud service.

```bash
scopeon team              # last 30 days (default)
scopeon team --days 7     # last week
scopeon team --days 90    # last quarter
```

**Prerequisites:**
1. Each developer has run `scopeon git-hook install` in the repository.
2. Since then, every commit has an `AI-Cost:` trailer like `AI-Cost: $0.42 (8 turns, 42k tokens, 68% cache)`.

**Sample output:**

```
## AI Cost by Author — last 30 days

| Author            | Commits | AI Commits | Total Cost | Avg / Commit | Tokens |
|-------------------|---------|------------|------------|--------------|--------|
| alice@example.com |      47 |         41 |     $18.34 |        $0.39 |  842k  |
| bob@example.com   |      23 |         19 |      $7.61 |        $0.33 |  311k  |
| carol@example.com |       8 |          6 |      $2.05 |        $0.26 |   91k  |

**Total**: 78 commits, $28.00 AI spend
_AI trailers found on 66/78 commits (85%)._
```

All computation is local to the git repository — no data leaves the machine.

---

## `scopeon serve` — Privacy-Filtered HTTP API

Run `scopeon serve` on each developer's machine for team-wide AI usage visibility — with zero cloud, zero data leaving any machine.

### Starting the server

```bash
scopeon serve                        # localhost:7771, tier 1 (default)
scopeon serve --port 8080 --tier 2   # per-session metadata
scopeon serve --lan --tier 1 --secret team-token   # allow LAN access with auth
scopeon serve --lan --tier 3 --secret <token>  # full metrics, token-protected
```

### REST & streaming endpoints

```
GET /health              → { status, version, uptime_secs, tier }         (all tiers)
GET /api/v1/stats        → token totals, cost totals, cache hit rate       (tier ≥ 1)
GET /api/v1/budget       → daily/weekly/monthly spend vs. limits           (tier ≥ 1)
GET /api/v1/sessions     → session list with cost and cache metadata       (tier ≥ 2)
GET /api/v1/context      → live context pressure for the active session    (tier ≥ 2)
GET /api/v1/interactions → tool/MCP/skill/hook provenance for a session    (tier ≥ 3)
GET /api/v1/tasks        → task/subagent history for a session             (tier ≥ 3)
GET /api/v1/provider-capabilities → exact/estimated provenance matrix      (tier ≥ 3)
WS  /ws/v1/metrics       → real-time WebSocket stream (2 s snapshots)      (tier ≥ 1)
GET /sse/v1/status       → Server-Sent Events status stream for IDEs       (tier ≥ 1)
```

All JSON responses include CORS headers — ready to consume from any browser dashboard or monitoring tool.

### IDE SSE stream

The `GET /sse/v1/status` endpoint delivers a persistent stream of compact status events ideal for IDE extensions and lightweight status bars. Unlike WebSocket, SSE works through any HTTP reverse proxy with no special handling.

```bash
# Try it:
curl -N http://localhost:7771/sse/v1/status
```

Each event is a compact JSON object:

```json
{
  "fill_pct": 42.3,
  "daily_cost_usd": 0.2341,
  "cache_hit_rate_pct": 68.2,
  "predicted_turns_remaining": 18,
  "should_compact": false
}
```

Events fire every ~2 seconds. The connection keeps alive via SSE keep-alive pings. Authentication follows the same `x-scopeon-token` header as other tiered endpoints.

## Privacy tiers

| Tier | Flag | Data exposed |
|---|---|---|
| **0** | `--tier 0` | `/health` only — just "is it running?" |
| **1** | `--tier 1` | Aggregate totals: cost, cache rate, turn count **(default)** |
| **2** | `--tier 2` | Per-session metadata (no prompt content) |
| **3** | `--tier 3` | Full metrics plus interaction/task provenance and provider capability details |

## Security notes

- By default the server binds to `127.0.0.1` (localhost only).
- Use `--lan` only if you intentionally want teammates to access your data.
- `--lan` with `--tier 1` or higher requires `--secret <token>`. Callers must pass `x-scopeon-token: <token>` to every tiered REST and WebSocket endpoint.
- `scopeon serve` is **read-only** — it never writes to the database.
- Webhook payloads contain only metric data (no prompts, no code).
- Prefer HTTPS or a trusted local reverse proxy for sensitive environments.
