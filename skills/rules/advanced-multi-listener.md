---
title: Combine Multiple Listener Types for Layered Agent Architectures
impact: MEDIUM
impactDescription: Using a single listener type forces all traffic through one gateway pattern — combining types lets you serve different clients with the right interface without running multiple Plano instances
tags: advanced, multi-listener, architecture, agent, model, prompt
---

## Combine Multiple Listener Types for Layered Agent Architectures

A single Plano `config.yaml` can define multiple listeners of different types, each on a separate port. This lets you serve different client types simultaneously: an OpenAI-compatible model gateway for direct API clients, a prompt gateway for inbound prompt traffic, and an agent orchestrator for multi-agent workflows — all from one Plano instance sharing the same model providers.

**Single listener (limited — forces all clients through one interface):**

```yaml
version: v0.3.0

listeners:
  - type: model             # Only model clients can use this
    name: model_gateway
    port: 12000

# Agent clients cannot connect without an agent listener
```

**Multi-listener architecture (serves model and agent clients):**

```yaml
version: v0.4.0

# --- Shared model providers ---
model_providers:
  - model: openai/gpt-4o-mini
    access_key: $OPENAI_API_KEY
    default: true

  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY

  - model: anthropic/claude-sonnet-4-6
    access_key: $ANTHROPIC_API_KEY

# --- Shared routing_preferences (top-level, v0.4.0+) ---
routing_preferences:
  - name: quick tasks
    description: Short answers, formatting, classification, simple generation
    models:
      - openai/gpt-4o-mini
  - name: complex reasoning
    description: Multi-step analysis, code generation, research synthesis
    models:
      - openai/gpt-4o
      - anthropic/claude-sonnet-4-6
  - name: long documents
    description: Summarizing or analyzing very long documents, PDFs, transcripts
    models:
      - anthropic/claude-sonnet-4-6
      - openai/gpt-4o

# --- Listener 1: OpenAI-compatible API gateway ---
# For: SDK clients, Claude Code, LangChain, etc.
listeners:
  - type: model
    name: model_gateway
    port: 12000
    timeout: "120s"

# --- Listener 2: Prompt gateway ---
# For: inbound prompt traffic via the prompt gateway WASM filter
  - type: prompt
    name: prompt_gateway
    port: 10000
    timeout: "60s"

# --- Listener 3: Agent orchestration gateway ---
# For: Multi-agent application clients
  - type: agent
    name: agent_orchestrator
    port: 8000
    timeout: "90s"
    router: plano_orchestrator_v1
    agents:
      - id: research_agent
        description: Searches, synthesizes, and summarizes information from multiple sources.
        filter_chain:
          - input_guards
          - context_builder
      - id: code_agent
        description: Writes, reviews, debugs, and explains code across all languages.
        default: true

# --- Agents ---
agents:
  - id: research_agent
    url: http://host.docker.internal:8001
  - id: code_agent
    url: http://host.docker.internal:8002

# --- Filters ---
filters:
  - id: input_guards
    url: http://host.docker.internal:10500
    type: mcp
    transport: streamable-http
  - id: context_builder
    url: http://host.docker.internal:10501
    type: mcp
    transport: streamable-http

# --- Observability ---
model_aliases:
  plano.fast.v1:
    target: gpt-4o-mini
  plano.smart.v1:
    target: gpt-4o

tracing:
  random_sampling: 50
  trace_arch_internal: true
  span_attributes:
    static:
      environment: production
    header_prefixes:
      - x-katanemo-
```

This architecture serves: SDK clients on `:12000`, prompt-gateway traffic on `:10000`, and multi-agent orchestration on `:8000` — with shared cost-optimized routing across all three.

Reference: [https://github.com/katanemo/archgw](https://github.com/katanemo/archgw)
