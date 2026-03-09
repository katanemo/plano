---
name: plano-deployment-security
description: Apply Plano deployment and production security practices. Use for Docker networking, state storage choices, readiness checks, and environment-based secret handling.
license: Apache-2.0
metadata:
  author: katanemo
  version: "1.0.0"
---

# Plano Deployment and Security

Use this skill to harden production deployments and reduce runtime surprises.

## When To Use

- "Fix unreachable agents in Docker"
- "Configure persistent conversation state"
- "Add readiness and health checks"
- "Prepare production deployment checklist"

## Apply These Rules

- `deploy-docker`
- `deploy-state`
- `deploy-health`

## Execution Checklist

1. Use `host.docker.internal` for host-side services from inside Plano container.
2. Prefer PostgreSQL state storage for production multi-turn workloads.
3. Verify `/healthz` before traffic or CI assertions.
4. Ensure secrets remain environment-based, never hardcoded.
5. Return deployment checks with failure-mode diagnostics.
