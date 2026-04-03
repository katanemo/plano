---
title: Verify Listener Health Before Sending Requests
impact: MEDIUM
impactDescription: Sending requests to Plano before listeners are healthy results in connection refused errors that look like application bugs — always confirm health before testing
tags: deployment, health-checks, readiness, debugging
---

## Verify Listener Health Before Sending Requests

Each Plano listener exposes a `/healthz` HTTP endpoint. `planoai up` automatically health-checks all listeners during startup (120s timeout), but in CI/CD pipelines, custom scripts, or when troubleshooting, you may need to check health manually.

**Health check endpoints:**

```bash
# Check model listener health (port from your config)
curl -f http://localhost:12000/healthz
# Returns 200 OK when healthy

# Check prompt listener
curl -f http://localhost:10000/healthz

# Check agent listener
curl -f http://localhost:8000/healthz
```

**Polling health in scripts (CI/CD pattern):**

```bash
#!/bin/bash
# wait-for-plano.sh

LISTENER_PORT=${1:-12000}
MAX_WAIT=120
INTERVAL=2
elapsed=0

echo "Waiting for Plano listener on port $LISTENER_PORT..."

until curl -sf "http://localhost:$LISTENER_PORT/healthz" > /dev/null; do
  if [ $elapsed -ge $MAX_WAIT ]; then
    echo "ERROR: Plano listener not healthy after ${MAX_WAIT}s"
    planoai logs --debug
    exit 1
  fi
  sleep $INTERVAL
  elapsed=$((elapsed + INTERVAL))
done

echo "Plano listener healthy after ${elapsed}s"
```

**Docker Compose health check:**

```yaml
# docker-compose.yml for services that depend on Plano
services:
  plano:
    image: katanemo/plano:latest
    # Plano is managed by planoai, not directly via compose in most setups
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:12000/healthz"]
      interval: 5s
      timeout: 3s
      retries: 24
      start_period: 10s

  my-agent:
    image: my-agent:latest
    depends_on:
      plano:
        condition: service_healthy
```

**Debug unhealthy listeners:**

```bash
# See startup logs
planoai logs --debug

# Check if port is already in use
lsof -i :12000

# Check container status
docker ps -a --filter name=plano

# Restart from scratch
planoai down && planoai up config.yaml --foreground
```

Reference: https://github.com/katanemo/archgw
