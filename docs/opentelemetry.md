# OpenTelemetry Integration

Scopeon exposes its AI cost and token metrics in **OpenTelemetry-compatible format**
without embedding any OTel SDK — keeping the binary small and local-first.

Three integration paths are available, ordered by complexity:

| Path | How | Scopeon code changes |
|---|---|---|
| [Prometheus Bridge](#1-prometheus-bridge-zero-config) | OTel Collector scrapes `/metrics` | Zero |
| [OTLP Push](#2-otlp-push-metrics-no-new-deps) | Scopeon POSTs OTLP/HTTP JSON | ~5 KB |
| [Trace Export](#3-trace-export-sessions-as-distributed-traces) | `scopeon export --format otlp-json` | ~12 KB |

---

## 1. Prometheus Bridge (Zero Config)

> **Ideal Final Result**: The Prometheus `/metrics` endpoint already present in
> `scopeon serve` is simultaneously an OTel-compatible metrics interface.
> Any OTel Collector with a Prometheus receiver bridges it to any backend in 8 lines of YAML.

Start the HTTP server:

```bash
scopeon serve          # starts on http://localhost:7771
```

Configure your OTel Collector to scrape it:

```yaml
# otelcol-config.yaml
receivers:
  prometheus:
    config:
      scrape_configs:
        - job_name: scopeon_ai_metrics
          scrape_interval: 30s
          static_configs:
            - targets: ['localhost:7771']

exporters:
  otlphttp:
    endpoint: https://api.honeycomb.io        # or Grafana Cloud, Datadog, etc.
    headers:
      x-honeycomb-team: ${HONEYCOMB_API_KEY}  # backend-specific auth

service:
  pipelines:
    metrics:
      receivers: [prometheus]
      exporters: [otlphttp]
```

```bash
otelcol --config otelcol-config.yaml
```

Generate a starter config automatically:

```bash
scopeon init --otel-collector            # prints otelcol-config.yaml to stdout
scopeon init --otel-collector > otelcol-config.yaml
```

### Metrics exposed

| Metric | Type | Description |
|---|---|---|
| `scopeon_context_fill_pct` | Gauge | Context window fill % (0–100) |
| `scopeon_cost_usd_today` | Gauge | Estimated AI cost today in USD |
| `scopeon_cost_usd_week` | Gauge | Estimated AI cost this week in USD |
| `scopeon_cache_hit_rate` | Gauge | Prompt cache hit ratio (0.0–1.0) |
| `scopeon_budget_daily_used_pct` | Gauge | Daily budget consumed % |
| `scopeon_total_sessions` | Counter | Total AI sessions recorded |
| `scopeon_total_turns` | Counter | Total turns recorded |
| `scopeon_total_cost_usd` | Counter | Lifetime total cost in USD |
| `scopeon_cache_savings_usd` | Counter | Lifetime prompt-cache savings in USD |

Labels available on each metric: `model="claude-sonnet-4-5"`, `project="my-app"`.

### Backend quick-start snippets

**Grafana Cloud**
```yaml
exporters:
  otlphttp:
    endpoint: https://<instance>.grafana.net/otlp
    headers:
      authorization: Basic ${GRAFANA_TOKEN}
```

**Datadog**
```yaml
exporters:
  datadog:
    api:
      key: ${DD_API_KEY}
      site: datadoghq.com
```

**Honeycomb**
```yaml
exporters:
  otlphttp:
    endpoint: https://api.honeycomb.io
    headers:
      x-honeycomb-team: ${HONEYCOMB_API_KEY}
      x-honeycomb-dataset: scopeon
```

**Self-hosted Grafana + Prometheus** (no Collector needed — Prometheus scrapes directly):
```yaml
# prometheus.yml
scrape_configs:
  - job_name: scopeon
    scrape_interval: 30s
    static_configs:
      - targets: ['localhost:7771']
```

---

## 2. OTLP Push Metrics (No New Deps)

> Scopeon POSTs OTLP/HTTP JSON to your collector endpoint every N seconds.
> Uses `serde_json` (already present) and system `curl` (already used for webhooks).
> Zero new Cargo dependencies. Zero idle overhead when unconfigured.

Add to `~/.scopeon/config.toml`:

```toml
[telemetry]
# OTLP/HTTP receiver endpoint (leave unset to disable — zero overhead)
otlp_endpoint = "http://localhost:4318"

# Push interval in seconds (default: 30)
otlp_interval_secs = 30

# Optional headers (e.g. auth tokens)
[telemetry.otlp_headers]
"x-honeycomb-team" = "your-api-key"
"x-honeycomb-dataset" = "scopeon"
```

The push exporter maps Scopeon's `MetricSnapshot` to OTel AI Semantic Conventions:

```
gen_ai.usage.input_tokens       ← aggregated input tokens (active session)
gen_ai.usage.output_tokens      ← aggregated output tokens
gen_ai.usage.cache_read_tokens  ← Scopeon extension (cache reads)
gen_ai.usage.cache_write_tokens ← Scopeon extension (cache writes)
gen_ai.system                   ← "claude" | "copilot" | "aider" | ...
gen_ai.request.model            ← model string (e.g. "claude-sonnet-4-5")

ai.context.fill_pct             ← context window fill percentage
ai.cost.usd_today               ← today's estimated cost in USD
ai.cache.hit_rate               ← prompt cache hit ratio
ai.context.turns_remaining      ← predicted turns before context limit
```

Resource attributes attached to every export:

```
service.name     = "scopeon"
service.version  = "<version>"
scopeon.project  = "<project slug>"
git.branch       = "<current branch>"
host.name        = "<hostname>"
```

A pull endpoint is also available for collector-initiated scrapes:

```
GET http://localhost:7771/otlp/v1/metrics
Content-Type: application/json
```

---

## 3. Trace Export: Sessions as Distributed Traces

> Map every AI session → OTel root span, every turn → child span.
> Tokens, cost, and cache data become span attributes.
> Works offline (no `scopeon serve` required) — ideal for CI pipelines.

### Offline / CI export

```bash
# Export all sessions as OTLP trace JSON
scopeon export --format otlp-json > ai-traces.json

# Export last 7 days and push to Honeycomb
scopeon export --format otlp-json --since 7d | \
  curl -s -X POST https://api.honeycomb.io/v1/traces \
       -H "Content-Type: application/json" \
       -H "X-Honeycomb-Team: $HONEYCOMB_API_KEY" \
       --data-binary @-

# Use alongside the CI cost gate
scopeon ci snapshot --out baseline.json
# ... run AI-assisted work ...
scopeon ci report --baseline baseline.json --fail-on-cost-delta 50
scopeon export --format otlp-json | \
  curl -s -X POST https://api.honeycomb.io/v1/traces --data-binary @-
```

### Span data model

```
Session (root span)
  trace_id    = deterministic hash of session.id
  name        = "ai.session"
  attributes:
    ai.session.id        = session.id
    gen_ai.system        = "claude" | "copilot" | ...
    gen_ai.request.model = model string
    git.branch           = session.git_branch
    ai.total_cost_usd    = session total cost
    ai.total_turns       = turn count
    ai.is_subagent       = true | false

  Turn (child span, one per turn)
    name        = "ai.turn"
    duration_ns = turn.duration_ms × 1_000_000
    attributes:
      gen_ai.usage.input_tokens        = turn.input_tokens
      gen_ai.usage.output_tokens       = turn.output_tokens
      ai.usage.cache_read_tokens       = turn.cache_read_tokens
      ai.usage.cache_write_tokens      = turn.cache_write_tokens
      ai.usage.thinking_tokens         = turn.thinking_tokens
      ai.cost_usd                      = turn.estimated_cost_usd
      ai.mcp_calls                     = turn.mcp_call_count
      ai.is_compaction_event           = true | false
```

---

## Data model overview

```
┌─────────────────────────────────────────────────────────────┐
│ Scopeon                                                      │
│                                                             │
│  FileWatcher → SQLite → MetricSnapshot                      │
│                              │                              │
│              ┌───────────────┼───────────────┐              │
│              ▼               ▼               ▼              │
│         /metrics         /otlp/v1        export             │
│       (Prometheus)      (pull JSON)    (--format            │
│              │               │          otlp-json)          │
└──────────────┼───────────────┼───────────────┼──────────────┘
               │               │               │
               ▼               ▼               ▼
         OTel Collector    OTel Collector   curl | CI pipeline
               │               │               │
               └───────────────┴───────────────┘
                                │
                                ▼
               Grafana · Datadog · Honeycomb · New Relic · Jaeger
```

---

## Choosing a path

| Scenario | Recommended path |
|---|---|
| Already running OTel Collector | [Prometheus Bridge](#1-prometheus-bridge-zero-config) — 8 lines of YAML |
| Want push without a Collector | [OTLP Push](#2-otlp-push-metrics-no-new-deps) — `[telemetry]` config block |
| CI / batch historical export | [Trace Export](#3-trace-export-sessions-as-distributed-traces) — `scopeon export --format otlp-json` |
| Just want Grafana dashboards | Prometheus scrape directly — no Collector needed |
| Full distributed tracing | Trace Export + continuous push (`otlp_endpoint` config) |

---

## OTel Semantic Convention alignment

Scopeon follows the [OpenTelemetry Generative AI Semantic Conventions](https://opentelemetry.io/docs/specs/semconv/gen-ai/)
(`gen_ai.*`) where standardized. Fields not yet in the spec use the `ai.*` namespace
(Scopeon extension) and will be migrated when the conventions stabilize.

| Attribute | Namespace | Status |
|---|---|---|
| `gen_ai.system` | `gen_ai` | Stable |
| `gen_ai.request.model` | `gen_ai` | Stable |
| `gen_ai.usage.input_tokens` | `gen_ai` | Stable |
| `gen_ai.usage.output_tokens` | `gen_ai` | Stable |
| `ai.usage.cache_read_tokens` | `ai` (extension) | Scopeon-specific |
| `ai.usage.cache_write_tokens` | `ai` (extension) | Scopeon-specific |
| `ai.usage.thinking_tokens` | `ai` (extension) | Scopeon-specific |
| `ai.cost_usd` | `ai` (extension) | Scopeon-specific |
| `ai.context.fill_pct` | `ai` (extension) | Scopeon-specific |

→ **[Features overview](features.md)** · **[Configuration reference](configuration.md)** · **[Webhooks](webhooks.md)**
