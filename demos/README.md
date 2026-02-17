# Plano Demos

This directory contains demos showcasing Plano's capabilities as an AI-native proxy for agentic applications.

## Getting Started

| Demo | Description |
|------|-------------|
| [Weather Forecast](getting_started/weather_forecast/) | Core function calling with a weather query agent, interactive chat UI, and Jaeger tracing |
| [LLM Gateway](getting_started/llm_gateway/) | Key management and dynamic routing to multiple LLM providers with header-based model override |

## LLM Routing

| Demo | Description |
|------|-------------|
| [Preference-Based Routing](llm_routing/preference_based_routing/) | Routes prompts to LLMs based on user-defined preferences and task type (e.g. code generation vs. understanding) |
| [Model Alias Routing](llm_routing/model_alias_routing/) | Maps semantic aliases (`arch.summarize.v1`) to provider-specific models for centralized governance |
| [Claude Code Router](llm_routing/claude_code_router/) | Extends Claude Code with multi-provider access and preference-aligned routing for coding tasks |

## Agent Orchestration

| Demo | Description |
|------|-------------|
| [Travel Agents](agent_orchestration/travel_agents/) | Multi-agent travel booking with weather and flight agents, intelligent routing, and OpenTelemetry tracing |
| [Multi-Agent CrewAI & LangChain](agent_orchestration/multi_agent_crewai_langchain/) | Framework-agnostic orchestration combining CrewAI and LangChain agents in unified conversations |

## Filter Chains

| Demo | Description |
|------|-------------|
| [HTTP Filter](filter_chains/http_filter/) | RAG agent with filter chains for input validation, query rewriting, and context building |
| [MCP Filter](filter_chains/mcp_filter/) | RAG agent using MCP-based filters for domain validation, query optimization, and knowledge base retrieval |

## Integrations

| Demo | Description |
|------|-------------|
| [Ollama](integrations/ollama/) | Use Ollama as a local LLM provider through Plano |
| [Spotify Bearer Auth](integrations/spotify_bearer_auth/) | Bearer token authentication for third-party APIs (Spotify new releases and top tracks) |

## Advanced

| Demo | Description |
|------|-------------|
| [Currency Exchange](advanced/currency_exchange/) | Function calling with public REST APIs (Frankfurter currency exchange) |
| [Stock Quote](advanced/stock_quote/) | Protected REST API integration with access key management |
| [Multi-Turn RAG](advanced/multi_turn_rag/) | Multi-turn conversational RAG agent for answering questions about energy sources |
| [Model Choice Test Harness](advanced/model_choice_test_harness/) | Evaluation framework for safely testing and switching between models with benchmark fixtures |
