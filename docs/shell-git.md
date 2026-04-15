# Shell & Git Integration

## Shell prompt status

Add live AI context stats to your shell prompt — health score, context fill, and daily cost appear on every command line draw.

**bash / zsh** — add to `~/.bashrc` or `~/.zshrc`:

```bash
eval "$(scopeon shell-hook)"
```

Then add `$SCOPEON_STATUS` to your prompt (`RPROMPT` in zsh, `PS1` suffix in bash):

```
⬡87  73%  $2.41
```

**fish** — add to `~/.config/fish/config.fish`:

```fish
scopeon shell-hook --shell fish | source
```

The `⬡N` health score, context fill %, and daily cost update every prompt draw.
Colour coding: fill ≥ 80% → amber, ≥ 95% → red.

## Git commit AI-Cost trailer

Track how much AI work went into each commit — visible forever in `git log`.

```bash
# Install the hook in your repository
scopeon git-hook install

# Remove it
scopeon git-hook uninstall
```

Every commit automatically gets a trailer line:

```
AI-Cost: $0.23 (14 turns, 182k tokens, 68% cache)
```

View in git log:

```bash
git log --format="%h %s%n%b" | grep -A1 "AI-Cost"
# abc1234 feat: implement user authentication
# AI-Cost: $1.84 (42 turns, 891k tokens, 71% cache)
```

## Weekly digest

Generate a Markdown report for sharing with your team or posting to Slack/Discord:

```bash
scopeon digest                                    # 7-day report to stdout
scopeon digest --days 30 > monthly-report.md
scopeon digest --post-to-slack <webhook-url>
scopeon digest --post-to-discord <webhook-url>
```

Example output:

```markdown
## AI Usage Digest — 2026-04-08 to 2026-04-15

### Executive Summary
| Metric | Value |
|---|---|
| Total cost | $18.40 |
| Sessions | 23 |
| Turns | 614 |
| Cache hit rate | 71.2% |
| Est. savings vs. no-cache | $12.30 |

### Top Optimization Recommendations
1. **High thinking ratio on model claude-sonnet** — consider capping thinking budget
2. **Cache bust on 4 sessions** — check system prompt stability
3. **3 sessions without tags** — tag work items for better cost attribution
```

## Shields.io badges

Add live AI usage badges to your project README:

```bash
scopeon badge              # Markdown snippets (default)
scopeon badge --format url # raw shields.io URLs
scopeon badge --format html
```

```markdown
![Daily AI Cost](https://img.shields.io/badge/AI_cost-$2.41%2Fday-blue)
![Cache Hit Rate](https://img.shields.io/badge/cache-68.3%25-brightgreen)
```

## Onboarding wizard

Run `scopeon onboard` once after installing to auto-detect every AI tool on your machine and configure MCP + shell integration interactively.

## Health diagnostics

`scopeon doctor` prints:

- Scopeon's own RSS memory footprint
- SQLite DB stats (size, migration version, row counts)
- Provider log paths and whether sessions are being tracked
- A diagnosis if sessions aren't appearing
