# Model Listener Filter Chain Demo

Run content-safety filters on direct LLM requests — no agent layer required.

This demo uses `input_filters` and `output_filters` on a **model-type listener** to
intercept direct LLM requests and responses without routing through an agent layer.
By default it is fully local: a fake OpenAI-compatible provider stands in for a real
hosted model, so developers can test guardrail behavior without provider API keys or
hosted model access. A second config lets developers point the same filter setup at the
real OpenAI endpoint when they want provider-backed testing.
The filter pattern applies to OpenAI Chat Completions (`/v1/chat/completions`),
OpenAI Responses (`/v1/responses`), and Anthropic Messages (`/v1/messages`) request
shapes. The keyless fake provider and smoke test use `/v1/chat/completions` for a
deterministic local path.

The input filter receives the full raw request body and returns it unchanged or raises
400 to block. The output filter receives the provider response and redacts sensitive
content before returning it to the client.

## Files

- `config.yaml` runs the default keyless path with the local fake provider.
- `config.openai.yaml` runs the same filters against OpenAI.
- `docker-compose.yaml` starts the local demo without requiring provider credentials.
- `docker-compose.openai.yaml` mounts `config.openai.yaml` and requires `OPENAI_API_KEY`
  for provider-backed testing.
- `test.sh` runs the Docker smoke test through Plano.
- `test_services.py` runs service-level regression tests without Docker.

## Architecture

```
Client ──► Plano (model listener :12000)
               │
               ├─ input_filters: content_guard ──► Block / Allow
               │
               ├─ model_provider: fake-provider (default) or OpenAI (optional)
               │
               └─ output_filters: output_redactor ──► Redact / Allow
```

## Quick Start

```bash
# 1. Start services
docker compose up --build

# 2. Run tests (in another terminal)
bash test.sh
```

The test script verifies three behaviors:

- safe requests reach the local fake provider and return a normal chat-completion response
- unsafe requests are blocked by the input filter before reaching the provider
- sensitive provider output is redacted by the output filter before the client receives it

You can also run the service-level tests without Docker:

```bash
uv run --with pytest --with fastapi --with httpx --with pydantic \
  python -m pytest demos/filter_chains/model_listener_filter/test_services.py -q
```

## Validate Locally

From this directory, validate the default keyless compose path:

```bash
docker compose config
```

Validate that the OpenAI path fails early when the API key is missing:

```bash
docker compose -f docker-compose.yaml -f docker-compose.openai.yaml config
```

Expected error:

```text
OPENAI_API_KEY environment variable is required but not set
```

Then confirm the OpenAI compose path renders when a key is provided:

```bash
OPENAI_API_KEY=dummy docker compose -f docker-compose.yaml -f docker-compose.openai.yaml config
```

Run the full local smoke test:

```bash
docker compose down
docker compose up --build -d
bash test.sh
docker compose down
```

## Test With Real OpenAI

The default `config.yaml` uses the local fake provider. To run the same model-listener
input and output filters against OpenAI, use the OpenAI compose override:

```bash
export OPENAI_API_KEY=sk-...
docker compose -f docker-compose.yaml -f docker-compose.openai.yaml up --build
```

The fake-provider service may still start because it is part of the shared compose file,
but Plano will not route traffic to it when `config.openai.yaml` is mounted.

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

**Redacted provider response:**

```bash
curl http://localhost:12000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "messages": [{"role": "user", "content": "Please return the secret marker"}],
    "stream": false
  }'
```

The fake provider emits `SECRET_TOKEN`; the output filter redacts it to `[REDACTED]`.

## Why This Helps Developers

Model-listener filters are guardrails for applications that call Plano as a transparent
LLM gateway. A local, deterministic demo helps developers verify filter wiring before
using real providers:

- config mistakes are caught early instead of silently bypassing guardrails
- teams can test request blocking and response redaction in CI without secrets
- contributors can reproduce filter behavior without external model availability
- application code does not need an extra passthrough agent just to run policy checks

## Tracing

Open [Jaeger UI](http://localhost:16686) to see distributed traces for both allowed and blocked requests.
