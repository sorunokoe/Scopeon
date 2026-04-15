# Security Policy

## Supported versions

| Version | Supported |
|---------|-----------|
| latest `main` | ✅ |
| 0.6.x | ✅ |
| 0.5.x | Security fixes only |
| < 0.5 | ❌ |

## Reporting a vulnerability

**Please do not open a public GitHub issue for security vulnerabilities.**

Report via **[GitHub private security advisory](https://github.com/sorunokoe/Scopeon/security/advisories/new)** (preferred) or email **security@scopeon.dev** with:

- A description of the vulnerability and its impact
- Steps to reproduce or a proof-of-concept
- Any suggested fix (optional but appreciated)
- Whether you would like to be credited in the release notes

You will receive an acknowledgment within 48 hours and a resolution timeline within 7 days. Critical vulnerabilities are patched and released as patch versions within 72 hours.

## Threat model

Scopeon is a **local-first** observability tool designed to keep AI usage data on your machine by default.

### What Scopeon does

- Reads log files already written to disk by AI agents (token counts, costs, timestamps — never prompt text or code content).
- Writes to a local SQLite database at `~/.scopeon/scopeon.db`.
- Exposes an optional read-only HTTP/WebSocket API when you explicitly run `scopeon serve`.
- Sends JSON-RPC messages over stdio to the parent MCP client process.
- Optionally POSTs alert payloads to user-configured webhook URLs.

### What Scopeon does NOT do

- Send any data to a hosted Scopeon backend.
- Intercept network traffic or modify agent behaviour.
- Read prompt text, code, or session content — only token counts and metadata.
- Execute file content.
- Follow symlinks outside the configured provider directories.

### Network exposure

| Component | Network access |
|---|---|
| TUI (`scopeon start`) | None — reads local files and local DB only |
| MCP server (`scopeon mcp`) | stdio only — no TCP listener |
| HTTP API (`scopeon serve`) | Binds `127.0.0.1` by default; `--lan` binds `0.0.0.0`; read-only |
| WebSocket stream (`/ws/v1/metrics`) | Available only when `scopeon serve` runs at tier 1 or higher |
| Webhooks | Opt-in HTTP(S) POST to user-configured URLs |

`scopeon serve` uses privacy tiers:

- **Tier 0** — health only
- **Tier 1** — aggregate metrics
- **Tier 2+** — per-session metadata and context endpoints

If you pass `--secret`, Scopeon requires `x-scopeon-token` for tier 2+ REST endpoints.

### File system access

Scopeon reads files in the following directories (based on `[providers] enabled` in config):

| Provider | Path read |
|---|---|
| Claude Code | `~/.claude/projects/` |
| GitHub Copilot CLI | `~/.config/github-copilot/` |
| Aider | `~/.aider/logs/` |
| Cursor | `~/.cursor/logs/` |
| Gemini CLI | `~/.gemini/logs/` |
| Generic OpenAI | User-configured `generic_paths` |

The database is created at `~/.scopeon/scopeon.db` with standard user file permissions. No data is written to provider directories.

### Webhook security

If you configure `[[alerts.webhooks]]` in `config.toml`:

- Payload contains only metric data (fill %, cost, turn counts) — no prompts, no code.
- The built-in sender supports **HTTP and HTTPS** using the system `curl` binary.
- Config validation rejects unsupported URL schemes and warns on embedded credentials or cleartext HTTP.
- Prefer HTTPS or a trusted local relay/reverse proxy for sensitive environments.
- Event filtering (`events = [...]`) limits which alerts reach each endpoint.

## Dependency audit

```bash
cargo audit    # checks all dependencies against the RustSec advisory database
```

The CI pipeline runs `cargo audit` on every commit. Any advisory classified as high or critical blocks the CI badge.
