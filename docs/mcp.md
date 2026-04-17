# MCP Integration

After running `scopeon init`, Scopeon registers itself as an MCP server in Claude Code.
The agent can then call Scopeon tools directly from its own context —
**without using any token budget for polling**.

## Setup

```bash
scopeon init
# → writes MCP server config to ~/.claude/settings.json
```

## Available tools

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
| `get_interaction_history` | Provenance ledger for tool, MCP, skill, hook, and subagent interactions |
| `get_task_history` | Derived task/subagent history with fan-out, model, and token totals |
| `get_provider_capabilities` | Exact vs. estimated vs. unsupported provenance support for the active provider |
| `suggest_compact` | Boolean: should the agent call `/compact` right now? |
| `get_project_stats` | Cost and cache breakdown by project and branch |
| `list_sessions` | Recent sessions with cost and cache metadata |
| `get_agent_tree` | Multi-agent hierarchy with per-agent cost totals |
| `set_session_tags` | Tag the current session for cost attribution (`feature`, `research`, `debug`) |
| `get_cost_by_tag` | Total cost grouped by tag — "how much did auth work cost in AI?" |

## Proactive push notifications

The MCP server sends JSON-RPC **notifications** (no `id` field = zero token cost,
per JSON-RPC 2.0 §4) to Claude Code's MCP client automatically when:

| Condition | Method | Alert type | Severity |
|---|---|---|---|
| Context ≥ 95% | `notifications/scopeon/alert` | `context_crisis` | Critical |
| Context 80–94% | `notifications/scopeon/alert` | `context_warning` | Warning |
| Daily spend > 90% of limit | `notifications/scopeon/alert` | `budget_warning` | Warning / Critical |
| Predicted turns remaining ≤ 5 | `notifications/scopeon/alert` | `low_turns_left` | Warning |
| Auto-compaction detected | `notifications/scopeon/alert` | `compaction_detected` | Info |
| Fill 55–79% and accelerating | `notifications/scopeon/alert` | `compaction_advisory` | Info |
| Every 30 s (fill < 80%) | `notifications/scopeon/status` | `ambient_status` | — |

Alert kinds have a **60-second cooldown** to prevent spam. Ambient status is periodic and not debounced.

### Ambient status payload

Every 30 seconds the server emits a zero-token status heartbeat:

```json
{
  "method": "notifications/scopeon/status",
  "params": {
    "type": "ambient_status",
    "fill_pct": 42.3,
    "predicted_turns_remaining": 18,
    "daily_cost_usd": 0.23,
    "cache_hit_rate_pct": 68.2,
    "should_compact": false,
    "health_score_proxy": 84.5
  }
}
```

Subscribe to this in your system prompt instead of polling `get_context_pressure` — it is literally free.

### Compaction advisory payload

Fires in the 55–79% fill window when fill is accelerating and cache writes are low:

```json
{
  "method": "notifications/scopeon/alert",
  "params": {
    "type": "compaction_advisory",
    "severity": "info",
    "fill_pct": 67.0,
    "advisory_score": 0.71,
    "predicted_turns_remaining": 14,
    "should_compact": true,
    "message": "Optimal compaction window: context 67% full and accelerating (~14 turns remain). Compact now to maximise cache savings."
  }
}
```

## Provenance-aware tooling

The new provenance tools stay within Scopeon's local-first privacy contract:

- no raw prompt or code bodies are exposed
- interaction/task payloads are represented with names, timestamps, sizes, totals, and confidence
- provider capability results explicitly tell the agent when a detail is **exact**, **estimated**, or **unsupported**

## Letting the agent manage its own context

Add this to your Claude Code system prompt:

```
Scopeon sends notifications/scopeon/status every 30 s — read fill_pct and should_compact
from those instead of calling get_context_pressure each turn.

If a notifications/scopeon/alert with type "compaction_advisory" or "context_warning" arrives:
  consider calling /compact before starting the next tool-heavy task.

After completing a task: call compare_sessions to report token efficiency.
When starting a new feature: call set_session_tags to attribute costs correctly.
```

> **Why this works**: the `notifications/scopeon/status` heartbeat is a free JSON-RPC
> push with no `id` field (§4 of the spec). The agent receives it without any tool call
> or token spend. Only poll `get_context_pressure` when you need immediate, on-demand data.

## compare_sessions example

```json
compare_sessions(
  session_a = "abc123",
  session_b = "def456"
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
