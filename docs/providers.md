# Supported Providers

Scopeon discovers log files automatically — no configuration needed for standard install paths.
Run `scopeon optimize scan` to inspect which providers support Scopeon-managed optimization presets.

| Provider | Log location | Optimization | Notes |
|---|---|---|---|
| **OpenAI Codex CLI** | `~/.codex/sessions/YYYY/MM/DD/*.jsonl` | Config + launcher | Full token breakdown per turn; prefers `CODEX_HOME`, falls back to legacy `CODEX_CONFIG_DIR` |
| **Claude Code** | `~/.claude/projects/**/*.jsonl` | Launcher presets | Full token breakdown; MCP identity exact, task history estimated |
| **GitHub Copilot CLI** | `~/.copilot/session-state/` | Launcher presets | Rich provenance: tasks, subagents, skills, hooks, MCP/tool lifecycle, model changes; config root honors `COPILOT_HOME` |
| **Aider** | `~/.aider/analytics.jsonl` | Observe only | Analytics log; override with `AIDER_ANALYTICS_LOG` |
| **Cursor** | Cursor app + `Cursor/User/globalStorage` | Observe only | Detection only for now; no token telemetry yet |
| **Gemini CLI** | `~/.gemini/tmp/*/session-*.jsonl` | Config + launcher | Reads Gemini CLI tmp session files and can generate settings override presets |
| **Ollama** | Local API polling | Observe only | Free — no cost tracking |
| **Generic OpenAI** | Configurable paths | Observe only | Set `generic_paths` in `~/.scopeon/config.toml` |

## Adding a new provider

It usually takes one provider file plus a registration change. See [contributing.md](contributing.md#adding-a-new-provider) for the full walkthrough.

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
