# Changelog

All notable changes to Scopeon will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.10.0] - 2026-05-01

### Added
- **TUI Config Tab**: Interactive AI provider optimization interface
  - Select and apply presets (most-savings, balanced, most-speed, most-power)
  - Detailed preset explanations with trade-offs and optimization details
  - Support for 4 AI providers: Claude Code, Copilot CLI, Codex, Gemini CLI
  - Intelligent command preview with text wrapping
  - On-demand refresh (no background polling)
- **AI Provider Optimization System**: CLI commands for managing AI provider configurations
  - `scopeon optimize scan` - detect installed providers and current presets
  - `scopeon optimize explain --provider <id>` - detailed provider information
  - `scopeon optimize preview --provider <id> --preset <mode>` - preview changes
  - `scopeon optimize apply --provider <id> --preset <mode>` - apply preset
  - Automatic launcher script generation for quick switching
  - Config persistence in `~/.scopeon/config.toml`

### Performance
- **Major TUI Performance Improvements**: 60-80% reduction in refresh overhead
  - Draw-on-change rendering (eliminates unconditional redraws)
  - Per-tab refresh intervals (Config: on-demand, Sessions: 5s, Spend: 10s)
  - Deadline-based event polling (near-zero CPU when idle)
  - Skip loading unused data (agent trees, interaction events, task runs)
  - Smooth scrolling and instant input response

### Fixed
- Config tab preset selections now persist to `~/.scopeon/config.toml`
- Config tab now visible in tab bar with keyboard shortcuts (press `3`)
- Config cache invalidates when entering tab to show fresh data

### Documentation
- Added comprehensive TUI Config tab documentation (`docs/tui.md`)
- Added Config tab screenshot to README
- Updated keyboard shortcuts reference

### Chore
- Force Node.js 24 for GitHub Actions
- Upgrade actions/cache from v4 to v5 for Node.js 24 compatibility
- Fix clippy warnings (collapse nested if in match arm)
- Fix --test-threads syntax in pricing-review workflow

## [0.9.0] - 2026-04-17

Initial tagged release with full TUI, MCP server, and observability features.

[0.10.0]: https://github.com/sorunokoe/Scopeon/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/sorunokoe/Scopeon/releases/tag/v0.9.0
