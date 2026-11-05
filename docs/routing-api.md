# Plano Routing API — Request & Response Format

## Overview

Plano intercepts LLM requests and routes them to the best available model based on semantic intent and live cost/latency data. The developer sends a standard OpenAI-compatible request with an optional `routing_preferences` field. Plano returns an ordered list of candidate models; the client uses the first and falls back to the next on 429 or 5xx errors.

---

## Request Format

Standard OpenAI chat completion body. The only addition is the optional `routing_preferences` field, which is stripped before the request is forwarded upstream.

```json
POST /v1/chat/completions
{
  "model": "openai/gpt-4o-mini",
  "messages": [
    {"role": "user", "content": "write a sorting algorithm in Python"}
  ],
  "routing_preferences": [
    {
      "name": "code generation",
      "description": "generating new code snippets",
      "models": ["anthropic/claude-sonnet-4-6", "openai/gpt-4o", "openai/gpt-4o-mini"]
    },
    {
      "name": "general questions",
      "description": "casual conversation and simple queries",
      "models": ["openai/gpt-4o-mini"]
    }
  ]
}
```

### `routing_preferences` fields


| Field         | Type     | Required | Description                                                                                 |
| ------------- | -------- | -------- | ------------------------------------------------------------------------------------------- |
| `name`        | string   | yes      | Route identifier. Must match the LLM router's route classification.                         |
| `description` | string   | yes      | Natural language description used by the router to match user intent.                       |
| `models`      | string[] | yes      | Ordered candidate pool. At least one entry required. Must be declared in `model_providers`. |


### Notes

- `routing_preferences` is **optional**. If omitted, the config-defined preferences are used.
- If provided in the request body, it **overrides** the config for that single request only.
- `model` is still required and is used as the fallback if no route is matched.

---

## Response Format

```json
{
  "models": [
    "anthropic/claude-sonnet-4-6",
    "openai/gpt-4o",
    "openai/gpt-4o-mini"
  ],
  "route": "code generation",
  "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736"
}
```

### Fields


| Field      | Type          | Description                                                                                             |
| ---------- | ------------- | ------------------------------------------------------------------------------------------------------- |
| `models`   | string[]      | Ranked model list. Use `models[0]` as primary; retry with `models[1]` on 429/5xx, and so on.            |
| `route`    | string | null | Name of the matched route. `null` if no route matched — client should use the original request `model`. |
| `trace_id` | string        | Trace ID for distributed tracing and observability.                                                     |


---

## Client Usage Pattern

```python
response = plano.routing_decision(request)
models = response["models"]

for model in models:
    try:
        result = call_llm(model, messages)
        break  # success — stop trying
    except (RateLimitError, ServerError):
        continue  # try next model in the ranked list
```

---

## Configuration (set by platform/ops team)

Requires `version: v0.4.0` or above. Models listed under `routing_preferences` must be declared in `model_providers`.

```yaml
version: v0.4.0

model_providers:
  - model: anthropic/claude-sonnet-4-6
    access_key: $ANTHROPIC_API_KEY
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
  - model: openai/gpt-4o-mini
    access_key: $OPENAI_API_KEY
    default: true

routing_preferences:
  - name: code generation
    description: generating new code snippets or boilerplate
    models:
      - anthropic/claude-sonnet-4-6
      - openai/gpt-4o

  - name: general questions
    description: casual conversation and simple queries
    models:
      - openai/gpt-4o-mini
      - openai/gpt-4o
```

---

## Per-request routing (default)

Applies when `route_on_user_only` is `false` (the default). Every request is routed independently, including loop continuations. A single user turn may therefore be served by multiple models. For example, a lightweight model handling routine tool-orchestration iterations while a stronger model is selected for a complex reasoning step or the final synthesis.

### When to use

Use per-request routing for:

- **Best model per step.** Each step in a turn is served by the model best matched to its difficulty or specialty.
- **Cost efficiency.** Simple steps go to smaller models; only hard steps use expensive ones.
- **Escalation on failure.** A struggling model can be swapped out for the remainder of the turn.
- **Capacity flexibility.** Each request can be placed wherever capacity exists, with no pinning constraint.

## Turn-level routing

Applies when `route_on_user_only` is `true`. The router selects a model once per user turn and pins the remainder of the agentic loop to it.

```yaml
routing:
  route_on_user_only: true
```

### When to use

Use turn-level routing for:

