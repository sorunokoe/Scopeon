# Team Mode

Run `scopeon serve` on each developer's machine for team-wide AI usage visibility — with zero cloud, zero data leaving any machine.

## Starting the server

```bash
scopeon serve                        # localhost:7771, tier 1 (default)
scopeon serve --port 8080 --tier 2   # per-session metadata
scopeon serve --lan --tier 1         # allow LAN access (0.0.0.0)
scopeon serve --lan --tier 3 --secret <token>  # full metrics, token-protected
```

## REST endpoints

```
GET /health              → { status, version, uptime_secs, tier }         (all tiers)
GET /api/v1/stats        → token totals, cost totals, cache hit rate       (tier ≥ 1)
GET /api/v1/budget       → daily/weekly/monthly spend vs. limits           (tier ≥ 1)
GET /api/v1/sessions     → session list with cost and cache metadata       (tier ≥ 2)
GET /api/v1/context      → live context pressure for the active session    (tier ≥ 2)
WS  /ws/v1/metrics       → real-time WebSocket stream (2 s snapshots)      (tier ≥ 1)
```

All responses are JSON with CORS headers — ready to consume from any browser dashboard or monitoring tool.

## Privacy tiers

| Tier | Flag | Data exposed |
|---|---|---|
| **0** | `--tier 0` | `/health` only — just "is it running?" |
| **1** | `--tier 1` | Aggregate totals: cost, cache rate, turn count **(default)** |
| **2** | `--tier 2` | Per-session metadata (no prompt content) |
| **3** | `--tier 3` | Full metrics including per-turn breakdown |

## Security notes

- By default the server binds to `127.0.0.1` (localhost only).
- Use `--lan` only if you intentionally want teammates to access your data.
- Use `--secret <token>` with `--lan` for tier ≥ 2 endpoints. Callers must pass `x-scopeon-token: <token>`.
- `scopeon serve` is **read-only** — it never writes to the database.
- Webhook payloads contain only metric data (no prompts, no code).
- Prefer HTTPS or a trusted local reverse proxy for sensitive environments.
