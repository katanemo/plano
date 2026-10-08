---
name: plano-advanced-patterns
description: Design advanced Plano architectures. Use for multi-listener systems and layered orchestration patterns.
license: Apache-2.0
metadata:
  author: katanemo
  version: "1.0.0"
---

# Plano Advanced Patterns

Use this skill for higher-order architecture decisions once fundamentals are stable.

## When To Use

- "Design a multi-listener Plano architecture"
- "Combine model, prompt, and agent listeners"
- "Refine advanced routing and orchestration behavior"

## Apply These Rules

- `advanced-multi-listener`

## Execution Checklist

1. Use multiple listeners only when interfaces are truly distinct.
2. Keep provider/routing definitions shared and consistent.
3. Prefer agent listeners for orchestration; use prompt listeners for prompt-gateway traffic.
4. Provide migration-safe recommendations and test scenarios.
