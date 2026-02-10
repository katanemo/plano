# AGENTS.md — Coding Agent Reference

> This file is optimized for AI coding agents. It contains hard constraints, ownership rules, and patterns that must not be violated. For human-readable architecture, see `architecture.md`.

---

## System Overview (30-second version)

Plano is an AI gateway. Client traffic enters **Envoy Proxy**, passes through two **WASM filters** (`prompt_gateway` → `llm_gateway`), and reaches **LLM providers**. A native Rust service (**Brightstaff**) handles intelligent routing and agent orchestration, but always communicates with the outside world **through Envoy**, never directly.

---

## Hard Rules — Never Violate These

### Rule 1: All external I/O goes through Envoy
- Brightstaff sends LLM requests to `localhost:12001` (Envoy egress listener)
- Brightstaff sends agent/API requests to `localhost:11000` (Envoy outbound listener)
- **NEVER** add `reqwest`/`hyper` calls from Brightstaff directly to external hosts
- Routing is controlled by setting `x-arch-llm-provider-hint` or `x-arch-upstream` headers

### Rule 2: WASM crates cannot use async runtimes
- `prompt_gateway` and `llm_gateway` compile to `wasm32-wasip1`
- **Forbidden in WASM crates:** `tokio`, `async-std`, `reqwest`, `hyper`, `std::net`, `std::fs`, `std::thread`
- All I/O uses `proxy-wasm` SDK's `dispatch_http_call` (callback-based, not async/await)
- `governor` must use `no_std` feature; `rand` is fine

### Rule 3: Dependency direction is one-way
```
prompt_gateway ──► common ──► hermesllm
llm_gateway    ──► common ──► hermesllm
                   llm_gateway ──► hermesllm (direct)
brightstaff    ──► hermesllm (direct, no common WASM code)
```
- `hermesllm` has **zero** dependencies on `proxy-wasm` or `common`
- `common` has **zero** dependencies on `brightstaff`
- WASM crates have **zero** dependencies on `brightstaff`

### Rule 4: Header names are canonical constants
All `x-arch-*` headers are defined in `common/src/consts.rs`. Changing a header name requires updating:
1. `common/src/consts.rs`
2. `config/envoy.template.yaml`
3. Every Rust consumer (grep for the old constant name)

### Rule 5: Config changes require a 4-file update
Adding a new user-facing config field:
1. `config/arch_config_schema.yaml` — JSON schema
2. `config/envoy.template.yaml` — Jinja2 template (if Envoy needs it)
3. `cli/planoai/config_generator.py` — Python validation/rendering
4. `common/src/configuration.rs` — Rust struct

### Rule 6: API paths are load-bearing
These paths appear in `consts.rs`, Brightstaff's Axum router, and `envoy.template.yaml`:
- `/v1/chat/completions`, `/v1/messages`, `/v1/responses`
- `/agents/v1/chat/completions`, `/agents/v1/messages`, `/agents/v1/responses`
- `/function_calling`, `/v1/models`, `/healthz`

Changing them breaks routing. Update all three locations simultaneously.

### Rule 7: Reserved model names
- `Arch-Function` — used for intent classification / function calling
- `Plano-Orchestrator` — used for agent selection
- Any model prefixed with `Arch` is treated as internal

---

## Crate Ownership Map

| Crate | Type | Target | Owner of |
|---|---|---|---|
| `brightstaff` | Binary (Axum) | Native | LLM routing, agent orchestration, state management, observability |
| `prompt_gateway` | cdylib (WASM) | wasm32-wasip1 | Intent matching, prompt guards, function calling, API orchestration |
| `llm_gateway` | cdylib (WASM) | wasm32-wasip1 | Provider routing, auth injection, rate limiting, request/response translation |
| `common` | Library | Both | Config types, HTTP client trait, constants, rate limiting, tokenization, shared OpenAI types |
| `hermesllm` | Library | Native | LLM protocol translation (OpenAI ↔ Anthropic ↔ Bedrock ↔ Gemini), SSE parsing, provider model catalog |

---

## Where to Put New Code

| You want to... | Put it in... | Why |
|---|---|---|
| Add a new LLM provider | `hermesllm` (protocol), `common/configuration.rs` (config type), `config/arch_config_schema.yaml`, `config/envoy.template.yaml` (cluster) | Provider translation is hermesllm's job |
| Add a new header for inter-component communication | `common/src/consts.rs` + `config/envoy.template.yaml` | Canonical source for all header names |
| Add rate limiting logic | `common/src/ratelimit.rs` | Shared between WASM filters |
| Add a new API endpoint to Brightstaff | `brightstaff/src/handlers/` + `brightstaff/src/main.rs` (router) | Axum handler + route registration |
| Add prompt guardrail logic | `prompt_gateway/src/stream_context.rs` or `prompt_gateway/src/http_context.rs` | Runs inline in Envoy |
| Add request/response transformation for a provider | `hermesllm/src/transforms/` | Pure Rust, no WASM dependency |
| Add config validation | `cli/planoai/config_generator.py` + `config/arch_config_schema.yaml` | Python validates before Envoy starts |
| Add a new metric | `common/src/stats.rs` (WASM) or `brightstaff/src/tracing/` (native) | Different metric systems |

---

## Build & Test Quick Reference

```bash
# Full build (WASM + native)
cd crates && ./build.sh

# WASM filters only
cargo build --release --target wasm32-wasip1 -p prompt_gateway -p llm_gateway

# Brightstaff only
cargo build --release -p brightstaff

# Run all Rust tests (native)
cargo test --workspace

# Run config generator tests
cd cli && python -m pytest test/

# Dev environment (Docker Compose)
cd config && docker compose -f docker-compose.dev.yaml up
```

---

## Envoy Listener Map (for routing decisions)

```
:10000 (ingress)          → passthrough to :10001
:10001 (prompt+llm)       → prompt_gateway.wasm → llm_gateway.wasm → LLM provider
:11000 (outbound API)     → developer APIs & agents (by x-arch-upstream header)
:agent_port (per-config)  → brightstaff :9091 /agents/...
:12000 (LLM egress)       → brightstaff :9091 (routing decision)
:12001 (LLM egress final) → llm_gateway.wasm → LLM provider
```

---

## Common Mistakes to Avoid

1. **Adding `tokio` to a WASM crate's Cargo.toml** — Will fail to compile for wasm32-wasip1
2. **Making Brightstaff call OpenAI directly** — Must go through Envoy at localhost:12001
3. **Adding a config field only in Rust** — Schema, Python generator, and template also need updates
4. **Changing a header name in one place** — Must grep and update consts.rs, envoy.template.yaml, and all consumers
5. **Adding `hermesllm` dependency on `proxy-wasm`** — hermesllm must stay pure Rust
6. **Creating a new Envoy cluster without updating the template** — Envoy won't know about it
7. **Forgetting `no_std` feature flag on `governor` in WASM crates** — std governor uses threads
