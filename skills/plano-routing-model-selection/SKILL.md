---
name: plano-routing-model-selection
description: Optimize Plano model routing and selection. Use for provider defaults, model aliases, passthrough auth, and routing preference quality.
license: Apache-2.0
metadata:
  author: katanemo
  version: "1.0.0"
---

# Plano Routing and Model Selection

Use this skill when requests are routed to the wrong model, costs are high, or fallback behavior is unclear.

## When To Use

- "Improve model routing"
- "Add aliases and defaults"
- "Fix passthrough auth with proxy providers"
- "Tune routing preferences for better classification"

## Apply These Rules

- `routing-default`
- `routing-aliases`
- `routing-passthrough`
- `routing-preferences`

## Execution Checklist

1. Ensure exactly one `default: true` provider.
2. Add semantic aliases for stable client contracts.
3. Configure passthrough auth only where required.
4. Rewrite vague preference descriptions with concrete task scopes.
5. Validate routing behavior using trace-based checks.
