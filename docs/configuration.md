# Configuration

`~/.scopeon/config.toml` is created automatically on first run with sensible defaults.

## Full reference

```toml
[general]
refresh_interval_secs = 2        # TUI base refresh rate (adaptive override activates at >50% fill)
theme = "standard"               # "standard" | "high-contrast"

[providers]
enabled = [
  "claude-code", "copilot-cli", "aider",
  "cursor", "gemini-cli", "ollama"
]
generic_paths = []               # extra log directories for Generic OpenAI provider
generic_name  = "Custom Agent"

[budget]
daily_usd   = 5.0                # 0.0 = no limit; TUI shows progress bar + alert banner
weekly_usd  = 20.0
monthly_usd = 50.0

[alerts]
daily_cost_usd     = 5.0         # TUI banner when daily cost exceeds this
cache_hit_rate_min = 0.20        # warn when cache hit rate drops below 20%

# Webhook escalation — POST JSON alerts to any HTTP(S) endpoint
[[alerts.webhooks]]
url    = "https://hooks.slack.com/services/T.../B.../..."
events = ["context_crisis", "budget_warning"]  # empty list = all event types

[[alerts.webhooks]]
url    = "https://discord.com/api/webhooks/..."
events = ["context_crisis"]

# Custom model pricing overrides
[pricing]
# override_file = "~/.scopeon/my-pricing.toml"
```

## Environment variables

| Variable | Default | Description |
|---|---|---|
| `SCOPEON_DB` | `~/.scopeon/scopeon.db` | Override the database path |
| `SCOPEON_CONFIG` | `~/.scopeon/config.toml` | Override the config file path |
| `RUST_LOG` | — | Set to `scopeon=debug` for verbose diagnostic logs |

## Resetting the database

```bash
rm ~/.scopeon/scopeon.db
scopeon start   # fresh backfill from all provider logs
```

## Repricing after a provider price change

```bash
scopeon reprice
```

Recalculates `estimated_cost_usd` for every stored turn using the current pricing table.
Run this whenever Anthropic, OpenAI, or Google update their rate cards.
