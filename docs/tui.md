# TUI Dashboard

Seven keyboard-navigable tabs — press a number to jump directly.

| Tab | Key | What you see |
|---|---|---|
| **Dashboard** | `1` | Narrative health header · active session · token breakdown · cache gauge · context bar |
| **Sessions** | `2` | Session list with git-branch tags · filter predicates · per-turn drill-down · temporal replay |
| **Config** | `3` | AI provider optimization presets · interactive configuration · detailed preset explanations |
| **Insights** | `4` | Severity-bordered anomaly cards · cross-session suggestions · trend chart |
| **Budget** | `5` | Daily/weekly/monthly spend vs. limits · EOD projection · adaptive threshold bands |
| **Providers** | `6` | Status · session count · last-seen per provider |
| **Agents** | `7` | Multi-agent tree with depth indentation · per-agent cost and token totals |

## Keyboard shortcuts

| Key | Action |
|---|---|
| `1` – `7` / `Tab` | Switch tab |
| `↑` / `k` | Scroll up |
| `↓` / `j` | Scroll down |
| `Enter` | Open session detail / fullscreen (Sessions) · Open preset selector (Config) |
| `→` / `l` | Replay: step forward one turn |
| `←` / `h` | Replay: step backward (first `←` exits replay) |
| `/` | Open session filter bar |
| `z` / `Z` | Toggle Zen mode |
| `r` | Force refresh |
| `?` | Help overlay |
| `q` | Quit |

## Config Tab — AI Provider Optimization

The Config tab provides an interactive interface for managing AI provider optimization presets. 

**What you see:**
- Left panel: List of detected AI providers (Claude Code, Copilot CLI, Codex, Gemini CLI)
- Provider detection status (✓ detected / ✗ not installed)
- Current optimization preset for each provider
- Right panel: Instructions and preset selector modal

**How to use:**
1. Press `3` or Tab to navigate to Config tab
2. Use `↑`/`↓` to select a provider
3. Press `Enter` to open the preset selector (only works for detected providers)
4. Browse presets with `↑`/`↓`:
   - **Most Savings** — Minimize cost (aggressive caching, cheaper models)
   - **Balanced** — Good cost/performance mix (recommended default)
   - **Most Speed** — Fastest responses (reduce latency)
   - **Most Power** — Best quality (premium models, full features)
5. Press `Enter` to apply the selected preset
6. Press `Esc` to cancel

**Preset details shown:**
- Summary — What this preset does
- Trade-off — Cost vs performance implications
- Command preview — Actual CLI flags (intelligently wrapped)
- Optimizations list — Bulleted explanation of all changes

**What happens when you apply:**
- Launcher scripts created in `~/.scopeon/launchers/`
- Config modifications applied (for supported providers)
- Toast notification confirms success
- Provider list updates to show new preset

For more control, use the CLI:
```bash
scopeon optimize scan                           # see all providers
scopeon optimize explain --provider claude-code # detailed explanations
scopeon optimize preview --provider <id> --preset <mode>
scopeon optimize apply --provider <id> --preset <mode>
```

## Session filter predicates

Press `/` in the Sessions tab to filter with plain text **or** structured predicates:

```
cost>5          sessions that cost more than $5
cache<20        sessions with cache hit rate below 20%
tag:feature     sessions tagged "feature"
model:sonnet    sessions using a model containing "sonnet"
today           sessions active today
anomaly         sessions flagged by the waste-analysis engine
```

Predicates can be combined with text search — unknown tokens fall through to substring match.
Invalid values (e.g. `cost>abc`) display an amber parse-error hint in the filter bar.

## Zen Mode

Press `z` anywhere in the TUI to collapse the dashboard to a single ambient line:

```
              ⬡87  ·  Ctx 73% ████████░░  ·  $2.41  ·  Cache 68%
```

Zen auto-exits when context ≥ 80% or daily budget ≥ 90% — urgency always wins.
It auto-restores after 3 consecutive refresh cycles below the threshold.

## Temporal Replay

In the Sessions tab, open a session with `Enter` then press `→` to scrub through its turns one by one:

```
┌─ Replay — turn 140 of 142 ─────────────────────────────────────────┐
│  Context     73.1% ████████████████████████████████████░░░          │
│  Input       44,887 tok   Cache↓  110,988 tok   Cache↑  24,601 tok  │
│  Thinking    16,384 tok   Output    3,211 tok                        │
│  Turn cost   $0.231        Cumulative  $28.42                        │
│                                           ← step back  → step fwd   │
└─────────────────────────────────────────────────────────────────────┘
```

Press `←` to step backward; the first `←` from turn 0 exits replay and returns to normal view.

## Adaptive refresh

The TUI dynamically adjusts its refresh rate:

| Context fill | Refresh interval | State |
|---|---|---|
| < 50% | Every 2 s | Idle |
| 50 – 80% | Every 500 ms | Active |
| > 80% | Every 100 ms | Crisis |

## Browser dashboard

`scopeon serve` starts a local HTTP server with a **live WebSocket browser dashboard**.

```bash
scopeon serve             # port 7771, tier 1 (aggregate stats)
scopeon serve --tier 3    # full metrics including per-turn detail
```

Open **http://localhost:7771** in any browser. The dashboard shows:

- 🟢 **Context pressure** — fill bar + predicted turns remaining
- 💰 **Daily cost** — spend vs. budget gauge
- ⚡ **Cache efficiency** — hit rate and USD saved
- 📊 **Token usage** — input, output, cache, thinking breakdown
- 📅 **30-day cost history** — bar chart with hover detail
- 🔴 **Live alerts** — push banners for context crisis, budget warnings

Updates every 2 seconds via WebSocket — no page refresh needed.
Zero npm, zero CDN, zero build step.

### Privacy tiers

| Tier | Flag | Data exposed |
|---|---|---|
| **0** | `--tier 0` | `/health` only — just "is it running?" |
| **1** | `--tier 1` | Aggregate totals: cost, cache rate, turn count **(default)** |
| **2** | `--tier 2` | Per-session metadata (no prompt content) |
| **3** | `--tier 3` | Full metrics including per-turn breakdown |
