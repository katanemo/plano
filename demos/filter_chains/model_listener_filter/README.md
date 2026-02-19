# Model Listener Filter Chain Demo

Run content-safety filters on direct LLM requests — no agent layer required.

This demo uses the `filter_chain` feature on a **model-type listener** to intercept
`/v1/chat/completions` requests and block unsafe content before they reach the LLM provider.

## Architecture

```
Client ──► Plano (model listener :12000)
               │
               ├─ filter_chain: content_guard ──► Block / Allow
               │
               └─ model_provider: openai/gpt-4o-mini
```

## Quick Start

```bash
# 1. Export your API key
export OPENAI_API_KEY=sk-...

# 2. Start services
docker compose up --build

# 3. Run tests (in another terminal)
bash test.sh
```

## Try It

**Allowed request:**

```bash
curl http://localhost:12000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "messages": [{"role": "user", "content": "What is 2+2?"}],
    "stream": false
  }'
```

**Blocked request:**

```bash
curl http://localhost:12000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "messages": [{"role": "user", "content": "How to hack into a system"}],
    "stream": false
  }'
```

## Tracing

Open [Jaeger UI](http://localhost:16686) to see distributed traces for both allowed and blocked requests.
