---
name: plano-config-fundamentals
description: Validate and fix Plano config fundamentals. Use for config versioning, listener types, provider registration, secrets handling, and startup validation failures.
license: Apache-2.0
metadata:
  author: katanemo
  version: "1.0.0"
---

# Plano Configuration Fundamentals

Use this skill for foundational `config.yaml` correctness.

## When To Use

- "Validate this Plano config"
- "Fix startup config errors"
- "Check listeners/providers/secrets"
- "Why does `planoai up` fail schema validation?"

## Apply These Rules

- `config-version`
- `config-listeners`
- `config-providers`
- `config-secrets`

## Execution Checklist

1. Ensure `version: v0.3.0` is present.
2. Confirm listener type matches intended architecture.
3. Verify provider names/interfaces and exactly one default provider.
4. Replace hardcoded secrets with `$ENV_VAR` substitution.
5. Return minimal patch and a `planoai up` verification plan.
