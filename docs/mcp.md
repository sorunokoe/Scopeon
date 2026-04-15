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
| `suggest_compact` | Boolean: should the agent call `/compact` right now? |
| `get_project_stats` | Cost and cache breakdown by project and branch |
| `list_sessions` | Recent sessions with cost and cache metadata |
| `get_agent_tree` | Multi-agent hierarchy with per-agent cost totals |
| `set_session_tags` | Tag the current session for cost attribution (`feature`, `research`, `debug`) |
| `get_cost_by_tag` | Total cost grouped by tag — "how much did auth work cost in AI?" |

## Proactive push notifications

The MCP server sends JSON-RPC **notifications** (no `id` field = zero token cost,
per JSON-RPC 2.0 §4) to Claude Code's MCP client automatically when:

| Condition | Alert type | Severity |
|---|---|---|
| Context ≥ 95% | `context_crisis` | Critical |
| Context 80–94% | `context_warning` | Warning |
| Daily spend > 90% of limit | `budget_warning` | Warning / Critical |
| Predicted turns remaining ≤ 5 | `low_turns_left` | Warning |
| Auto-compaction detected | `compaction_detected` | Info |

Each alert kind has a **60-second cooldown** to prevent notification spam.

## Letting the agent manage its own context

Add this to your Claude Code system prompt:

```
Before any long task: call get_context_pressure.
If should_compact is true OR fill_pct > 85 OR predicted_turns_remaining < 10: call /compact.
After completing a task: call compare_sessions to report token efficiency.
When starting a new feature: call set_session_tags to attribute costs correctly.
```

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
