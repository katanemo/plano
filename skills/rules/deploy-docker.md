---
title: Understand Plano's Docker Network Topology for Agent URL Configuration
impact: HIGH
impactDescription: Using `localhost` for agent URLs inside Docker always fails — Plano runs in a container and cannot reach host services via localhost
tags: deployment, docker, networking, agents, urls
---

## Understand Plano's Docker Network Topology for Agent URL Configuration

Plano runs inside a Docker container managed by `planoai up`. Services running on your host machine (agent servers, filter servers, databases) are not accessible as `localhost` from inside the container. Use Docker's special hostname `host.docker.internal` to reach host services.

**Docker network rules:**
- `localhost` / `127.0.0.1` inside the container → Plano's own container (not your host)
- `host.docker.internal` → Your host machine's loopback interface
- Container name or `docker network` hostname → Other Docker containers
- External domain / IP → Reachable if Docker has network access

**Incorrect (using localhost — agent unreachable from inside container):**

```yaml
version: v0.3.0

agents:
  - id: weather_agent
    url: http://localhost:8001       # Wrong: this is Plano's own container

  - id: flight_agent
    url: http://127.0.0.1:8002      # Wrong: same issue

filters:
  - id: input_guards
    url: http://localhost:10500      # Wrong: filter server unreachable
```

**Correct (using host.docker.internal for host-side services):**

```yaml
version: v0.3.0

agents:
  - id: weather_agent
    url: http://host.docker.internal:8001    # Correct: reaches host port 8001

  - id: flight_agent
    url: http://host.docker.internal:8002    # Correct: reaches host port 8002

filters:
  - id: input_guards
    url: http://host.docker.internal:10500   # Correct: reaches filter server on host

endpoints:
  internal_api:
    endpoint: host.docker.internal            # Correct for internal API on host
    protocol: http
```

**Production deployment patterns:**

```yaml
# Kubernetes / Docker Compose — use service names
agents:
  - id: weather_agent
    url: http://weather-service:8001    # Kubernetes service DNS

# External cloud services — use full domain
agents:
  - id: cloud_agent
    url: https://my-agent.us-east-1.amazonaws.com/v1

# Custom TLS (self-signed or internal CA)
overrides:
  upstream_tls_ca_path: /etc/ssl/certs/internal-ca.pem
```

**Ports exposed by Plano's container:**
- All `port` values from your `listeners` blocks are automatically mapped
- `9901` — Envoy admin interface (for advanced debugging)
- `12001` — Plano internal management API

Reference: https://github.com/katanemo/archgw
