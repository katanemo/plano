# Travel Booking Agent Demo

A production-ready multi-agent travel booking system demonstrating Plano's intelligent agent routing. This demo showcases two specialized agents working together to help users plan trips with weather information and flight searches.

## Overview

This demo consists of two intelligent agents that work together seamlessly:

- **Weather Agent** - Real-time weather conditions and multi-day forecasts for any city worldwide
- **Flight Agent** - Live flight information between airports with real-time tracking

All agents use Plano's agent router to intelligently route user requests to the appropriate specialized agent based on conversation context and user intent. Both agents run as Docker containers for easy deployment.

## Features

- **Intelligent Routing**: Plano automatically routes requests to the right agent
- **Conversation Context**: Agents understand follow-up questions and references
- **Real-Time Data**: Live weather and flight data from public APIs
- **Multi-Day Forecasts**: Weather agent supports up to 16-day forecasts
- **LLM-Powered**: Uses GPT-4o-mini for extraction and GPT-4o for responses
- **Streaming Responses**: Real-time streaming for better user experience

## Prerequisites

- Docker and Docker Compose
- [Plano CLI](https://docs.planoai.dev) installed
- OpenAI API key

## Quick Start

### 1. Set Environment Variables

Create a `.env` file or export environment variables:

```bash
export AEROAPI_KEY="your-flightaware-api-key"  # Optional, demo key included
```

### 2. Start All Agents with Docker

```bash
chmod +x start_agents.sh
./start_agents.sh
```

Or directly:

```bash
docker compose up --build
```

This starts:
- Weather Agent on port 10510
- Flight Agent on port 10520
- Open WebUI on port 8080

### 3. Start Plano Orchestrator

In a new terminal:

```bash
cd /path/to/travel_agents
planoai up config.yaml
# Or if installed with uv: uvx planoai up config.yaml
```

The gateway will start on port 8001 and route requests to the appropriate agents.

### 4. Test the System

**Option 1**: Use Open WebUI at http://localhost:8080

**Option 2**: Send requests directly to Plano Orchestrator:

```bash
curl http://localhost:8001/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o",
    "messages": [
      {"role": "user", "content": "What is the weather like in Paris?"}
    ]
  }'
```

## Example Conversations

### Weather Query
```
User: What's the weather in Istanbul?
Assistant: [Weather Agent provides current conditions and forecast]
```

### Flight Search
```
User: What flights go from London to Seattle?
Assistant: [Flight Agent shows available flights with schedules and status]
```

### Multi-Agent Conversation
```
User: What's the weather in Istanbul?
Assistant: [Weather information]

User: Do they fly out from Seattle?
Assistant: [Flight information from Istanbul to Seattle]
```

The system understands context and pronouns, automatically routing to the right agent.

### Multi-Intent Queries
```
User: What's the weather in Seattle, and do any flights go direct to New York?
Assistant: [Both weather_agent and flight_agent respond simultaneously]
  - Weather Agent: [Weather information for Seattle]
  - Flight Agent: [Flight information from Seattle to New York]
```

The orchestrator can select multiple agents simultaneously for queries containing multiple intents.

## Agent Details

### Weather Agent
- **Port**: 10510
- **API**: Open-Meteo (free, no API key)
- **Capabilities**: Current weather, multi-day forecasts, temperature, conditions, sunrise/sunset

### Flight Agent
- **Port**: 10520
- **API**: FlightAware AeroAPI
- **Capabilities**: Real-time flight status, schedules, delays, gates, terminals, live tracking

## Architecture

```
    User Request
         ↓
    Plano (8001)
     [Orchestrator]
         |
    ┌────┴────┐
    ↓         ↓
Weather    Flight
Agent      Agent
(10510)    (10520)
[Docker]   [Docker]
```

```

Each agent:
1. Extracts intent using GPT-4o-mini (with OpenTelemetry tracing)
2. Fetches real-time data from APIs
3. Generates response using GPT-4o
4. Streams response back to user

Both agents run as Docker containers and communicate with Plano via `host.docker.internal`.

## Project Structure

```
travel_agents/
├── config.yaml          # Plano configuration
├── docker-compose.yaml       # Docker services orchestration
├── Dockerfile               # Multi-agent container image
├── start_agents.sh          # Quick start script
├── pyproject.toml           # Python dependencies
└── src/
    └── travel_agents/
        ├── __init__.py      # CLI entry point
        ├── weather_agent.py # Weather forecast agent (multi-day support)
        └── flight_agent.py  # Flight information agent
```

## Configuration Files

### config.yaml

Defines the two agents, their descriptions, and routing configuration. The agent router uses these descriptions to intelligently route requests.

### docker-compose.yaml

Orchestrates the deployment of:
- Weather Agent (builds from Dockerfile)
- Flight Agent (builds from Dockerfile)
- Open WebUI (for testing)
- Jaeger (for distributed tracing)

## Troubleshooting

**Docker containers won't start**
- Verify Docker and Docker Compose are installed
- Check that ports 10510, 10520, 8080 are available
- Review container logs: `docker compose logs weather-agent` or `docker compose logs flight-agent`

**Plano won't start**
- Verify Plano is installed: `plano --version`
- Ensure you're in the travel_agents directory
- Check config.yaml is valid

**No response from agents**
- Verify all containers are running: `docker compose ps`
- Check that Plano is running on port 8001
- Review agent logs: `docker compose logs -f`
- Verify `host.docker.internal` resolves correctly (should point to host machine)

## API Endpoints

All agents expose OpenAI-compatible chat completion endpoints:

- `POST /v1/chat/completions` - Chat completion endpoint
- `GET /health` - Health check endpoint
