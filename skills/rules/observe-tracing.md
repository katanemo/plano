---
title: Enable Tracing with Appropriate Sampling for Your Environment
impact: HIGH
impactDescription: Without tracing enabled, debugging routing decisions, latency issues, and model selection is guesswork — traces are the primary observability primitive in Plano
tags: observability, tracing, opentelemetry, otel, debugging
---

## Enable Tracing with Appropriate Sampling for Your Environment

Plano emits OpenTelemetry (OTEL) traces for every request, capturing routing decisions, LLM provider selection, filter chain execution, and response latency. Traces are the best tool for understanding why a request was routed to a particular model and debugging unexpected behavior.

**Incorrect (no tracing configured — flying blind in production):**

```yaml
version: v0.3.0

listeners:
  - type: model
    name: model_listener
    port: 12000

model_providers:
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    default: true

# No tracing block — no visibility into routing, latency, or errors
```

**Correct (tracing enabled with environment-appropriate sampling):**

```yaml
version: v0.3.0

listeners:
  - type: model
    name: model_listener
    port: 12000

model_providers:
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    default: true

tracing:
  random_sampling: 100              # 100% for development/debugging
  trace_arch_internal: true         # Include Plano's internal routing spans
```

**Production configuration (sampled to control volume):**

```yaml
tracing:
  random_sampling: 10               # Sample 10% of requests in production
  trace_arch_internal: false        # Skip internal spans to reduce noise
  span_attributes:
    header_prefixes:
      - x-katanemo-               # Match all x-katanemo-* headers
    static:
      environment: production
      service.name: my-plano-service
      version: "1.0.0"
```

With `x-katanemo-` configured, Plano maps headers to attributes by stripping the prefix and converting hyphens to dots:

- `x-katanemo-user-id` -> `user.id`
- `x-katanemo-session-id` -> `session.id`
- `x-katanemo-request-id` -> `request.id`

**Starting the trace collector:**

```bash
# Start Plano with built-in OTEL collector
planoai up config.yaml --with-tracing
```

Sampling rates: 100% for dev/staging, 5–20% for high-traffic production, 100% for low-traffic production. `trace_arch_internal: true` adds spans showing which routing preference matched — essential for debugging preference configuration.

Reference: [https://github.com/katanemo/archgw](https://github.com/katanemo/archgw)