- **Plan consistency.** One model carries its own reasoning and plan through the entire loop.
- **Cache locality.** Loop iterations reuse the shared prompt prefix cache; no switch-induced misses.
- **No state translation.** Avoids stripping or converting model-specific artifacts (e.g., signed thinking blocks).
- **Simpler parameter handling.** Engine-native parameters are resolved once per turn, not re-mapped per request.

### When routing occurs

A routing decision is made when the incoming request represents the **start of a new user turn**: the last normalized message has `role: "user"` and still carries text once harness-injected envelopes are removed. That is genuine user text, not a tool result being fed back into an in-flight loop.

Across client APIs that means:

- OpenAI Chat: the last message is `role: "user"`
- Anthropic: the last content is user text (a `tool_result`-only turn normalizes to `role: "tool"`)
- Responses: the last item is not a `function_call_output` or `custom_tool_call_output` (Codex registers custom tools by default, so its steps use the latter)
- Bedrock Converse: the last message carries user text, not only `toolResult` blocks

A new user message always re-routes, including Anthropic packing new user text alongside a `tool_result`.

An empty user message, or one containing only Claude Code's `<system-reminder>` / `<user-prompt-submit-hook>` envelopes, is harness output rather than an utterance, so the loop stays pinned. An attachment with no caption (a pasted image) is user input and does route.

**Known limitation.** Agent frameworks that feed tool output back as plain user prose — ReAct's `Observation: ...`, for example — are indistinguishable from a real user message on the wire, so each step re-routes. Send tool output as `role: "tool"` (or Anthropic `tool_result` / Responses `function_call_output`) to get turn-level pinning.

### When routing is skipped (sticky)

Routing is skipped and the previously selected model is reused when the request is a **continuation of an in-flight agentic loop** — the last normalized message is not a user turn (tool results, assistant steps, unresolved `tool_use`, or an empty / envelope-only user message). The request is pinned to the model recorded for the current turn.

A step still re-routes when that prior decision can no longer be identified — the session went cold, the system prompt or tool set changed, or the request is on a different model (Claude Code's `ANTHROPIC_SMALL_FAST_MODEL` calls route independently of the main loop).

Skips are observable: the `plano.routing.skipped` span attribute and the `brightstaff_router_skips_total` counter.

## Model Affinity

The user-turn check covers a single query. To keep a whole *conversation* on one model — across user turns — send an `X-Model-Affinity` header. Without it, Plano derives an implicit session key from the stable prompt prefix (system + tools + first user message), which is what makes replay work header-free.

**Use one id per conversation, not one per client session.** A session key holds a single binding, so if side calls (a side chat, a summarizer, a subagent) share the main loop's id, whichever ran last owns the binding. The lane and prefix guards keep the side call from being pinned to the main loop's model, but the loop's own pin is evicted, and its next continuation re-routes. Give side calls their own id, or omit the header and let implicit affinity separate them by prompt prefix.

```json
POST /v1/chat/completions
X-Model-Affinity: a1b2c3d4-5678-...

{
  "model": "openai/gpt-4o-mini",
  "messages": [...]
}
```

The routing decision endpoint also supports model affinity:

```json
POST /routing/v1/chat/completions
X-Model-Affinity: a1b2c3d4-5678-...
```

Response when pinned:

```json
{
  "models": ["anthropic/claude-sonnet-4-6"],
  "route": "code generation",
  "trace_id": "...",
  "session_id": "a1b2c3d4-5678-...",
  "pinned": true,
  "switched": false,
  "prefix_hash": "3f9a1c0e7b2d4a65"
}
```

`pinned` reports that the session was warm on its bound model, so callers can treat it as a signal to keep that provider's cache warm.

`prefix_hash` is the same 16-hex value the proxy sends upstream as `x-plano-prefix-hash`. Callers that dispatch inference to self-hosted multi-replica backends can key a consistent-hash load-balancing policy on it (Envoy ring-hash or equivalent), so requests sharing a prompt prefix land on the replica holding its warm KV cache, mirroring Plano's own `prefix_affinity` clusters. It is omitted when the request sent `X-Plano-Cache: off` or has no anchorable prompt.

The decision endpoint applies the same loop handling as the proxy, so a client polling it between tool calls gets a stable answer for the whole turn.

Configure TTL and cache size:

```yaml
routing:
  session_ttl_seconds: 600    # default: 10 min
  session_max_entries: 10000  # upper limit
```

---

## Version Requirements


| Version    | Top-level `routing_preferences`        |
| ---------- | -------------------------------------- |
| `< v0.4.0` | Not allowed — startup error if present |
| `v0.4.0+`  | Supported (required for model routing) |
