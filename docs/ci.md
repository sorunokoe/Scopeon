# CI Cost Gate

Gate pull requests on AI cost — catch regressions before they reach `main`.

## How it works

1. **On `main`** — save a baseline snapshot after merging.
2. **On each PR** — compare current AI usage against the baseline.
3. **Post as a PR comment** — reviewers see cost and cache changes at a glance.

## Commands

```bash
# Save baseline (run on main)
scopeon ci snapshot --output baseline.json

# Compare in CI
scopeon ci report --baseline baseline.json

# Hard gate: fail if AI cost increased by more than 50%
scopeon ci report --baseline baseline.json --fail-on-cost-delta 50
```

## GitHub Actions workflow

```yaml
# .github/workflows/ai-cost-gate.yml
name: AI Cost Gate

on:
  pull_request:
    branches: [main]

jobs:
  ai-cost-gate:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Scopeon
        run: curl -fsSL https://raw.githubusercontent.com/sorunokoe/Scopeon/main/install.sh | sh

      - name: Download baseline
        run: gh release download --pattern baseline.json --dir .
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}

      - name: Generate cost report
        run: |
          ~/.local/bin/scopeon ci report \
            --baseline baseline.json \
            --fail-on-cost-delta 50 > cost-report.md

      - name: Post PR comment
        run: gh pr comment ${{ github.event.pull_request.number }} --body-file cost-report.md
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

Full example at [`.github/examples/ai-cost-gate.yml`](../.github/examples/ai-cost-gate.yml).

## Example report output

```
## 🔬 AI Cost Analysis

Comparing current (2026-04-13) to baseline (2026-03-01).

| Metric           | Baseline  | Current   | Delta       |
|------------------|-----------|-----------|-------------|
| Total cost       | $41.20    | $38.80    | -5.8%  🟢  |
| Cache hit rate   | 48.3%     | 68.3%     | +20.0% 🟢  |
| Context peak     | 91.2%     | 73.1%     | -18.1% 🟢  |
| Avg tokens/turn  | 12,400    | 8,468     | -31.7% 🟢  |
| Sessions         | 120       | 188       | —           |
| Turns            | 3,100     | 4,521     | —           |
```

## Optimization feedback loop

Use snapshots to measure the impact of any change — system prompt tuning, model switching, cache restructuring:

```bash
scopeon ci snapshot --output before.json

# ... make changes ...

scopeon ci snapshot --output after.json
scopeon ci report --baseline before.json
```
