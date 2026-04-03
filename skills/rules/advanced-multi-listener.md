---
title: Combine Multiple Listener Types for Layered Agent Architectures
impact: MEDIUM
impactDescription: Using a single listener type forces all traffic through one gateway pattern — combining types lets you serve different clients with the right interface without running multiple Plano instances
tags: advanced, multi-listener, architecture, agent, model, prompt
---

## Combine Multiple Listener Types for Layered Agent Architectures

A single Plano `config.yaml` can define multiple listeners of different types, each on a separate port. This lets you serve different client types simultaneously: an OpenAI-compatible model gateway for direct API clients, a prompt gateway for LLM-callable function applications, and an agent orchestrator for multi-agent workflows — all from one Plano instance sharing the same model providers.

**Single listener (limited — forces all clients through one interface):**

```yaml
version: v0.3.0

listeners:
  - type: model             # Only model clients can use this
    name: model_gateway
    port: 12000

# Prompt target clients and agent clients cannot connect
```

**Multi-listener architecture (serves all client types):**

```yaml
version: v0.3.0

# --- Shared model providers ---
model_providers:
  - model: openai/gpt-4o-mini
    access_key: $OPENAI_API_KEY
    default: true
    routing_preferences:
      - name: quick tasks
        description: Short answers, formatting, classification, simple generation

  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    routing_preferences:
      - name: complex reasoning
        description: Multi-step analysis, code generation, research synthesis

  - model: anthropic/claude-sonnet-4-20250514
    access_key: $ANTHROPIC_API_KEY
    routing_preferences:
      - name: long documents
        description: Summarizing or analyzing very long documents, PDFs, transcripts

# --- Listener 1: OpenAI-compatible API gateway ---
# For: SDK clients, Claude Code, LangChain, etc.
listeners:
  - type: model
    name: model_gateway
    port: 12000
    timeout: "120s"

# --- Listener 2: Prompt function gateway ---
# For: Applications that expose LLM-callable APIs
  - type: prompt
    name: function_gateway
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

# --- Prompt targets (for function gateway) ---
endpoints:
  internal_api:
    endpoint: host.docker.internal
    protocol: http

prompt_targets:
  - name: search_knowledge_base
    description: Search the internal knowledge base for relevant documents and facts.
    parameters:
      - name: query
        type: str
        required: true
        description: Search query to find relevant information
    endpoint:
      name: internal_api
      path: /kb/search?q={query}
      http_method: GET

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

This architecture serves: SDK clients on `:12000`, function-calling apps on `:10000`, and multi-agent orchestration on `:8000` — with shared cost-optimized routing across all three.

Reference: [https://github.com/katanemo/archgw](https://github.com/katanemo/archgw)
