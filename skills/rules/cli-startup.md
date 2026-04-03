---
title: Follow the `planoai up` Validation Workflow Before Debugging Runtime Issues
impact: HIGH
impactDescription: `planoai up` validates config, checks API keys, and health-checks all listeners — skipping this diagnostic information leads to unnecessary debugging of container or network issues
tags: cli, startup, validation, debugging, workflow
---

## Follow the `planoai up` Validation Workflow Before Debugging Runtime Issues

`planoai up` is the entry point for running Plano. It performs sequential checks before the container starts: schema validation, API key presence check, container startup, and health checks on all configured listener ports. Understanding what each failure stage means prevents chasing the wrong root cause.

**Validation stages and failure signals:**

```
Stage 1: Schema validation        → "config.yaml: invalid against schema"
Stage 2: API key check            → "Missing required environment variables: OPENAI_API_KEY"
Stage 3: Container start          → "Docker daemon not running" or image pull errors
Stage 4: Health check (/healthz)  → "Listener not healthy after 120s" (timeout)
```

**Development startup workflow:**

```bash
# Standard startup — config.yaml in current directory
planoai up

# Explicit config file path
planoai up my-config.yaml

# Start in foreground to see all logs immediately (great for debugging)
planoai up config.yaml --foreground

# Start with built-in OTEL trace collector
planoai up config.yaml --with-tracing

# Enable verbose logging for debugging routing decisions
LOG_LEVEL=debug planoai up config.yaml --foreground
```

**Checking what's running:**

```bash
# Stream recent logs (last N lines, then exit)
planoai logs

# Follow logs in real-time
planoai logs --follow

# Include Envoy/gateway debug messages
planoai logs --debug --follow
```

**Stopping and restarting after config changes:**

```bash
# Stop the current container
planoai down

# Restart with updated config
planoai up config.yaml
```

**Common failure patterns:**

```bash
# API key missing — check your .env file or shell environment
export OPENAI_API_KEY=sk-proj-...
planoai up config.yaml

# Health check timeout — listener port may conflict
# Check if another process uses port 12000
lsof -i :12000

# Container fails to start — verify Docker daemon is running
docker ps
```

`planoai down` fully stops and removes the Plano container. Always run `planoai down` before `planoai up` when changing config to avoid stale container state.

Reference: https://github.com/katanemo/archgw
