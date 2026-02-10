# Plano Rust Crates

This workspace contains 5 Rust crates that form the core of the Plano AI gateway. They are organized by compilation target and responsibility.

## Workspace Layout

```
crates/
├── Cargo.toml          # Workspace root (resolver = "2")
├── build.sh            # Builds WASM filters + native binary
├── brightstaff/        # Native Rust HTTP server (Axum)
├── common/             # Shared library (WASM-compatible)
├── hermesllm/          # LLM protocol translation (pure Rust)
├── llm_gateway/        # WASM filter: LLM routing & auth
└── prompt_gateway/     # WASM filter: intent matching & guardrails
```

---

## Crate Details

### `prompt_gateway` — Inbound Prompt Processing

| | |
|---|---|
| **Type** | `cdylib` (WASM filter) |
| **Target** | `wasm32-wasip1` |
| **Envoy listener** | `ingress_traffic_prompt` (:10001) |
| **Root ID** | `prompt_gateway` |
| **Depends on** | `common`, `proxy-wasm` |

**Responsibilities:**
- Intercepts incoming chat completion requests
- Converts `prompt_targets` into OpenAI tool definitions
- Dispatches to `Arch-Function` model for intent classification
- If intent matches: calls developer API endpoints, augments prompt with response context
- If no match: prepends system prompt, forwards to upstream LLM
- Manages multi-turn state via `x-arch-state` header
- Applies `prompt_guards` (jailbreak detection)

**Key modules:**
- `filter_context.rs` — RootContext, config parsing
- `http_context.rs` — Request interception, tool definition construction
- `stream_context.rs` — Core orchestration (intent matching, API calls, response handling)
- `tools.rs` — URL path/query parameter substitution for API calls

**Constraints:**
- No `tokio`, `async/await`, threads, or network sockets
- All HTTP calls via `proxy-wasm` `dispatch_http_call`

---

### `llm_gateway` — LLM Provider Routing & Translation

| | |
|---|---|
| **Type** | `cdylib` (WASM filter) |
| **Target** | `wasm32-wasip1` |
| **Envoy listeners** | `ingress_traffic_prompt` (:10001), `egress_traffic_llm` (:12001) |
| **Root ID** | `llm_gateway` |
| **Depends on** | `common`, `hermesllm`, `proxy-wasm` |

**Responsibilities:**
- Selects LLM provider based on `x-arch-llm-provider-hint` header or default
- Injects authentication credentials (Bearer token, x-api-key, passthrough)
- Rewrites request path for target provider API
- Transforms request/response formats between providers (OpenAI ↔ Anthropic ↔ Bedrock) via `hermesllm`
- Enforces token-based rate limits (`governor` with `no_std`)
- Handles SSE stream reassembly across chunk boundaries (`SseStreamBuffer`)
- Records metrics: TTFT, tokens/sec, request latency, rate-limited count

**Key modules:**
- `filter_context.rs` — RootContext, provider & rate limit initialization
- `stream_context.rs` — Request/response transformation, auth, rate limiting, streaming
- `metrics.rs` — Gauge, counter, histogram definitions

**Constraints:**
- Same WASM constraints as `prompt_gateway`
- Uses `hermesllm` for protocol translation — do NOT duplicate translation logic here

---

### `common` — Shared Types & Utilities

| | |
|---|---|
| **Type** | `lib` |
| **Target** | Both native and `wasm32-wasip1` |
| **Depends on** | `hermesllm`, `proxy-wasm`, `governor` (no_std), `tiktoken-rs` |

**Responsibilities:**
- Central configuration schema (`Configuration`, `LlmProvider`, `PromptTarget`, `PromptGuards`, etc.)
- `LlmProviders` collection — provider lookup with slug matching and wildcard expansion
- HTTP client trait wrapping `proxy-wasm` `dispatch_http_call`
- All `x-arch-*` header constants and path constants (`consts.rs`)
- Token-based rate limiting (`governor`, keyed by model + header selector)
- Token counting via `tiktoken-rs`
- OpenAI-compatible API types (`ChatCompletionsRequest`, `Message`, `ToolCall`, etc.)
- Error types (`ClientError`, `ServerError`)
- Metrics primitives (`Gauge`, `Counter`, `Histogram`)
- URL path parameter substitution
- PII obfuscation for logging

**Key modules:**
- `configuration.rs` — All config structs, deserialization, validation
- `consts.rs` — Canonical header names, paths, timeouts, cluster names
- `llm_providers.rs` — Provider collection with lookup logic
- `ratelimit.rs` — Token-based rate limiter (global `OnceLock`)
- `http.rs` — `Client` trait for WASM HTTP dispatch
- `tokenizer.rs` — Token counting (tiktoken, GPT-4 fallback)

