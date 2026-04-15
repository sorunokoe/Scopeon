# Webhook Escalation

Scopeon can POST alerts to any HTTP endpoint — Slack, Discord, PagerDuty, custom services.

## Configuration

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

## Alert payload

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

## Alert types

| Type | Trigger | Severity |
|---|---|---|
| `context_crisis` | Context ≥ 95% | Critical |
| `context_warning` | Context 80–94% | Warning |
| `budget_warning` | Daily spend > 90% of limit | Warning / Critical |
| `low_turns_left` | Predicted turns remaining ≤ 5 | Warning |
| `compaction_detected` | Auto-compaction detected | Info |

Webhooks are opt-in and fire asynchronously — they never block the main data pipeline.
Prefer HTTPS or a trusted local relay for sensitive environments.
