---
title: Use `planoai cli_agent` to Connect Claude Code Through Plano
impact: MEDIUM-HIGH
impactDescription: Running Claude Code directly against provider APIs bypasses Plano's routing, observability, and guardrails — cli_agent routes all Claude Code traffic through your configured Plano instance
tags: cli, cli-agent, claude, coding-agent, integration
---

## Use `planoai cli_agent` to Connect Claude Code Through Plano

`planoai cli_agent` starts a Claude Code session that routes all LLM traffic through your running Plano instance instead of directly to Anthropic. This gives you routing preferences, model aliases, tracing, and guardrails for your coding agent workflows — making Claude Code a first-class citizen of your Plano configuration.

**Prerequisites:**

```bash
# 1. Plano must be running with a model listener
planoai up config.yaml

# 2. ANTHROPIC_API_KEY must be set (Claude Code uses it for auth)
export ANTHROPIC_API_KEY=sk-ant-...
```

**Starting the CLI agent:**

```bash
# Start CLI agent using config.yaml in current directory
planoai cli_agent claude

# Use a specific config file
planoai cli_agent claude config.yaml

# Use a config in a different directory
planoai cli_agent claude --path /path/to/project
```

**Recommended config for Claude Code routing:**

```yaml
version: v0.3.0

listeners:
  - type: model
    name: claude_code_router
    port: 12000

model_providers:
  - model: anthropic/claude-sonnet-4-20250514
    access_key: $ANTHROPIC_API_KEY
    default: true
    routing_preferences:
      - name: general coding
        description: >
          Writing code, debugging, code review, explaining concepts,
          answering programming questions, general development tasks.

  - model: anthropic/claude-opus-4-6
    access_key: $ANTHROPIC_API_KEY
    routing_preferences:
      - name: complex architecture
        description: >
          System design, complex refactoring across many files,
          architectural decisions, performance optimization, security audits.

model_aliases:
  claude.fast.v1:
    target: claude-sonnet-4-20250514
  claude.smart.v1:
    target: claude-opus-4-6

tracing:
  random_sampling: 100
  trace_arch_internal: true

overrides:
  upstream_connect_timeout: "10s"
```

**What happens when cli_agent runs:**

1. Reads your config.yaml to find the model listener port
2. Configures Claude Code to use `http://localhost:<port>` as its API endpoint
3. Starts a Claude Code session in your terminal
4. All Claude Code LLM calls flow through Plano — routing, tracing, and guardrails apply

After your session, use `planoai trace` to inspect every LLM call Claude Code made, which model was selected, and why.

Reference: [https://github.com/katanemo/archgw](https://github.com/katanemo/archgw)
