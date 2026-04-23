---
name: plano-advanced-patterns
description: Design advanced Plano architectures. Use for multi-listener systems, prompt target schema quality, and layered orchestration patterns.
license: Apache-2.0
metadata:
  author: katanemo
  version: "1.0.0"
---

# Plano Advanced Patterns

Use this skill for higher-order architecture decisions once fundamentals are stable.

## When To Use

- "Design a multi-listener Plano architecture"
- "Improve prompt target schema precision"
- "Combine model, prompt, and agent listeners"
- "Refine advanced routing/function-calling behavior"

## Apply These Rules

- `advanced-multi-listener`
- `advanced-prompt-targets`

## Execution Checklist

1. Use multiple listeners only when interfaces are truly distinct.
2. Keep provider/routing definitions shared and consistent.
3. Define prompt target parameters with strict, explicit schemas.
4. Minimize ambiguity that causes malformed tool calls.
5. Provide migration-safe recommendations and test scenarios.
