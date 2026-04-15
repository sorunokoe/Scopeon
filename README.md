<div align="center">

<h1>
  <br/>Scopeon
</h1>

<pre align="center">
◈  ╔═╗╔═╗╔═╗╔═╗╔═╗╔═╗╔╗╔
   ╚═╗║  ║ ║╠═╝║╣ ║ ║║║║
   ╚═╝╚═╝╚═╝╩  ╚═╝╚═╝╝╚╝
   AI Context Observability
   for Claude Code & friends
</pre>

**The AI context observatory — for every coding agent, every token, every dollar.**

[![CI](https://github.com/sorunokoe/Scopeon/actions/workflows/ci.yml/badge.svg)](https://github.com/sorunokoe/Scopeon/actions/workflows/ci.yml)
[![GitHub release](https://img.shields.io/github/v/release/sorunokoe/Scopeon?label=release)](https://github.com/sorunokoe/Scopeon/releases)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](LICENSE-MIT)
[![MSRV: 1.86](https://img.shields.io/badge/MSRV-1.86-orange)](https://blog.rust-lang.org/2025/02/20/Rust-1.86.0.html)

[**Install**](#installation) · [**Quick Start**](#quick-start) · [**Docs**](docs/) · [**Contributing**](#contributing)

</div>

---

*You fire up Claude Code and start building. An hour later: **"Context window full."** You have no idea what burned it — was it the MCP tools? The thinking budget? Yesterday's file edits? You're flying blind on a meter that costs real money.*

**Scopeon gives you the instrument panel.**

```
╔══════════════════════════════════ Scopeon v0.6.0 ═══════════════════════════════════════╗
║  Health 91/100  ▪  Cost today $2.41  ▪  Cache 68.3%  ▪  Ctx 73% ████████░░ ~12t [!]   ║
╠════════════╦════════════╦═══════════╦═══════════╦═════════════╦════════════════════════╣
║ Dashboard  ║  Sessions  ║ Insights  ║  Budget   ║  Providers  ║        Agents          ║
╠════════════╩════════════╩═══════════╩═══════════╩═════════════╩════════════════════════╣
║                                                                                          ║
║  TURN BREAKDOWN — last 5 turns                                                           ║
║  ┌──────┬────────────┬───────────┬───────────┬──────────┬─────────┬──────────┐          ║
║  │ Turn │ Input      │ Cache↓    │ Cache↑    │ Thinking │ Output  │ Cost     │          ║
║  ├──────┼────────────┼───────────┼───────────┼──────────┼─────────┼──────────┤          ║
║  │  142 │  48,291    │  112,443  │      0    │  12,800  │  2,847  │ $0.182   │          ║
║  │  141 │  46,102    │  115,201  │      0    │   8,192  │  1,923  │ $0.154   │          ║
║  │  140 │  44,887    │  110,988  │  24,601   │  16,384  │  3,211  │ $0.231   │          ║
║  └──────┴────────────┴───────────┴───────────┴──────────┴─────────┴──────────┘          ║
║                                                                                          ║
║  PROMPT CACHE ████████████████████████░░░░░░  68.3% hit rate  saved $1.82 today        ║
║  CONTEXT      █████████████████████████████████████░░░░░░  73.1% used  ~12 turns left  ║
╚══════════════════════════════════════════════════════════════════════════════════════════╝
```

---

## Why Scopeon?

| Without Scopeon | With Scopeon |
|---|---|
| "Why is this session so expensive?" | Turn-by-turn cost breakdown with waste signals |
| "Is the prompt cache actually working?" | Hit-rate gauge, USD saved, optimization suggestions |
| "How close am I to the context limit?" | Real-time fill bar + *"~12 turns remaining"* prediction |
| "Which project costs the most?" | Per-project / per-branch cost breakdown |
| "Did my optimization actually help?" | `compare_sessions` before/after diff |
| "Can I gate AI cost in CI?" | `scopeon ci report --fail-on-cost-delta 50` |
| "How is my whole team using AI?" | `scopeon serve` — privacy-filtered team API |

---

## What it does

- **Token breakdown per turn** — input · cache reads · cache writes · thinking · output · MCP calls
- **Prompt cache intelligence** — hit-rate gauge, USD saved vs. uncached
- **Predictive context countdown** — *"~12 turns remaining"* before hitting the context wall
- **14 MCP tools** — agent self-monitoring + proactive push alerts without polling
- **CI cost gate** — `scopeon ci report --fail-on-cost-delta 50` fails PRs on AI cost spikes
- **Browser dashboard** — `scopeon serve` → live WebSocket charts at `http://localhost:7771`, zero npm
- **Shell & git integration** — cost in your prompt, `AI-Cost:` trailer in every commit
- **Zen Mode · Temporal Replay · Natural-language session filter**
- **Local-first** — no cloud backend, no accounts, data stays on your machine

→ **[Full feature list](docs/features.md)**

---

## Installation

### Fastest: `cargo binstall` (pre-built binary)

```bash
cargo install cargo-binstall   # one-time
cargo binstall scopeon
```

### curl one-liner (macOS & Linux)

```bash
curl -fsSL https://raw.githubusercontent.com/sorunokoe/Scopeon/main/install.sh | sh
```

### From source

```bash
cargo install scopeon
```

**Requirements:** Rust 1.86+ · macOS 12+ or Linux (glibc 2.31+) · Windows 10+

### Pre-built binaries

Download from [GitHub Releases](https://github.com/sorunokoe/Scopeon/releases):

| Platform | Asset |
|---|---|
| macOS Apple Silicon | `scopeon-vX.Y.Z-aarch64-apple-darwin.tar.gz` |
| macOS Intel | `scopeon-vX.Y.Z-x86_64-apple-darwin.tar.gz` |
| Linux x86\_64 | `scopeon-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz` |
| Linux ARM64 | `scopeon-vX.Y.Z-aarch64-unknown-linux-gnu.tar.gz` |
| Windows x86\_64 | `scopeon-vX.Y.Z-x86_64-pc-windows-msvc.zip` |

---

## Quick Start

```bash
scopeon onboard    # auto-detect AI tools, configure MCP + shell integration
scopeon            # open the TUI dashboard
scopeon serve      # browser dashboard → http://localhost:7771
scopeon status     # quick inline stats, no TUI
scopeon doctor     # health diagnostics
```

### Connect to Claude Code (MCP)

```bash
scopeon init
# → writes MCP server config to ~/.claude/settings.json
# Claude Code now has 14 Scopeon tools + proactive push alerts
```

### All commands

```bash
scopeon [start]                # daemon + TUI (default)
scopeon tui                    # TUI only (no file watching)
scopeon mcp                    # MCP server over stdio
scopeon status                 # inline stats
scopeon serve [--port N] [--tier 0-3]

scopeon tag set <id> feature   # tag sessions for cost attribution
scopeon export --format csv --days 30
scopeon reprice                # recalculate costs after a price change

scopeon digest [--days N] [--post-to-slack <url>]
scopeon badge [--format markdown|url|html]
scopeon ci snapshot --output baseline.json
scopeon ci report  --baseline baseline.json [--fail-on-cost-delta 50]

scopeon shell-hook             # emit shell prompt hook (bash/zsh/fish)
scopeon git-hook install       # add AI-Cost trailer to commits
scopeon onboard                # interactive setup wizard
scopeon doctor                 # health diagnostics
```

---

## Documentation

| Topic | Link |
|---|---|
| Full feature list | [docs/features.md](docs/features.md) |
| TUI guide (tabs, shortcuts, Zen, Replay, filter) | [docs/tui.md](docs/tui.md) |
| MCP tools & push notifications | [docs/mcp.md](docs/mcp.md) |
| Webhook escalation | [docs/webhooks.md](docs/webhooks.md) |
| CI cost gate | [docs/ci.md](docs/ci.md) |
| Shell & git integration | [docs/shell-git.md](docs/shell-git.md) |
| Team mode & REST API | [docs/team.md](docs/team.md) |
| Supported providers | [docs/providers.md](docs/providers.md) |
| Configuration reference | [docs/configuration.md](docs/configuration.md) |
| Architecture & codebase map | [ARCHITECTURE.md](ARCHITECTURE.md) |

---

## Supported Providers

Claude Code · GitHub Copilot CLI · Aider · Cursor · Gemini CLI · Ollama · Generic OpenAI

Scopeon discovers log files automatically — no config needed for standard install paths.
Adding a new provider takes ~50 lines of Rust. See [docs/providers.md](docs/providers.md).

---

## Data & Privacy

- **Local-first** — no cloud backend, no accounts, no API keys required
- Reads token *counts* and *costs* only — never prompt text or code
- `scopeon serve` is read-only and localhost-bound by default
- Webhooks are opt-in; payloads contain only metric data

---

## Contributing

Contributions are warmly welcomed — bug fixes, new providers, dashboard features.

```bash
git clone https://github.com/sorunokoe/Scopeon
cd Scopeon
make          # fmt-check + clippy + test (same as CI)
make install  # install to ~/.cargo/bin
```

- **[CONTRIBUTING.md](CONTRIBUTING.md)** — dev workflow, PR process, adding providers
- **[ARCHITECTURE.md](ARCHITECTURE.md)** — codebase map: crate roles, data flow, schema

Every PR must pass the full CI suite (fmt · clippy · tests on Linux/macOS/Windows · MSRV · docs).

---

## Changelog

See **[CHANGELOG.md](CHANGELOG.md)** for the full version history.

---

## License

Dual-licensed under **[MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE)** — use it however you like.

---

<div align="center">

Built with ❤️ in Rust · [Report a bug](https://github.com/sorunokoe/Scopeon/issues/new?template=bug_report.md) · [Request a feature](https://github.com/sorunokoe/Scopeon/issues/new?template=feature_request.md) · [Discussions](https://github.com/sorunokoe/Scopeon/discussions)

*If Scopeon saved you money or context headaches, consider giving it a ⭐*

</div>
