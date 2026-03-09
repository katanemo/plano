---
name: plano-observability-debugging
description: Improve Plano tracing and debugging workflows. Use for sampling strategy, span attributes, and trace query-based root-cause analysis.
license: Apache-2.0
metadata:
  author: katanemo
  version: "1.0.0"
---

# Plano Observability and Debugging

Use this skill to make routing and latency behavior inspectable and debuggable.

## When To Use

- "Enable tracing correctly"
- "Add useful span attributes"
- "Debug why a request routed incorrectly"
- "Inspect filter/model latency from traces"

## Apply These Rules

- `observe-tracing`
- `observe-span-attributes`
- `observe-trace-query`

## Execution Checklist

1. Enable tracing with environment-appropriate sampling.
2. Add useful static and header-derived span attributes.
3. Use `planoai trace` filters to isolate route and latency issues.
4. Prefer trace evidence over assumptions in recommendations.
5. Return exact commands to reproduce and validate findings.
