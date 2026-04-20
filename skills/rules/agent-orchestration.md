---
title: Register All Sub-Agents in Both `agents` and `listeners.agents`
impact: CRITICAL
impactDescription: An agent registered only in `agents` but not referenced in a listener's agent list is unreachable; an agent listed in a listener but missing from `agents` causes a startup error
tags: agent, orchestration, config, multi-agent
---

## Register All Sub-Agents in Both `agents` and `listeners.agents`

Plano's agent system has two separate concepts: the global `agents` array (defines the agent's ID and backend URL) and the `listeners[].agents` array (controls which agents are available to an orchestrator and provides their routing descriptions). Both must reference the same agent ID.

**Incorrect (agent defined globally but not referenced in listener):**

```yaml
version: v0.3.0

agents:
  - id: weather_agent
    url: http://host.docker.internal:8001
  - id: news_agent              # Defined but never referenced in any listener
    url: http://host.docker.internal:8002

listeners:
  - type: agent
    name: orchestrator
    port: 8000
    router: plano_orchestrator_v1
    agents:
      - id: weather_agent
        description: Provides weather forecasts and current conditions.
      # news_agent is missing here — the orchestrator cannot route to it
```

**Incorrect (listener references an agent ID not in the global agents list):**

```yaml
agents:
  - id: weather_agent
    url: http://host.docker.internal:8001

listeners:
  - type: agent
    name: orchestrator
    port: 8000
    router: plano_orchestrator_v1
    agents:
      - id: weather_agent
        description: Provides weather forecasts.
      - id: flights_agent        # ID not in global agents[] — startup error
        description: Provides flight status information.
```

**Correct (every agent ID appears in both places):**

```yaml
version: v0.3.0

agents:
  - id: weather_agent
    url: http://host.docker.internal:8001
  - id: flights_agent
    url: http://host.docker.internal:8002
  - id: hotels_agent
    url: http://host.docker.internal:8003

model_providers:
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    default: true

listeners:
  - type: agent
    name: travel_orchestrator
    port: 8000
    router: plano_orchestrator_v1
    agents:
      - id: weather_agent
        description: Real-time weather, forecasts, and climate data for any city.
      - id: flights_agent
        description: Live flight status, schedules, gates, and delays.
      - id: hotels_agent
        description: Hotel search, availability, pricing, and booking.
        default: true    # Fallback if no other agent matches
```

Set `default: true` on one agent in each listener's agents list to handle unmatched requests. The agent's URL in the global `agents` array is the HTTP endpoint Plano forwards matching requests to — it must be reachable from within the Docker container (use `host.docker.internal` for services on the host).

Reference: https://github.com/katanemo/archgw
