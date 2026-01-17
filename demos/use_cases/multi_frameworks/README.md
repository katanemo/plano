# Multi-Framework Travel Agents

This demo shows how Plano orchestrates multiple agents built on different frameworks. We run a CrewAI flight agent and a LangChain weather agent side by side to highlight that Plano is framework‑agnostic while still providing a consistent gateway for requests, tools, and telemetry.

## How it works

Plano sits between clients and agents. Each agent runs independently and exposes its own tools and behavior. The gateway routes requests to the right agent, normalizes requests/responses, and keeps orchestration consistent across frameworks without coupling them together.

## Agents

- **CrewAI Flight Agent** (Port 10520): flight search and itineraries
- **LangChain Weather Agent** (Port 10510): weather forecasts and conditions

## Quick start

```bash
docker compose build
docker compose up -d
```

## Environment variables

- `OPENAI_API_KEY`: required for LLM access
- `AEROAPI_KEY`: optional for flight data
- `LLM_GATEWAY_ENDPOINT`: Plano gateway endpoint (default: http://host.docker.internal:12000/v1)
