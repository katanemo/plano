# Data Contracts — Inter-Component Communication

This document defines the contracts between Plano's components: custom HTTP headers, internal API formats, streaming protocols, and Envoy routing conventions. Breaking any of these contracts will cause silent routing failures.

---

## 1. Custom Header Protocol

All custom headers are defined in `common/src/consts.rs`. This is the **single source of truth** — if a header name appears in `envoy.template.yaml` or Brightstaff code, it must match the constant in `consts.rs`.

### Routing Headers (Envoy-critical)

These headers are used in Envoy's `route_config` for cluster selection. Changing them requires updating `envoy.template.yaml`.

| Header | Constant | Set By | Read By | Value Format | Purpose |
|---|---|---|---|---|---|
| `x-arch-llm-provider` | `ARCH_ROUTING_HEADER` | WASM filters | Envoy routes | Provider slug (e.g., `openai`, `anthropic`) | Selects the LLM provider cluster in Envoy |
| `x-arch-upstream` | `ARCH_UPSTREAM_HOST_HEADER` | WASM filters, Brightstaff | Envoy routes | Cluster name (e.g., agent endpoint name) | Routes to a specific upstream cluster |
| `x-arch-llm-provider-hint` | `ARCH_PROVIDER_HINT_HEADER` | Brightstaff | llm_gateway | `provider/model` (e.g., `openai/gpt-4`) | Hints which provider+model to use |
| `x-arch-agent-listener-name` | — | Envoy (set in route config) | Brightstaff | Listener name string | Identifies which agent listener a request arrived on |

### Internal State Headers (WASM filter internal)

These headers pass state between the prompt_gateway filter's request/response phases or between prompt_gateway and the function calling service.

| Header | Constant | Set By | Read By | Value Format | Purpose |
|---|---|---|---|---|---|
| `x-arch-state` | `X_ARCH_STATE_HEADER` | prompt_gateway | prompt_gateway | Base64-encoded JSON (`ArchState`) | Multi-turn conversation state across filter invocations |
| `x-arch-tool-call-message` | `X_ARCH_TOOL_CALL` | prompt_gateway | prompt_gateway | JSON string | Tool call metadata for API orchestration |
| `x-arch-api-response-message` | `X_ARCH_API_RESPONSE` | prompt_gateway | prompt_gateway | JSON string | Developer API response data |
| `x-arch-fc-model-response` | `X_ARCH_FC_MODEL_RESPONSE` | prompt_gateway | prompt_gateway | JSON string | Raw Arch-Function model response |
| `x-arch-llm-route` | `LLM_ROUTE_HEADER` | Brightstaff | llm_gateway | Route name string | LLM route decision result |

### Signaling Headers

| Header | Constant | Set By | Read By | Purpose |
|---|---|---|---|---|
| `x-arch-streaming-request` | `ARCH_IS_STREAMING_HEADER` | Brightstaff | llm_gateway | Indicates the request is streaming mode |
| `x-arch-ratelimit-selector` | `RATELIMIT_SELECTOR_HEADER_KEY` | Client / Envoy | llm_gateway | Key for per-tenant rate limit partitioning |

### Standard Headers Used

| Header | Constant | Purpose |
|---|---|---|
| `x-request-id` | `REQUEST_ID_HEADER` | Request tracing (set by Envoy or caller) |
| `x-envoy-original-path` | `ENVOY_ORIGINAL_PATH_HEADER` | Original path before Envoy rewrites |
| `x-envoy-max-retries` | `ENVOY_RETRY_HEADER` | Retry count for Envoy's retry policy |
| `traceparent` | `TRACE_PARENT_HEADER` | W3C Trace Context for OpenTelemetry |

---

## 2. Internal Cluster Names

Defined in `consts.rs` and referenced in `envoy.template.yaml`:

| Constant | Value | Target | Purpose |
|---|---|---|---|
| `MODEL_SERVER_NAME` | `"bright_staff"` | localhost:9091 | Brightstaff service |
| `ARCH_INTERNAL_CLUSTER_NAME` | `"arch_internal"` | localhost:11000 | Outbound API router |
| `ARCH_FC_CLUSTER` | `"arch"` | archfc.katanemo.dev:443 | Katanemo Arch-Function model |

Additional clusters generated from config:
- `arch_prompt_gateway_listener` → localhost:10001
- `arch_listener_llm` → localhost:12001
- Per-provider clusters (e.g., `openai`, `anthropic`, `gemini`) from `envoy.template.yaml`
- Per-agent/endpoint clusters from user config

---

## 3. Internal API Formats

### Brightstaff → Envoy (LLM requests via :12001)

Brightstaff sends OpenAI-compatible `ChatCompletionsRequest` JSON to `localhost:12001` with:
- `x-arch-llm-provider-hint: <provider>/<model>` to select the provider
- `x-arch-is-streaming: true/false` to indicate streaming
- Standard `Content-Type: application/json`
- `traceparent` for distributed tracing

