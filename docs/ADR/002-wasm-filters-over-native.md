# ADR 002: WASM Filters Over Native Envoy Filters

**Status:** Accepted

## Context

Envoy supports three extension mechanisms:
1. **Native C++ filters** — compiled into the Envoy binary, highest performance
2. **WASM filters** — compiled to WebAssembly, loaded at runtime via Envoy's WASM VM
3. **Lua filters** — scripted, limited functionality
4. **External processing (ext_proc)** — gRPC callout to an external service

We need filters that: parse and transform LLM request/response bodies, perform intent matching, inject authentication headers, enforce rate limits, and handle SSE stream reassembly.

## Decision

Use **WASM filters** written in Rust, compiled to `wasm32-wasip1`, loaded by Envoy's V8 runtime. We have two filters:
- `prompt_gateway.wasm` — inbound prompt processing (intent matching, guardrails, function calling)
- `llm_gateway.wasm` — outbound LLM processing (provider routing, auth, rate limiting, format translation)

## Consequences

**Enables:**
- Filters written in Rust with strong type safety and shared crates (`common`, `hermesllm`)
- Runtime-loadable: no need to rebuild Envoy itself
- Sandboxed execution: a filter crash doesn't bring down Envoy
- Same language (Rust) for WASM filters and Brightstaff — shared types and logic via workspace crates

**Requires:**
- No `tokio`, `async/await`, threads, filesystem, or network sockets in WASM crates
- All I/O must use `proxy-wasm` SDK's `dispatch_http_call` (callback-based)
- Dependencies must be WASM-compatible: `governor` needs `no_std` feature, no crates using `std::net`
- `crate-type = ["cdylib"]` — these build as shared libraries, not binaries
- Testing runs natively (`cargo test`), but building requires `--target wasm32-wasip1`

**Prevents:**
- Using async Rust patterns in filter code (callback-based `on_http_call_response` instead)
- Using popular HTTP client crates (`reqwest`, `hyper`) in filters
- Easy debugging — WASM filters run inside Envoy's V8 VM with limited introspection

**Trade-off vs. ext_proc:**
External processing would allow using Brightstaff (native Rust with full async) for all processing, but would add network round-trips for every request. WASM filters run inline in Envoy's filter chain — zero additional network hops for common operations like auth injection and rate limiting.
