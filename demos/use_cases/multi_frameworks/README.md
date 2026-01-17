# Multi-Framework Travel Agents

This demo shows how Plano orchestrates multiple agents built on different frameworks. We run a CrewAI flight agent and a LangChain weather agent side by side to highlight Plano's framework-agnostic design while still providing a consistent gateway for requests, tools, and telemetry.

## Overview

We act as an orchestration layer between clients and specialized agents. Each agent is built using a different Python AI framework—CrewAI for flight searches and LangChain for weather forecasts—demonstrating that Plano works seamlessly across frameworks without requiring modifications to agent code or forcing a single framework choice.

When a user asks a question like "What's the weather in Seattle and are there flights from San Francisco?", Plano automatically routes the weather portion to the LangChain agent and the flight portion to the CrewAI agent, then combines their responses into a single coherent answer. This orchestration happens transparently, with agents remaining unaware of each other.

## What runs

The demo spins up five services:

**Plano** (ports 12000, 8001) is the orchestration engine. It exposes two endpoints: the main gateway on port 12000 for direct API access, and the travel booking service on port 8001 that orchestrates agent routing. The gateway reads `config.yaml` to learn about available agents, their capabilities, and how to route requests.

**Weather Agent** (port 10510) is a specialized service built with LangChain that handles weather queries. It connects to the Open-Meteo API to fetch real-time weather data and forecasts for any city worldwide. The agent receives requests from Plano, processes weather-related questions, and returns structured responses.

**Flight Agent** (port 10520) is a flight information service built with CrewAI that provides live flight data between airports. It uses the FlightAware AeroAPI to deliver real-time status, gate information, delays, and schedules. Like the weather agent, it operates independently and only handles flight-related queries.

**Open WebUI** (port 8080) provides a chat interface for interacting with the system. It's configured to send requests through Plano's orchestration endpoint at port 8001, giving you a familiar chat experience backed by multi-agent orchestration.

**Jaeger** (port 16686) collects distributed traces from all services. You can view request flows, latencies, and agent interactions through the Jaeger UI, making it easy to debug and understand how Plano routes and coordinates agents.

## How orchestration works

Plano uses the `plano_orchestrator_v1` router configured in `config.yaml`. When a request arrives, Plano first analyzes the user's question to determine which agents are relevant. Each agent has a detailed description explaining its capabilities—weather data, flight information, etc.—and Plano uses an LLM to decide which agent(s) should handle the request.

Both agents call back to Plano's gateway when they need LLM inference, creating a controlled loop where Plano provides the underlying model (GPT-4o or GPT-4o-mini, depending on the task) while agents focus purely on their domain logic. This means you can swap models, add rate limiting, or implement caching at the gateway level without touching agent code.

## Framework agnostic design

The key insight is that Plano doesn't care what framework you use.

The LangChain agent uses LangChain's tool calling and conversation patterns. The CrewAI agent uses CrewAI's crew and task abstractions. Plano simply expects OpenAI-compatible HTTP requests and responses, which both frameworks naturally support.

You could add a third agent built with raw OpenAI SDK calls, or LlamaIndex, or any other framework—Plano would orchestrate it identically. There's no vendor lock-in, no forced migration, and no framework coupling.

## Running the demo

Start all services with:

```bash
docker compose up -d
```

This builds and launches Plano, both agents, the web UI, and Jaeger. Wait about 30 seconds for services to initialize, then open `http://localhost:8080` to access the chat interface.

Try asking questions like:
- "What's the weather in Seattle?"
- "Are there flights from San Francisco to Seattle?"
- "What's the weather in Boston and are there flights from New York?"

You'll see Plano route requests to the appropriate agent(s) and combine their responses. Open `http://localhost:16686` to view traces in Jaeger and see exactly how requests flow through the system.

## Configuration

The `config.yaml` file defines the system's behavior. It declares two agents (`weather_agent` and `flight_agent`), two model providers (GPT-4o and GPT-4o-mini), and one listener on port 8001 that uses the Plano orchestrator. Each agent's description tells Plano what that agent can do, which drives routing decisions.

## Environment variables

You'll need `OPENAI_API_KEY` set in your environment for LLM access. Optionally set `AEROAPI_KEY` if you have a FlightAware API key for enhanced flight data (the demo works without it, but live data requires the key).


We're excited to see how you use this demo to build your own multi-framework agents, and we'd love to hear your feedback!
