---
name: plano-agent-orchestration
description: Improve multi-agent orchestration in Plano. Use for agent registration, agent listener wiring, and capability-focused agent descriptions for accurate routing.
license: Apache-2.0
metadata:
  author: katanemo
  version: "1.0.0"
---

# Plano Agent Orchestration

Use this skill for agent listener quality, sub-agent registration, and route accuracy.

## When To Use

- "Fix multi-agent routing"
- "Validate agents vs listeners.agents config"
- "Improve agent descriptions"
- "Set up a reliable orchestrator"

## Apply These Rules

- `agent-orchestration`
- `agent-descriptions`

## Execution Checklist

1. Verify each agent exists in both `agents` and `listeners[].agents`.
2. Ensure one fallback/default agent where appropriate.
3. Rewrite descriptions to be capability-focused and non-overlapping.
4. Keep descriptions specific, concise, and example-driven.
5. Provide test prompts to validate routing outcomes.
