---
name: plano-agent-skills
description: Best practices for building agents and agentic applications with Plano, including configuration, routing, orchestration, guardrails, observability, and deployment.
license: Apache-2.0
metadata:
  author: katanemo
  version: "1.0.0"
---

# Plano Agent Skills

Comprehensive Plano guidance for coding agents. Use this umbrella skill when a task spans multiple areas (config, routing, orchestration, filters, observability, CLI, deployment).

## When To Use

- Validating or fixing Plano `config.yaml`
- Designing listener architecture (`model`, `prompt`, `agent`)
- Improving model/provider routing quality and fallback behavior
- Hardening filter chains and prompt guardrails
- Debugging routing with traces and CLI workflows
- Preparing deployment and production readiness checks

## How To Use

1. Classify the request by scope (single section vs. cross-cutting).
2. For focused work, prefer a section-specific skill (for example `plano-routing-model-selection`).
3. For broad work, apply this umbrella skill and reference section rules from `skills/AGENTS.md`.
4. Produce concrete edits first, then concise reasoning and validation steps.

## Operating Workflow

1. Identify the task area first: config, routing, orchestration, filters, observability, CLI, or deployment.
2. Apply the smallest correct change that satisfies the requested behavior.
3. Preserve security and reliability defaults:
   - `version: v0.3.0`
   - exactly one `default: true` model provider
   - secrets via `$ENV_VAR` substitution only
   - `host.docker.internal` for host services from inside Docker
   - guardrails before enrichment in filter chains
4. For debugging, prioritize traces over guesswork (`planoai up --with-tracing`, `planoai trace`).
5. Return concrete diffs and a short validation checklist.

## Response Style

- Prefer actionable edits over generic advice.
- Be explicit about why a config choice is correct.
- Call out risky patterns (hardcoded secrets, missing default provider, bad filter ordering).
- Keep examples minimal and production-viable.

## References

- Repo: https://github.com/katanemo/plano
- Full rulebook: `skills/AGENTS.md`
