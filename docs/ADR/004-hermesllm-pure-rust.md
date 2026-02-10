# ADR 004: hermesllm as a Pure Rust Library

**Status:** Accepted

## Context

LLM providers use different API formats (OpenAI Chat Completions, Anthropic Messages, Amazon Bedrock Converse, Gemini). The gateway needs to translate between these formats in two places:
1. In the `llm_gateway` WASM filter (inline in Envoy)
2. In Brightstaff (for routing decisions and response processing)

The options were:
1. Duplicate translation logic in both places
2. Put translation logic in `common` (shared crate, but WASM-constrained)
3. Create a separate pure Rust library with no WASM dependencies

## Decision

Create **`hermesllm`** as a standalone Rust library that handles all LLM protocol translation. It must never depend on `proxy-wasm` or `common`. Both WASM crates (via `common`) and Brightstaff use `hermesllm` directly.

## Consequences

**Enables:**
- Single source of truth for LLM protocol translation
- Reusable outside the gateway context (could be published as an independent crate)
- Full Rust standard library available (no WASM constraints on the library itself)
- Clean separation: protocol knowledge lives in `hermesllm`, gateway logic lives in filters

**Requires:**
- `hermesllm` must not import `proxy-wasm`, `common`, or any WASM-specific crate
- Adding a new provider requires changes only in `hermesllm` (plus config in `common/configuration.rs` and `envoy.template.yaml`)
- Types shared between `hermesllm` and the filters go through `common`'s re-exports

**Prevents:**
- Circular dependencies (hermesllm is always a leaf in the dependency graph)
- Accidentally coupling protocol translation to WASM runtime specifics
- Needing to maintain two separate translation implementations

**Dependency direction:**
```
prompt_gateway → common → hermesllm
llm_gateway    → common → hermesllm
llm_gateway    → hermesllm (direct)
brightstaff    → hermesllm (direct)
hermesllm      → (no workspace deps)
```
