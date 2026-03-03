---
title: Write Capability-Focused Agent Descriptions for Accurate Routing
impact: HIGH
impactDescription: The orchestrator LLM routes requests purely by reading agent descriptions — poor descriptions cause misroutes to the wrong specialized agent
tags: agent, orchestration, descriptions, routing, multi-agent
---

## Write Capability-Focused Agent Descriptions for Accurate Routing

In an `agent` listener, Plano's orchestrator reads each agent's `description` and routes user requests to the best-matching agent. This is LLM-based intent matching — the description is the entire specification the router sees. Write it as a capability manifest: what can this agent do, what data does it have access to, and what types of requests should it handle?

**Incorrect (generic, overlapping descriptions):**

```yaml
listeners:
  - type: agent
    name: orchestrator
    port: 8000
    router: plano_orchestrator_v1
    agents:
      - id: agent_1
        description: Helps users with information    # Too generic — matches everything

      - id: agent_2
        description: Also helps users               # Indistinguishable from agent_1
```

**Correct (specific capabilities, distinct domains, concrete examples):**

```yaml
version: v0.3.0

agents:
  - id: weather_agent
    url: http://host.docker.internal:8001
  - id: flight_agent
    url: http://host.docker.internal:8002
  - id: hotel_agent
    url: http://host.docker.internal:8003

listeners:
  - type: agent
    name: travel_orchestrator
    port: 8000
    router: plano_orchestrator_v1
    agents:
      - id: weather_agent
        description: >
          Provides real-time weather conditions and multi-day forecasts for any city
          worldwide. Handles questions about temperature, precipitation, wind, humidity,
          sunrise/sunset times, and severe weather alerts. Examples: "What's the weather
          in Tokyo?", "Will it rain in London this weekend?", "Sunrise time in New York."

      - id: flight_agent
        description: >
          Provides live flight status, schedules, gate information, delays, and
          aircraft details for any flight number or route between airports.
          Handles questions about departures, arrivals, and airline information.
          Examples: "Is AA123 on time?", "Flights from JFK to LAX tomorrow."

      - id: hotel_agent
        description: >
          Searches and books hotel accommodations, compares room types, pricing,
          and availability. Handles check-in/check-out dates, amenities, and
          cancellation policies. Examples: "Hotels near Times Square for next Friday."
```

**Description writing checklist:**
- State the primary domain in the first sentence
- List 3–5 specific data types or question categories this agent handles
- Include 2–3 concrete example user queries in quotes
- Avoid capability overlap between agents — if they overlap, the router will split traffic unpredictably
- Keep descriptions under 150 words — the orchestrator reads all descriptions per request

Reference: https://github.com/katanemo/archgw
