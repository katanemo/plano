---
title: Choose the Right Listener Type for Your Use Case
impact: CRITICAL
impactDescription: The listener type determines the entire request processing pipeline — choosing the wrong type means features like prompt functions or agent routing are unavailable
tags: config, listeners, architecture, routing
---

## Choose the Right Listener Type for Your Use Case

Plano supports three listener types, each serving a distinct purpose. `listeners` is the only required top-level array in a Plano config. Every listener needs at minimum a `type`, `name`, and `port`.

| Type | Use When | Key Feature |
|------|----------|-------------|
| `model` | You want an OpenAI-compatible LLM gateway | Routes to multiple LLM providers, supports model aliases and routing preferences |
| `prompt` | You want LLM-callable custom functions | Define `prompt_targets` that the LLM dispatches as function calls |
| `agent` | You want multi-agent orchestration | Routes user requests to specialized sub-agents by matching agent descriptions |

**Incorrect (using `model` when agents need orchestration):**

```yaml
version: v0.3.0

# Wrong: a model listener cannot route to backend agent services
listeners:
  - type: model
    name: main
    port: 12000

agents:
  - id: weather_agent
    url: http://host.docker.internal:8001
```

**Correct (use `agent` listener for multi-agent systems):**

```yaml
version: v0.3.0

agents:
  - id: weather_agent
    url: http://host.docker.internal:8001
  - id: travel_agent
    url: http://host.docker.internal:8002

listeners:
  - type: agent
    name: orchestrator
    port: 8000
    router: plano_orchestrator_v1
    agents:
      - id: weather_agent
        description: Provides real-time weather, forecasts, and conditions for any city.
      - id: travel_agent
        description: Books flights, hotels, and travel itineraries.

model_providers:
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    default: true
```

A single Plano instance can expose multiple listeners on different ports, each with a different type, to serve different clients simultaneously.

Reference: https://github.com/katanemo/archgw
