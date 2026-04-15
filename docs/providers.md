# Supported Providers

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

## Adding a new provider

It takes ~50 lines of Rust and one PR. See [CONTRIBUTING.md](../CONTRIBUTING.md#adding-a-new-provider) for the full walkthrough.

In short:

1. Create `crates/scopeon-collector/src/providers/myprovider.rs`
2. Implement the `Provider` trait (two methods: `name()` and `log_paths()`)
3. Implement `parse_turn()` to map a JSONL line to a `Turn` struct
4. Register it in `src/main.rs` → `build_providers()`
5. Add a test in `crates/scopeon-collector/tests/integration.rs`
