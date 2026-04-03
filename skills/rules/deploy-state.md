---
title: Use PostgreSQL State Storage for Multi-Turn Conversations in Production
impact: HIGH
impactDescription: The default in-memory state storage loses all conversation history when the container restarts — production multi-turn agents require persistent PostgreSQL storage
tags: deployment, state, postgres, memory, multi-turn, production
---

## Use PostgreSQL State Storage for Multi-Turn Conversations in Production

`state_storage` enables Plano to maintain conversation context across requests. Without it, each request is stateless. The `memory` type works for development and testing — all state is lost on container restart. Use `postgres` for any production deployment where conversation continuity matters.

**Incorrect (memory storage in production):**

```yaml
version: v0.3.0

# Memory storage — all conversations lost on planoai down / container restart
state_storage:
  type: memory

listeners:
  - type: agent
    name: customer_support
    port: 8000
    router: plano_orchestrator_v1
    agents:
      - id: support_agent
        description: Customer support assistant with conversation history.
```

**Correct (PostgreSQL for production persistence):**

```yaml
version: v0.3.0

state_storage:
  type: postgres
  connection_string: "postgresql://${DB_USER}:${DB_PASS}@${DB_HOST}:5432/${DB_NAME}"

listeners:
  - type: agent
    name: customer_support
    port: 8000
    router: plano_orchestrator_v1
    agents:
      - id: support_agent
        description: Customer support assistant with access to full conversation history.

model_providers:
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    default: true
```

**Setting up PostgreSQL for local development:**

```bash
# Start PostgreSQL with Docker
docker run -d \
  --name plano-postgres \
  -e POSTGRES_USER=plano \
  -e POSTGRES_PASSWORD=devpassword \
  -e POSTGRES_DB=plano \
  -p 5432:5432 \
  postgres:16

# Set environment variables
export DB_USER=plano
export DB_PASS=devpassword
export DB_HOST=host.docker.internal   # Use host.docker.internal from inside Plano container
export DB_NAME=plano
```

**Production `.env` pattern:**

```bash
DB_USER=plano_prod
DB_PASS=<strong-random-password>
DB_HOST=your-rds-endpoint.amazonaws.com
DB_NAME=plano
```

Plano automatically creates its state tables on first startup. The `connection_string` supports all standard PostgreSQL connection parameters including SSL: `postgresql://user:pass@host:5432/db?sslmode=require`.

Reference: https://github.com/katanemo/archgw
