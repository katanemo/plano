# Plano Routing API — Request & Response Format

## Overview

Plano intercepts LLM requests and routes them to the best available model based on semantic intent and live cost/latency data. The developer sends a standard OpenAI-compatible request with an optional `routing_preferences` field. Plano returns an ordered list of candidate models; the client uses the first and falls back to the next on 429 or 5xx errors.

---

## Request Format

Standard OpenAI chat completion body. The only addition is the optional `routing_preferences` field, which is stripped before the request is forwarded upstream.

```json
POST /v1/chat/completions
{
  "model": "openai/gpt-4o-mini",
  "messages": [
    {"role": "user", "content": "write a sorting algorithm in Python"}
  ],
  "routing_preferences": [
    {
      "name": "code generation",
      "description": "generating new code snippets",
      "models": ["anthropic/claude-sonnet-4-20250514", "openai/gpt-4o", "openai/gpt-4o-mini"],
      "selection_policy": {"prefer": "fastest"}
    },
    {
      "name": "general questions",
      "description": "casual conversation and simple queries",
      "models": ["openai/gpt-4o-mini"],
      "selection_policy": {"prefer": "cheapest"}
    }
  ]
}
```

### `routing_preferences` fields

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | Route identifier. Must match the LLM router's route classification. |
| `description` | string | yes | Natural language description used by the router to match user intent. |
| `models` | string[] | yes | Ordered candidate pool. At least one entry required. Must be declared in `model_providers`. |
| `selection_policy.prefer` | enum | yes | How to rank models: `cheapest`, `fastest`, `random`, or `none`. |

### `selection_policy.prefer` values

| Value | Behavior |
|---|---|
| `cheapest` | Sort by ascending cost from the metrics endpoint. Models with no data appended last. |
| `fastest` | Sort by ascending latency from the metrics endpoint. Models with no data appended last. |
| `random` | Shuffle the model list randomly on each request. |
| `none` | Return models in the order they were defined — no reordering. |

### Notes

- `routing_preferences` is **optional**. If omitted, the config-defined preferences are used.
- If provided in the request body, it **overrides** the config for that single request only.
- `model` is still required and is used as the fallback if no route is matched.

---

## Response Format

```json
{
  "models": [
    "anthropic/claude-sonnet-4-20250514",
    "openai/gpt-4o",
    "openai/gpt-4o-mini"
  ],
  "route": "code generation",
  "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736"
}
```

### Fields

| Field | Type | Description |
|---|---|---|
| `models` | string[] | Ranked model list. Use `models[0]` as primary; retry with `models[1]` on 429/5xx, and so on. |
| `route` | string \| null | Name of the matched route. `null` if no route matched — client should use the original request `model`. |
| `trace_id` | string | Trace ID for distributed tracing and observability. |

---

## Client Usage Pattern

```python
response = plano.routing_decision(request)
models = response["models"]

for model in models:
    try:
        result = call_llm(model, messages)
        break  # success — stop trying
    except (RateLimitError, ServerError):
        continue  # try next model in the ranked list
```

---

## Configuration (set by platform/ops team)

Requires `version: v0.4.0` or above. Models listed under `routing_preferences` must be declared in `model_providers`.

```yaml
version: v0.4.0

model_providers:
  - model: anthropic/claude-sonnet-4-20250514
    access_key: $ANTHROPIC_API_KEY
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
  - model: openai/gpt-4o-mini
    access_key: $OPENAI_API_KEY
    default: true

routing_preferences:
  - name: code generation
    description: generating new code snippets or boilerplate
    models:
      - anthropic/claude-sonnet-4-20250514
      - openai/gpt-4o
    selection_policy:
      prefer: fastest

  - name: general questions
    description: casual conversation and simple queries
    models:
      - openai/gpt-4o-mini
      - openai/gpt-4o
    selection_policy:
      prefer: cheapest

# Optional: live cost and latency data sources (max one per type)
model_metrics_sources:
  - type: cost_metrics
    url: https://internal-cost-api/models
    refresh_interval: 300  # seconds; omit for fetch-once on startup
    auth:
      type: bearer
      token: $COST_API_TOKEN

  - type: prometheus_metrics
    url: https://internal-prometheus/
    query: histogram_quantile(0.95, sum by (model_name, le) (rate(model_latency_seconds_bucket[5m])))
    refresh_interval: 60
```

### cost_metrics endpoint

Plano GETs `url` on startup (and on each `refresh_interval`). Expected response — a flat JSON object mapping model name to cost value:

```json
{
  "anthropic/claude-sonnet-4-20250514": 0.003,
  "openai/gpt-4o": 0.005,
  "openai/gpt-4o-mini": 0.00015
}
```

- `auth.type: bearer` adds `Authorization: Bearer <token>` to the request
- Cost units are arbitrary (e.g. USD per 1k tokens) — only relative order matters

### prometheus_metrics endpoint

Plano queries `{url}/api/v1/query?query={query}` on startup and each `refresh_interval`. The PromQL expression must return an instant vector with a `model_name` label:

```json
{
  "status": "success",
  "data": {
    "resultType": "vector",
    "result": [
      {"metric": {"model_name": "anthropic/claude-sonnet-4-20250514"}, "value": [1234567890, "120.5"]},
      {"metric": {"model_name": "openai/gpt-4o"}, "value": [1234567890, "200.3"]}
    ]
  }
}
```

- The PromQL query is responsible for computing the percentile (e.g. `histogram_quantile(0.95, ...)`)
- Latency units are arbitrary — only relative order matters
- Models missing from the result are appended at the end of the ranked list

---

## Version Requirements

| Version | Top-level `routing_preferences` |
|---|---|
| `< v0.4.0` | Not allowed — startup error if present |
| `v0.4.0+` | Supported (required for model routing) |
