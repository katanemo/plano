---
name: plano-filter-guardrails
description: Harden Plano filter chains and guardrails. Use for MCP filter setup, prompt guard responses, and safe filter ordering.
license: Apache-2.0
metadata:
  author: katanemo
  version: "1.0.0"
---

# Plano Filter Chains and Guardrails

Use this skill when safety controls or filter pipelines need correction.

## When To Use

- "Fix filter chain ordering"
- "Set up MCP filters correctly"
- "Improve guardrail rejection behavior"
- "Harden request processing for safety"

## Apply These Rules

- `filter-mcp`
- `filter-guardrails`
- `filter-ordering`

## Execution Checklist

1. Configure filter `type`, `transport`, and `tool` explicitly for MCP.
2. Ensure rejection messages are clear and actionable.
3. Order chain as guards -> rewriters -> enrichment -> output checks.
4. Prevent expensive enrichment on unsafe requests.
5. Verify with representative blocked and allowed test prompts.
