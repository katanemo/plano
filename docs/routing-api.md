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
      "models": ["anthropic/claude-sonnet-4-20250514", "openai/gpt-4o", "openai/gpt-4o-mini"]
    },
    {
      "name": "general questions",
      "description": "casual conversation and simple queries",
      "models": ["openai/gpt-4o-mini"]
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

  - name: general questions
    description: casual conversation and simple queries
    models:
      - openai/gpt-4o-mini
      - openai/gpt-4o
```

---

## Model Affinity

In agentic loops where the same session makes multiple LLM calls, send an `X-Model-Affinity` header to pin the routing decision. The first request routes normally and caches the result. All subsequent requests with the same affinity ID return the cached model without re-running routing.

```json
POST /v1/chat/completions
X-Model-Affinity: a1b2c3d4-5678-...

{
  "model": "openai/gpt-4o-mini",
  "messages": [...]
}
```

The routing decision endpoint also supports model affinity:

```json
POST /routing/v1/chat/completions
X-Model-Affinity: a1b2c3d4-5678-...
```

Response when pinned:
```json
{
  "models": ["anthropic/claude-sonnet-4-20250514"],
  "route": "code generation",
  "trace_id": "...",
  "session_id": "a1b2c3d4-5678-...",
  "pinned": true
}
```

Without the header, routing runs fresh every time (no breaking change).

Configure TTL and cache size:
```yaml
routing:
  session_ttl_seconds: 600    # default: 10 min
  session_max_entries: 10000  # upper limit
```

---

## Version Requirements

| Version | Top-level `routing_preferences` |
|---|---|
| `< v0.4.0` | Not allowed — startup error if present |
| `v0.4.0+` | Supported (required for model routing) |