**Constraints:**
- Must compile for `wasm32-wasip1` — no std networking, no threads
- Must NOT depend on `brightstaff`

---

### `hermesllm` — LLM Protocol Translation

| | |
|---|---|
| **Type** | `lib` |
| **Target** | Native only (but no WASM-incompatible deps) |
| **Depends on** | `serde`, `serde_json`, `aws-smithy-eventstream`, `uuid` |

**Responsibilities:**
- Cross-provider request/response translation (OpenAI ↔ Anthropic ↔ Amazon Bedrock ↔ Gemini)
- `ProviderRequest` / `ProviderResponse` / `ProviderStreamResponse` traits
- SSE stream parsing (`SseStreamIter`, `SseStreamBuffer`, `SseChunkProcessor`)
- AWS Event Stream binary frame decoding (Bedrock)
- Provider identification (`ProviderId` enum with model catalog from `provider_models.yaml`)
- Target endpoint path rewriting (`/v1/chat/completions` → provider-specific paths)

**Key modules:**
- `apis/` — Format definitions: `openai.rs`, `anthropic.rs`, `amazon_bedrock.rs`, `openai_responses.rs`
- `apis/streaming_shapes/` — SSE and binary stream parsing
- `providers/` — `id.rs` (ProviderId), `request.rs`, `response.rs`, `streaming_response.rs`
- `clients/endpoints.rs` — API path mapping
- `transforms/` — Request/response transformations organized by direction

**Constraints:**
- **MUST NOT depend on `proxy-wasm` or `common`** — this is a pure Rust library
- Must remain usable outside of the WASM/Envoy context
- Optional `model-fetch` feature gates network dependencies (`ureq`)

---

### `brightstaff` — Native HTTP Server

| | |
|---|---|
| **Type** | Binary (Axum) |
| **Target** | Native only |
| **Port** | `0.0.0.0:9091` |
| **Depends on** | `hermesllm`, `common` (non-WASM parts), `tokio`, `axum`, `reqwest`, `opentelemetry` |

**Responsibilities:**
- LLM request routing via `Arch-Router` model (selects best provider/model)
- Agent orchestration via `Plano-Orchestrator` model (selects and chains agents)
- Agent execution pipeline: filter chains → agent invocation (MCP JSON-RPC or HTTP)
- `Arch-Function` handler: tool calling with hallucination detection
- Conversation state management for Responses API (memory or PostgreSQL)
- Model alias resolution
- OpenTelemetry tracing with per-component service names
- Interaction signal analysis (frustration, repetition, escalation detection)

**Key modules:**
- `handlers/llm.rs` — LLM passthrough with routing
- `handlers/agent_chat_completions.rs` — Agent orchestration entry point
- `handlers/agent_selector.rs` — Agent selection logic
- `handlers/pipeline_processor.rs` — Sequential agent/filter execution
- `handlers/function_calling.rs` — Arch-Function tool calling
- `router/llm_router.rs` — `RouterService` (Arch-Router model)
- `router/plano_orchestrator.rs` — `OrchestratorService` (Plano-Orchestrator model)
- `state/` — `StateStorage` trait, memory & PostgreSQL backends
- `signals/` — Conversation quality analysis
- `tracing/` — OpenTelemetry setup with custom service name routing

**Constraints:**
- All external calls go through Envoy (localhost:12001 for LLMs, localhost:11000 for agents)
- Does NOT use `common`'s `proxy-wasm` Client trait — uses `reqwest` instead

---

## Dependency Graph

```
prompt_gateway ──► common ──► hermesllm
llm_gateway ───┬► common ──► hermesllm
               └► hermesllm
brightstaff ───┬► hermesllm
               └► common (config types only, not WASM code)

hermesllm ────► (standalone — no proxy-wasm, no common)
```

**Direction is strictly enforced:**
- Arrows point toward dependencies
- No cycles allowed
- `hermesllm` is the leaf node — it must never depend on any other workspace crate

---

## Build Commands

```bash
# Everything (recommended)
./build.sh

# Equivalent to:
cargo build --release --target wasm32-wasip1 -p prompt_gateway -p llm_gateway
cargo build --release -p brightstaff

# Tests (all crates, native target)
cargo test --workspace

# Single crate test
cargo test -p common
cargo test -p hermesllm
cargo test -p prompt_gateway
cargo test -p llm_gateway
cargo test -p brightstaff
```

## WASM Output Location

After building, WASM filter binaries are at:
```
target/wasm32-wasip1/release/prompt_gateway.wasm
target/wasm32-wasip1/release/llm_gateway.wasm
```

These are loaded by Envoy at startup from `/etc/envoy/proxy-wasm-plugins/` in the Docker image.
