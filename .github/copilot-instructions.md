# Copilot Instructions for Plano (ArchGW)

## System Identity

Plano is an AI-native gateway built on Envoy Proxy. It uses WASM filters for inline request processing and a native Rust service (Brightstaff) for orchestration. All components run in a single container managed by Supervisord.

## Critical Architectural Rules

### 1. Envoy Is the Data Plane — Never Bypass It

All external traffic MUST flow through Envoy. Brightstaff NEVER makes direct outbound HTTP calls to LLM providers or developer APIs. It always routes through Envoy listeners:
- LLM requests → `localhost:12001` (egress LLM listener with `llm_gateway.wasm`)
- Agent/API requests → `localhost:11000` (outbound API listener)

**Do not** add direct HTTP calls from Brightstaff to external services. Use Envoy's cluster routing via `x-arch-*` headers instead.

### 2. WASM Crate Constraints

`prompt_gateway` and `llm_gateway` compile to `wasm32-wasip1`. This means:
- **No `tokio`, no `async/await`, no threads, no filesystem, no network sockets**
- All I/O goes through `proxy-wasm` SDK's `dispatch_http_call` (async callback-based)
- No crate with `std` networking features — use `governor` with `no_std`, etc.
- The `crate-type` is `["cdylib"]` — these are shared libraries, not binaries
- Test with `cargo test` (native), but build with `--target wasm32-wasip1`

**Do not** add dependencies to WASM crates that require `std::net`, `tokio`, `reqwest`, `hyper`, or any async runtime.

### 3. Crate Dependency Direction

```
prompt_gateway → common
llm_gateway    → common, hermesllm
common         → hermesllm
brightstaff    → common (non-WASM parts), hermesllm
hermesllm      → (standalone, no proxy-wasm)
```

- `hermesllm` must NEVER depend on `proxy-wasm` or `common` — it's a pure Rust library usable outside WASM
- `common` provides the `proxy-wasm` abstractions — WASM crates use `common`, not raw `proxy-wasm` directly (except for the SDK traits)
- `brightstaff` uses `hermesllm` directly for LLM types but does NOT use `common`'s WASM-specific code (like `proxy-wasm` Client trait)

### 4. Header-Based Routing Protocol

Envoy routes requests using custom headers. These are the canonical header names defined in `common/src/consts.rs`:

| Header | Purpose | Do NOT change |
|--------|---------|---------------|
| `x-arch-llm-provider` | Envoy route matching for LLM provider cluster | Used in envoy.template.yaml |
| `x-arch-llm-provider-hint` | Brightstaff → llm_gateway provider selection | Both sides must agree |
| `x-arch-upstream` | Targets a specific agent/API cluster in Envoy | Used in envoy.template.yaml |
| `x-arch-streaming-request` | Signals streaming mode | llm_gateway reads this |
| `x-arch-state` | Multi-turn conversation state in prompt_gateway | Serialized JSON |
| `x-arch-tool-call-message` | Tool call metadata | prompt_gateway internal |
| `x-arch-api-response-message` | Developer API response | prompt_gateway internal |
| `x-arch-agent-listener-name` | Identifies agent listener | Set by Envoy, read by Brightstaff |
| `x-arch-llm-route` | LLM route decision result | Brightstaff ↔ llm_gateway |

Changing header names requires updating: `consts.rs`, `envoy.template.yaml`, and all consumers.

### 5. Build System

```bash
# WASM filters — must use wasm32-wasip1 target
cargo build --release --target wasm32-wasip1 -p prompt_gateway -p llm_gateway

# Brightstaff — native binary
cargo build --release -p brightstaff
```

The workspace uses Rust edition 2021 and resolver "2". The workspace root is `crates/Cargo.toml`.

### 6. Configuration Flow

User config (`arch_config.yaml`) is validated and rendered by `cli/planoai/config_generator.py`:
- Schema: `config/arch_config_schema.yaml`
- Template: `config/envoy.template.yaml` (Jinja2)
- Output: `envoy.yaml` (for Envoy) + `arch_config_rendered.yaml` (for Brightstaff + WASM filter configs)

When adding new config fields: update the schema, the template (if Envoy-relevant), the Python generator, AND the Rust `Configuration` struct in `common/src/configuration.rs`.

### 7. Internal Model Names

These are reserved model names used internally — do not conflict with them:
- `Arch-Function` — intent classification / function calling
- `Arch-Router` — (used as route name prefix, not direct model name)
- `Plano-Orchestrator` — agent selection orchestrator

### 8. API Compatibility

Brightstaff exposes OpenAI-compatible endpoints:
- `/v1/chat/completions` — Chat Completions API
- `/v1/messages` — Anthropic Messages API compatible
- `/v1/responses` — OpenAI Responses API with state management
- `/function_calling` — Internal Arch-Function endpoint

The `/agents/` prefix variants mirror these for agent orchestration.

Do NOT change these path structures without updating `consts.rs`, Brightstaff router, and `envoy.template.yaml`.

### 9. Streaming

- LLM responses use SSE (Server-Sent Events) format: `data: {json}\n\n`
- The `llm_gateway` WASM filter handles SSE stream reassembly across chunk boundaries via `SseStreamBuffer`
- Brightstaff uses `mpsc` channels for streaming responses back to clients
- Bedrock uses AWS Event Stream binary protocol — decoded by `hermesllm`

### 10. Testing Conventions

- WASM crates: unit tests run natively (`cargo test`), NOT under WASM runtime
- Brightstaff: unit tests with `mockito` for HTTP mocking
- E2E tests: separate `tests/` directory, run via GitHub Actions workflows
- Config validation tests: `cli/test/test_config_generator.py`

## File Layout Reference

```
crates/
  Cargo.toml          # Workspace root
  brightstaff/        # Native Rust HTTP server (Axum)
  common/             # Shared types, config, HTTP, rate limiting
  hermesllm/          # LLM protocol translation (pure Rust)
  llm_gateway/        # WASM filter: provider routing, auth, rate limits
  prompt_gateway/     # WASM filter: intent matching, guardrails
config/
  arch_config_schema.yaml   # User config JSON schema
  envoy.template.yaml       # Jinja2 template → envoy.yaml
  docker-compose.dev.yaml   # Dev environment
cli/
  planoai/                  # Python CLI (config generator, Docker management)
```
