# Supported Providers

Scopeon discovers log files automatically — no configuration needed for standard install paths.

| Provider | Log location | Notes |
|---|---|---|
| **Claude Code** | `~/.claude/projects/**/*.jsonl` | Full token breakdown; MCP identity exact, task history estimated |
| **GitHub Copilot CLI** | `~/.copilot/session-state/` | Rich provenance: tasks, subagents, skills, hooks, MCP/tool lifecycle, model changes |
| **Aider** | `~/.aider/analytics.jsonl` | Analytics log; override with `AIDER_ANALYTICS_LOG` |
| **Cursor** | Cursor app + `Cursor/User/globalStorage` | Detection only for now; no token telemetry yet |
| **Gemini CLI** | `~/.gemini/tmp/*/session-*.jsonl` | Reads Gemini CLI tmp session files |
| **Ollama** | Local API polling | Free — no cost tracking |
| **Generic OpenAI** | Configurable paths | Set `generic_paths` in `~/.scopeon/config.toml` |

## Adding a new provider

It usually takes one provider file plus a registration change. See [CONTRIBUTING.md](../CONTRIBUTING.md#adding-a-new-provider) for the full walkthrough.

In short:

1. Create `crates/scopeon-collector/src/providers/myprovider.rs`
2. Implement the `Provider` trait (`id`, `name`, `description`, `is_available`, `watch_paths`, `scan`)
3. Register it in `crates/scopeon-collector/src/providers/mod.rs` and `src/main.rs`
4. Add or extend tests in `crates/scopeon-collector/tests/integration.rs`

## Provenance capability levels

Scopeon now publishes provider capabilities for provenance-heavy features:

- **exact** — emitted directly by the provider logs
- **estimated** — derived safely from sizes, timing, or correlations
- **unsupported** — not available from that provider's logs, so Scopeon will not fabricate it

Use the MCP tool `get_provider_capabilities` or the HTTP endpoint `/api/v1/provider-capabilities` to see the current matrix for a session.
