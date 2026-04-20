---
title: Use `planoai trace` to Inspect Routing Decisions
impact: MEDIUM-HIGH
impactDescription: The trace CLI lets you verify which model was selected, why, and how long each step took — without setting up a full OTEL backend
tags: observability, tracing, cli, debugging, routing
---

## Use `planoai trace` to Inspect Routing Decisions

`planoai trace` provides a built-in trace viewer backed by an in-memory OTEL collector. Use it to inspect routing decisions, verify preference matching, measure filter latency, and debug failed requests — all from the CLI without configuring Jaeger, Zipkin, or another backend.

**Workflow: start collector, run requests, then inspect traces:**

```bash
# 1. Start Plano with the built-in trace collector (recommended)
planoai up config.yaml --with-tracing

# 2. Send test requests through Plano
curl http://localhost:12000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "plano.v1", "messages": [{"role": "user", "content": "Write a Python function to sort a list"}]}'

# 3. Show the latest trace
planoai trace
```

You can also run the trace listener directly:

```bash
planoai trace listen # available on a process ID running OTEL collector
```

Stop the background trace listener:

```bash
planoai trace down
```

**Useful trace viewer patterns:**

```bash
# Show latest trace (default target is "last")
planoai trace

# List available trace IDs
planoai trace --list

# Show all traces
planoai trace any

# Show a specific trace (short 8-char or full 32-char ID)
planoai trace 7f4e9a1c
planoai trace 7f4e9a1c0d9d4a0bb9bf5a8a7d13f62a

# Filter by specific span attributes (AND semantics for repeated --where)
planoai trace any --where llm.model=gpt-4o-mini

# Filter by user ID (if header prefix is x-katanemo-, x-katanemo-user-id maps to user.id)
planoai trace any --where user.id=user_123

# Limit results for a quick sanity check
planoai trace any --limit 5

# Time window filter
planoai trace any --since 30m

# Filter displayed attributes by key pattern
planoai trace any --filter "http.*"

# Output machine-readable JSON
planoai trace any --json
```

**What to look for in traces:**


| Span name           | What it tells you                                             |
| ------------------- | ------------------------------------------------------------- |
| `plano.routing`     | Which routing preference matched and which model was selected |
| `plano.filter.<id>` | How long each filter in the chain took                        |
| `plano.llm.request` | Time to first token and full response time                    |
| `plano.agent.route` | Which agent description matched for agent listeners           |


Reference: [https://github.com/katanemo/archgw](https://github.com/katanemo/archgw)