The `llm_gateway` WASM filter at :12001 transforms the request to the target provider's format.

### Brightstaff → Envoy (Agent/API requests via :11000)

Brightstaff sends requests to `localhost:11000` with:
- `x-arch-upstream-host: <cluster_name>` to route to the target agent/API
- `x-envoy-max-retries: 3` for resilience

**MCP Agent Protocol:**
```
POST /  (with x-arch-upstream-host)
Content-Type: application/json

# Step 1: Initialize
{"jsonrpc":"2.0","method":"initialize","id":"<uuid>","params":{...}}

# Step 2: Initialized notification
{"jsonrpc":"2.0","method":"notifications/initialized"}

# Step 3: Tool call
{"jsonrpc":"2.0","method":"tools/call","id":"<uuid>","params":{"name":"<tool>","arguments":{...}}}
```

**HTTP Agent Protocol:**
```
POST /  (with x-arch-upstream-host)
Content-Type: application/json

[{"role":"user","content":"..."},{"role":"assistant","content":"..."}]
```
Response: Array of messages.

### prompt_gateway → Arch-Function (/function_calling)

```
POST /function_calling
Content-Type: application/json

{
  "messages": [...],
  "tools": [...],
  "model": "Arch-Function",
  "stream": false,
  "metadata": {"raw_response": true, "logprobs": true}
}
```

Response contains `tool_calls`, `response`, or `clarification` in the assistant message content (JSON string).

---

## 4. Streaming Protocol

### SSE (Server-Sent Events) — Standard LLM Streaming

All streaming LLM responses use SSE format:
```
data: {"id":"...","choices":[...]}\n\n
data: {"id":"...","choices":[...]}\n\n
data: [DONE]\n\n
```

**Important:** SSE events can be split across HTTP chunks. The `llm_gateway` uses `SseStreamBuffer` and `SseChunkProcessor` (from `hermesllm`) to reassemble events across chunk boundaries before processing.

### Bedrock Binary Streaming

Amazon Bedrock uses AWS Event Stream binary protocol instead of SSE. The `BedrockBinaryFrameDecoder` in `hermesllm` handles decoding.

### Brightstaff Streaming

Brightstaff uses `tokio::sync::mpsc` channels to stream responses:
1. Spawns a background task to read from upstream (via `reqwest`)
2. Parses SSE events, optionally transforms them
3. Sends chunks through the mpsc channel
4. Axum's `StreamBody` delivers to the client

---

## 5. Configuration Injection

### WASM Filter Configuration

Envoy injects config into WASM filters via the `configuration` field in the filter definition:

- **prompt_gateway** receives: `prompt_targets`, `prompt_guards`, `system_prompt`, `endpoints`, `overrides`, `tracing`
- **llm_gateway** receives: `model_providers`, `ratelimits`, `overrides`

Both receive YAML strings parsed by `serde_yaml` in each filter's `RootContext::on_configure()`.

### Brightstaff Configuration

Brightstaff reads `arch_config_rendered.yaml` (path from `ARCH_CONFIG_PATH_RENDERED` env var), which contains the full rendered config including `model_providers`, `agents`, `filters`, `listeners`, `routing`, `model_aliases`, `state_storage`, `tracing`, and `overrides`.

---

## 6. Timeouts

All timeouts are defined in `consts.rs`:

| Constant | Value | Used For |
|---|---|---|
| `ARCH_FC_REQUEST_TIMEOUT_MS` | 30,000 ms | Arch-Function model calls from prompt_gateway |
| `DEFAULT_TARGET_REQUEST_TIMEOUT_MS` | 30,000 ms | Default prompt target endpoint calls |
| `API_REQUEST_TIMEOUT_MS` | 30,000 ms | Developer API calls from prompt_gateway |
| `MODEL_SERVER_REQUEST_TIMEOUT_MS` | 30,000 ms | Model server calls |

Envoy also enforces its own route-level timeouts configured in `envoy.template.yaml` (default 300s for LLM routes).

---

## 7. Error Response Format

All error responses from Brightstaff follow this format:

```json
{
  "error": {
    "message": "Human-readable error description",
    "type": "error_type",
    "code": 400
  }
}
```

The `llm_gateway` WASM filter returns errors as:
- HTTP 429 for rate limit exceeded
- HTTP 503 for provider unavailable
- The original upstream error status code for pass-through errors

---

## 8. Contract Change Checklist

When modifying any data contract:

- [ ] Update the constant in `common/src/consts.rs`
- [ ] Grep the entire codebase for the old value (`grep -r "old_value" crates/`)
- [ ] Update `config/envoy.template.yaml` if the header is used in routing
- [ ] Update `cli/planoai/config_generator.py` if the config schema changed
- [ ] Update `config/arch_config_schema.yaml` if user-facing config changed
- [ ] Run `cargo test --workspace` to catch compile/test failures
- [ ] Run `cd cli && python -m pytest test/` for config generation tests
