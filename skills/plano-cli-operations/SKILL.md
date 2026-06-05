---
name: plano-cli-operations
description: Apply Plano CLI best practices. Use for startup troubleshooting, cli_agent workflows, and template-based project bootstrapping.
license: Apache-2.0
metadata:
  author: katanemo
  version: "1.0.0"
---

# Plano CLI Operations

Use this skill when the task is primarily operational and CLI-driven.

## When To Use

- "Fix `planoai up` failures"
- "Use `planoai cli_agent` with coding agents"
- "Bootstrap a project with `planoai init` templates"

## Apply These Rules

- `cli-startup`
- `cli-agent`
- `cli-init`

## Execution Checklist

1. Follow startup validation order before deep debugging.
2. Use `cli_agent` to route coding-agent traffic through Plano.
3. Start from templates for reliable first-time setup.
4. Provide a compact runbook with exact CLI commands.
