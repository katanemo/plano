---
title: Order Filter Chains with Guards First, Enrichment Last
impact: HIGH
impactDescription: Running context builders before input guards means jailbreak attempts get RAG-enriched context before being blocked — wasting compute and risking data exposure
tags: filter, guardrails, security, pipeline, ordering
---

## Order Filter Chains with Guards First, Enrichment Last

A `filter_chain` is an ordered list of filter IDs applied sequentially to each request. The order is semantically meaningful: each filter receives the output of the previous one. Safety and validation filters must run first to short-circuit bad requests before expensive enrichment filters process them.

**Recommended filter chain order:**

1. **Input guards** — jailbreak detection, PII detection, topic restrictions (reject early)
2. **Query rewriting** — normalize or enhance the user query
3. **Context building** — RAG retrieval, tool lookup, knowledge injection (expensive)
4. **Output guards** — validate or sanitize LLM response before returning

**Incorrect (context built before guards — wasteful and potentially unsafe):**

```yaml
filters:
  - id: context_builder
    url: http://host.docker.internal:10502    # Runs expensive RAG retrieval first
  - id: query_rewriter
    url: http://host.docker.internal:10501
  - id: input_guards
    url: http://host.docker.internal:10500    # Guards run last — jailbreak gets context

listeners:
  - type: agent
    name: rag_orchestrator
    port: 8000
    router: plano_orchestrator_v1
    agents:
      - id: rag_agent
        filter_chain:
          - context_builder   # Wrong: expensive enrichment before safety check
          - query_rewriter
          - input_guards
```

**Correct (guards block bad requests before any enrichment):**

```yaml
version: v0.3.0

filters:
  - id: input_guards
    url: http://host.docker.internal:10500
    type: mcp
    transport: streamable-http
  - id: query_rewriter
    url: http://host.docker.internal:10501
    type: mcp
    transport: streamable-http
  - id: context_builder
    url: http://host.docker.internal:10502
    type: mcp
    transport: streamable-http

listeners:
  - type: agent
    name: rag_orchestrator
    port: 8000
    router: plano_orchestrator_v1
    agents:
      - id: rag_agent
        description: Answers questions using internal knowledge base documents.
        filter_chain:
          - input_guards      # 1. Block jailbreaks and policy violations
          - query_rewriter    # 2. Normalize the safe query
          - context_builder   # 3. Retrieve relevant context for the clean query
```

Different agents within the same listener can have different filter chains — a public-facing agent may need all guards while an internal admin agent may skip them.

Reference: https://github.com/katanemo/archgw
