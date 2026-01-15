# Travel Booking Agent Demo (LangChain-first)

A lightweight **LangChain-powered** multi-agent travel booking system that runs two agents behind Plano's router: a weather agent and a flight agent. Each agent is implemented with LangChain's tool-calling capabilities for a clean, modular design.

## Overview

This demo showcases how to integrate LangChain agents with Plano:

- **Weather Agent** - Uses `@tool` decorator to fetch real-time weather from Open-Meteo API
- **Flight Agent** - Uses `@tool` decorator to search flights via FlightAware API

Both agents use LangChain's `create_tool_calling_agent` and `AgentExecutor` for:
- Automatic tool selection and execution
- Streaming responses via `astream_events`
- OpenAI-compatible API endpoints

## Architecture

```
    User Request
         ↓
    Plano Gateway (8001)
     [Orchestrator]
         |
    ┌────┴────┐
    ↓         ↓
Weather    Flight
Agent      Agent
(10510)    (10520)
   │          │
   └──────────┴─── LangChain Tools ───→ External APIs
```

Each agent:
1. Receives OpenAI-compatible chat requests
2. Uses LangChain's agent executor with tools
3. Tools fetch data from external APIs (Open-Meteo, FlightAware)
4. Streams responses back in OpenAI format

## LangChain Implementation Details

### Weather Agent Tools

```python
@tool
async def get_weather(city: str, days: int = 1) -> str:
    """Get weather information for a city."""
    # Geocode city → fetch weather from Open-Meteo
    ...
```

### Flight Agent Tools

```python
@tool
async def resolve_airport_code(city: str) -> str:
    """Convert city name to IATA airport code."""
    ...

@tool
async def search_flights(origin_code: str, destination_code: str, travel_date: str = None) -> str:
    """Search flights between two airports."""
    # Query FlightAware AeroAPI
    ...
```

### Agent Setup

```python
from langchain.agents import create_tool_calling_agent, AgentExecutor
from langchain_openai import ChatOpenAI

llm = ChatOpenAI(
    model="openai/gpt-4o",
    base_url=LLM_GATEWAY_ENDPOINT,  # Plano gateway
    api_key="EMPTY",
    streaming=True,
)

agent = create_tool_calling_agent(llm, tools, prompt)
executor = AgentExecutor(agent=agent, tools=tools, verbose=True)
```

## Prerequisites

- Docker and Docker Compose
- [Plano CLI](https://docs.planoai.dev) installed
- OpenAI API key
- (Optional) FlightAware AeroAPI key for live flight data

## Quick Start

### 1. Set Environment Variables

```bash
export OPENAI_API_KEY="your-openai-api-key"
export AEROAPI_KEY="your-flightaware-api-key"  # Optional for flight agent
```

### 2. Start All Services with Docker Compose

```bash
docker compose up --build
```

This starts:
- Plano Gateway on port 8001 (and 12000 for LLM proxy)
- Weather Agent on port 10510
- Flight Agent on port 10520
- Open WebUI on port 8080
- Jaeger tracing on port 16686

### 3. Test the System

**Option 1**: Use Open WebUI at http://localhost:8080

**Option 2**: Send requests directly:

```bash
# Weather query
curl http://localhost:8001/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o",
    "messages": [{"role": "user", "content": "What is the weather like in Paris?"}]
  }'

# Flight query
curl http://localhost:8001/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o",
    "messages": [{"role": "user", "content": "Find flights from Seattle to New York"}]
  }'
```

## Example Conversations

### Weather Query
```
User: What's the 5-day forecast for Tokyo?
Assistant: [Weather Agent uses get_weather tool → presents forecast]
```

### Flight Search
```
User: What flights go from London to Seattle tomorrow?
Assistant: [Flight Agent uses resolve_airport_code → search_flights → presents results]
```

### Multi-Agent (via Plano routing)
```
User: What's the weather in Seattle, and any flights to New York?
Assistant: [Plano routes to both agents → combined response]
```

## Local Development

### Run agents locally (without Docker)

```bash
# Install dependencies
cd demos/use_cases/langchain
uv sync

# Start weather agent
uv run python src/travel_agents/weather_agent.py

# In another terminal, start flight agent
uv run python src/travel_agents/flight_agent.py
```

### Using the CLI

```bash
# Start weather agent
uv run travel_agents weather --port 10510

# Start flight agent
uv run travel_agents flight --port 10520
```

## Project Structure

```
langchain/
├── config.yaml              # Plano gateway configuration
├── docker-compose.yaml      # Docker services orchestration
├── Dockerfile               # Container image
├── pyproject.toml           # Python dependencies (LangChain, FastAPI, etc.)
├── README.md                # This file
└── src/
    └── travel_agents/
        ├── __init__.py      # CLI entry points
        ├── weather_agent.py # Weather agent with get_weather tool
        └── flight_agent.py  # Flight agent with search_flights tool
```

## Configuration

### config.yaml

Defines agent descriptions for Plano's intelligent routing:

```yaml
agents:
  - id: weather_agent
    url: http://host.docker.internal:10510
  - id: flight_agent
    url: http://host.docker.internal:10520

listeners:
  - type: agent
    name: travel_booking_service
    port: 8001
    router: plano_orchestrator_v1
    agents:
      - id: weather_agent
        description: |
          WeatherAgent provides real-time weather and forecasts...
      - id: flight_agent
        description: |
          FlightAgent provides live flight information...
```

## Troubleshooting

**Agents not responding**
- Check container logs: `docker compose logs weather-agent`
- Verify Plano is running: `curl http://localhost:8001/health`

**LangChain agent errors**
- Check that `LLM_GATEWAY_ENDPOINT` is correctly set
- Verify OpenAI API key is valid

**Flight API returning mock data**
- Set `AEROAPI_KEY` for live FlightAware data
- Without the key, the agent returns sample flight data

## API Endpoints

All agents expose OpenAI-compatible endpoints:

- `POST /v1/chat/completions` - Chat completion (streaming)
- `GET /health` - Health check

## Key Dependencies

- `langchain>=0.3.13` - Agent framework
- `langchain-openai>=0.2.14` - OpenAI integration via Plano
- `fastapi>=0.115.0` - Web framework
- `httpx>=0.24.0` - Async HTTP client for API calls
