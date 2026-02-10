# ADR 005: Header-Based Routing Protocol

**Status:** Accepted

## Context

Envoy needs to route requests to different upstream clusters (LLM providers, developer APIs, agents) based on runtime decisions made by WASM filters and Brightstaff. The options were:
1. **Path-based routing** — different URL paths for different upstreams
2. **Header-based routing** — custom headers to signal routing decisions
3. **Dynamic cluster selection** — programmatic cluster selection in filters

## Decision

Use **custom `x-arch-*` headers** for all routing decisions. WASM filters and Brightstaff set headers like `x-arch-llm-provider` and `x-arch-upstream`, and Envoy's route configuration matches on these headers to select the upstream cluster.

All header names are defined as constants in `common/src/consts.rs` — this is the single source of truth.

## Consequences

**Enables:**
- Decoupled routing: WASM filters decide *where* to route, Envoy handles *how* to connect
- Transparent to the client — custom headers are internal, clients see standard HTTP
- Easy to debug: inspect headers to understand routing decisions
- Composable: multiple filters can add/modify routing headers in the filter chain

**Requires:**
- Header names must be consistent between `consts.rs` and `envoy.template.yaml`
- Any new routing dimension needs a new header constant + Envoy route match rule
- Developers must grep all consumers when changing a header name

**Prevents:**
- Routing logic in Envoy's configuration alone (routing decisions are made by Rust code, not Envoy config)
- Using Envoy's native routing features (like weighted clusters) independently — they must be combined with header matching

**Key headers:**
- `x-arch-llm-provider` — LLM provider cluster selection (Envoy route matching)
- `x-arch-llm-provider-hint` — Provider hint from Brightstaff to llm_gateway
- `x-arch-upstream` — Agent/API endpoint cluster selection
- `x-arch-streaming-request` — Streaming mode signal
- `x-arch-state` — Multi-turn conversation state (prompt_gateway internal)
