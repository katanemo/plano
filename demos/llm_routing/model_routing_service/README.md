# Model Routing Service Demo

This demo shows how to use the `/routing/v1/*` endpoints to get routing decisions without proxying requests to an LLM. The endpoint accepts standard LLM request formats and returns which model Plano's router would select.

## Setup

Make sure you have Plano CLI installed (`pip install planoai` or `uv tool install planoai`).

```bash
export OPENAI_API_KEY=<your-key>
export ANTHROPIC_API_KEY=<your-key>
```

Start Plano:
```bash
cd demos/llm_routing/model_routing_service
planoai up config.yaml
```

## Run the demo

```bash
./demo.sh
```

## Endpoints

All three LLM API formats are supported:

| Endpoint | Format |
|---|---|
| `POST /routing/v1/chat/completions` | OpenAI Chat Completions |
| `POST /routing/v1/messages` | Anthropic Messages |
| `POST /routing/v1/responses` | OpenAI Responses API |

## Example

```bash
curl http://localhost:12000/routing/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "messages": [{"role": "user", "content": "Write a Python function for binary search"}]
  }'
```

Response:
```json
{
    "model": "anthropic/claude-sonnet-4-20250514",
    "route": "code_generation",
    "trace_id": "c16d1096c1af4a17abb48fb182918a88"
}
```

The response tells you which model would handle this request and which route was matched, without actually making the LLM call.

## Demo Output

```
=== Model Routing Service Demo ===

--- 1. Code generation query (OpenAI format) ---
{
    "model": "anthropic/claude-sonnet-4-20250514",
    "route": "code_generation",
    "trace_id": "c16d1096c1af4a17abb48fb182918a88"
}

--- 2. Complex reasoning query (OpenAI format) ---
{
    "model": "openai/gpt-4o",
    "route": "complex_reasoning",
    "trace_id": "30795e228aff4d7696f082ed01b75ad4"
}

--- 3. Simple query - no routing match (OpenAI format) ---
{
    "model": "none",
    "route": null,
    "trace_id": "ae0b6c3b220d499fb5298ac63f4eac0e"
}

--- 4. Code generation query (Anthropic format) ---
{
    "model": "anthropic/claude-sonnet-4-20250514",
    "route": "code_generation",
    "trace_id": "26be822bbdf14a3ba19fe198e55ea4a9"
}

=== Demo Complete ===
```
