# Session Affinity with Redis — Multi-Replica Model Pinning

This demo shows Plano's **session affinity** (`X-Model-Affinity` header) backed by a **Redis session cache** instead of the default in-memory store.

## The Problem

By default, model affinity stores routing decisions in a per-process `HashMap`.
This works for single-instance deployments, but breaks when you run multiple
Plano replicas behind a load balancer:

```
Client ──► Load Balancer ──► Replica A  (session pinned here)
                         └──► Replica B  (knows nothing about the session)
```

A request that was pinned to `gpt-4o` on Replica A will be re-routed from
scratch on Replica B, defeating the purpose of affinity.

## The Solution

Plano's `session_cache` config key accepts a `type: redis` backend that is
shared across all replicas:

```yaml
routing:
  session_ttl_seconds: 300
  session_cache:
    type: redis
    url: redis://localhost:6379
```

All replicas read and write the same Redis keyspace. A session pinned on any
replica is immediately visible to all others.

## What to Look For

| What | Expected behaviour |
|------|--------------------|
| First request with a session ID | Plano routes normally (via Arch-Router) and writes the result to Redis (`SET session-id ... EX 300`) |
| Subsequent requests with the **same** session ID | Plano reads from Redis and skips the router — same model every time |
| Requests with a **different** session ID | Routed independently; may land on a different model |
| After `session_ttl_seconds` elapses | Redis key expires; next request re-routes and sets a new pin |
| `x-plano-pinned: true` response header | Tells you the response was served from the session cache |

## Architecture

```
Client
  │  X-Model-Affinity: my-session-id
  ▼
Plano (brightstaff)
  ├── GET redis://localhost:6379/my-session-id
  │     hit?  → return pinned model immediately (no Arch-Router call)
  │     miss? → call Arch-Router → SET key EX 300 → return routed model
  ▼
Redis  (shared across replicas)
```

## Prerequisites

| Requirement | Notes |
|-------------|-------|
| `planoai` CLI | `pip install planoai` |
| Docker + Docker Compose | For Redis and Jaeger |
| `OPENAI_API_KEY` | Required for routing model (Arch-Router) and downstream LLMs |
| Python 3.11+ | Only needed to run `verify_affinity.py` |

## Quick Start

```bash
# 1. Set your API key
export OPENAI_API_KEY=sk-...
# or copy and edit:
cp .env.example .env

# 2. Start Redis, Jaeger, and Plano
./run_demo.sh up

# 3. Verify session pinning works
python verify_affinity.py
```

## Manual Verification with curl

### Step 1 — Pin a session (first request sets the affinity)

```bash
curl -s http://localhost:12000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "x-model-affinity: my-session-abc" \
  -d '{"model":"openai/gpt-4o-mini","messages":[{"role":"user","content":"Write a short poem about the ocean."}]}' \
  | jq '{model, pinned: .x_plano_pinned}'
```

Expected output (first request — not yet pinned, Arch-Router picks the model):

```json
{
  "model": "openai/gpt-5.2",
  "pinned": null
}
```

### Step 2 — Confirm the pin is held on subsequent requests

```bash
for i in 1 2 3 4; do
  curl -s http://localhost:12000/v1/chat/completions \
    -H "Content-Type: application/json" \
    -H "x-model-affinity: my-session-abc" \
    -d "{\"model\":\"openai/gpt-4o-mini\",\"messages\":[{\"role\":\"user\",\"content\":\"Request $i\"}]}" \
    | jq -r '"\(.model)"'
done
```

Expected output (same model for every request):

```
openai/gpt-5.2
openai/gpt-5.2
openai/gpt-5.2
openai/gpt-5.2
```

### Step 3 — Inspect the Redis key directly

```bash
docker exec plano-session-redis redis-cli \
  GET my-session-abc | python3 -m json.tool
```

Expected output:

```json
{
  "model_name": "openai/gpt-5.2",
  "route_name": "deep_reasoning"
}
```

```bash
# Check the TTL (seconds remaining)
docker exec plano-session-redis redis-cli TTL my-session-abc
# e.g. 287
```

### Step 4 — Different sessions may get different models

```bash
for session in session-A session-B session-C; do
  model=$(curl -s http://localhost:12000/v1/chat/completions \
    -H "Content-Type: application/json" \
    -H "x-model-affinity: $session" \
    -d '{"model":"openai/gpt-4o-mini","messages":[{"role":"user","content":"Explain quantum entanglement in detail with equations."}]}' \
    | jq -r '.model')
  echo "$session -> $model"
done
```

Sessions with content matched to `deep_reasoning` will pin to `openai/gpt-5.2`;
sessions matched to `fast_responses` will pin to `openai/gpt-4o-mini`.

## Verification Script Output

Running `python verify_affinity.py` produces output like:

```
Plano endpoint : http://localhost:12000/v1/chat/completions
Sessions       : 3
Rounds/session : 4

============================================================
Phase 1: Requests WITHOUT X-Model-Affinity header
  (model may vary between requests — that is expected)
============================================================
  Request 1: model = openai/gpt-4o-mini
  Request 2: model = openai/gpt-5.2
  Request 3: model = openai/gpt-4o-mini
  Models seen across 3 requests: {'openai/gpt-4o-mini', 'openai/gpt-5.2'}

============================================================
Phase 2: Requests WITH X-Model-Affinity (session pinning)
  Each session should be pinned to exactly one model.
============================================================

  Session 'demo-session-001':
    Round 1: model = openai/gpt-4o-mini  [FIRST — sets affinity]
    Round 2: model = openai/gpt-4o-mini  [PINNED]
    Round 3: model = openai/gpt-4o-mini  [PINNED]
    Round 4: model = openai/gpt-4o-mini  [PINNED]

  Session 'demo-session-002':
    Round 1: model = openai/gpt-5.2       [FIRST — sets affinity]
    Round 2: model = openai/gpt-5.2       [PINNED]
    Round 3: model = openai/gpt-5.2       [PINNED]
    Round 4: model = openai/gpt-5.2       [PINNED]

  Session 'demo-session-003':
    Round 1: model = openai/gpt-4o-mini  [FIRST — sets affinity]
    Round 2: model = openai/gpt-4o-mini  [PINNED]
    Round 3: model = openai/gpt-4o-mini  [PINNED]
    Round 4: model = openai/gpt-4o-mini  [PINNED]

============================================================
Results
============================================================
  PASS  demo-session-001 -> always routed to 'openai/gpt-4o-mini'
  PASS  demo-session-002 -> always routed to 'openai/gpt-5.2'
  PASS  demo-session-003 -> always routed to 'openai/gpt-4o-mini'

All sessions were pinned consistently.
Redis session cache is working correctly.
```

## Observability

Open Jaeger at **http://localhost:16686** and select service `plano`.

- Requests **without** affinity: look for a span to the Arch-Router service
- Requests **with** affinity (pinned): the Arch-Router span will be absent —
  the decision was served from Redis without calling the router at all

This is the clearest observable signal that the cache is working: pinned
requests are noticeably faster and produce fewer spans.

## Switching to the In-Memory Backend

To compare against the default in-memory backend, change `config.yaml`:

```yaml
routing:
  session_ttl_seconds: 300
  session_cache:
    type: memory     # ← change this
```

In-memory mode does **not** require Redis and works identically for a
single Plano process. The difference only becomes visible when you run
multiple replicas.

## Teardown

```bash
./run_demo.sh down
```

This stops Plano, Redis, and Jaeger.
