# Multi-Framework Travel Agents

This project demonstrates multi-framework integration with both CrewAI and LangChain agents working together.

## Agents

- **CrewAI Flight Agent** (Port 10520): Handles flight-related queries
- **LangChain Weather Agent** (Port 10510): Handles weather-related queries

## Setup

```bash
docker compose build
docker compose up -d
```

## Environment Variables

- `OPENAI_API_KEY`: Required for LLM access
- `AEROAPI_KEY`: Optional for flight data
- `LLM_GATEWAY_ENDPOINT`: Plano gateway endpoint (default: http://host.docker.internal:12000/v1)
