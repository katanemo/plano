# Plano (ArchGW) — High-Level Architecture

## Overview

Plano is an AI-native gateway built on **Envoy Proxy**, extended with custom **WebAssembly (WASM) filters** and a native Rust service called **Brightstaff**. It acts as an intelligent intermediary between client applications, AI agents, and LLM providers — handling intent-based routing, prompt guardrails, function calling, agent orchestration, rate limiting, and multi-provider LLM translation.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              Plano Gateway                                  │
│                                                                             │
│   ┌──────────────────────────────────────────────────────────────────────┐  │
│   │                         Envoy Proxy (L7)                             │  │
│   │                                                                      │  │
│   │   ┌──────────────────┐       ┌──────────────────┐                    │  │
│   │   │  prompt_gateway  │──────▶│   llm_gateway     │                   │  │
│   │   │    (WASM)        │       │     (WASM)        │                   │  │
│   │   │                  │       │                   │                   │  │
│   │   │ • Intent matching│       │ • Provider routing│                   │  │
│   │   │ • Guardrails     │       │ • Auth injection  │                   │  │
│   │   │ • Function call  │       │ • Rate limiting   │                   │  │
│   │   │ • Prompt targets │       │ • API translation │                   │  │
│   │   └──────────────────┘       └────────┬─────────┘                   │  │
│   │                                       │                              │  │
│   └───────────────────────────────────────┼──────────────────────────────┘  │
│                                           │                                 │
│   ┌───────────────────────────────────────┼──────────────────────────────┐  │
│   │                    Brightstaff (Rust HTTP Server :9091)               │  │
│   │                                                                      │  │
│   │   • LLM request routing (Arch-Router model)                          │  │
│   │   • Agent orchestration (Plano-Orchestrator model)                   │  │
│   │   • Conversation state management (memory / PostgreSQL)              │  │
│   │   • Function calling handler (Arch-Function model)                   │  │
│   │   • Observability & signal analysis                                  │  │
│   └──────────────────────────────────────────────────────────────────────┘  │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
         │                    │                         │
         ▼                    ▼                         ▼
   ┌──────────┐      ┌──────────────┐          ┌──────────────┐
   │  Agents  │      │ Developer    │          │ LLM Providers│
   │ (MCP/HTTP)│     │   APIs       │          │ (OpenAI, etc)│
   └──────────┘      └──────────────┘          └──────────────┘
```

---

## The Role of Envoy

Envoy is the **data plane** of Plano. All client traffic — both inbound prompts and outbound LLM calls — flows through Envoy. It provides:

- **L7 HTTP routing** based on paths and custom headers
- **WASM filter execution** for inline request/response transformation
- **Connection pooling and TLS** to upstream LLM providers
- **Retry policies** for resilience
- **Compression/decompression** for LLM streaming responses

### Envoy Listeners

Envoy defines **six listener types**, each serving a distinct role in the request flow:

| Listener | Port | Direction | Purpose |
|---|---|---|---|
| `ingress_traffic` | 10000 (configurable) | Inbound | Client-facing entry point. Forwards all traffic to the prompt gateway listener. |
| `ingress_traffic_prompt` | 10001 | Inbound | **Core processing listener.** Runs both WASM filters (`prompt_gateway` → `llm_gateway`). Routes to LLM providers by `x-arch-llm-provider` header. |
| `outbound_api_traffic` | 11000 | Internal | Routes to upstream developer APIs and agents using `x-arch-upstream` header. No WASM filters. |
| Agent listeners | Per-config | Inbound | One per agent listener in config. Routes to Brightstaff with `/agents/` path prefix. |
| `egress_traffic` | 12000 (configurable) | Outbound | LLM gateway entry for agents/services reaching LLMs. Routes to Brightstaff for routing decisions. |
| `egress_traffic_llm` | 12001 | Outbound | **Final outbound LLM listener.** Runs `llm_gateway.wasm` for auth injection, provider translation, and rate limiting before reaching the actual LLM provider. |

### Envoy Clusters

Envoy manages connections to all upstream services:

**LLM Provider Clusters** — Pre-configured TLS clusters for: OpenAI, Anthropic (Claude), Groq, Mistral, DeepSeek, Gemini, xAI, MoonshotAI, Zhipu, Together AI, and Katanemo's hosted Arch models. Custom-URL providers (e.g., Azure OpenAI, Ollama) are dynamically added from config.

**Internal Clusters:**

| Cluster | Target | Purpose |
|---|---|---|
| `bright_staff` | localhost:9091 | The Brightstaff Rust service |
| `arch_prompt_gateway_listener` | localhost:10001 | Internal forwarding from ingress |
| `arch_listener_llm` | localhost:12001 | Internal forwarding for LLM egress |
| `arch_internal` | localhost:11000 | Outbound API router |

**Dynamic Clusters** — Generated from `endpoints` and `agents` config sections (developer APIs, agent services).

### Custom Headers Used for Routing

| Header | Set By | Used By | Purpose |
|---|---|---|---|
| `x-arch-llm-provider` | WASM filters | Envoy routes | Selects the LLM provider cluster |
| `x-arch-llm-provider-hint` | Brightstaff | llm_gateway | Hints which provider/model to use |
| `x-arch-upstream` / `x-arch-upstream-host` | WASM filters / Brightstaff | Envoy routes | Targets a specific agent or API endpoint |
| `x-arch-is-streaming` | Brightstaff | llm_gateway | Indicates streaming mode |
| `x-arch-state` | prompt_gateway | prompt_gateway | Carries multi-turn conversation state |
| `x-arch-tool-call` | prompt_gateway | prompt_gateway | Carries tool call metadata |
| `x-arch-api-response` | prompt_gateway | prompt_gateway | Carries developer API response data |
| `x-arch-agent-listener-name` | Envoy | Brightstaff | Identifies which agent listener a request arrived on |

---

## Request Flows

### Flow 1: Direct LLM Chat (`POST /v1/chat/completions`)

This is the standard path for client-to-LLM requests with optional intent matching and routing.

```
Client
  │
  ▼
[Envoy :10000 — ingress_traffic]
  │  (simple passthrough)
  ▼
[Envoy :10001 — ingress_traffic_prompt]
  │
  ├── prompt_gateway.wasm
  │     1. Parse ChatCompletions request
  │     2. Convert prompt_targets → tool definitions
  │     3. Dispatch to Arch-Function model at /function_calling
  │     4. If intent matched:
  │         → Call developer API endpoint via :11000
  │         → Augment prompt with API response context
  │     5. If no intent matched:
  │         → Prepend system prompt, forward to LLM
  │
  ├── llm_gateway.wasm
  │     1. Select LLM provider (from header hint or default)
  │     2. Enforce rate limits (token-based via tiktoken)
  │     3. Inject auth credentials (Bearer / x-api-key)
  │     4. Transform request format (OpenAI ↔ Anthropic ↔ Bedrock)
  │     5. Rewrite upstream path for target provider
  │
  ▼
LLM Provider (OpenAI, Anthropic, Gemini, etc.)
  │
  ▼
(Response flows back through llm_gateway for format translation)
  │
  ▼
Client
```

### Flow 2: Brightstaff LLM Routing (`POST /v1/chat/completions` via egress)

When requests reach Brightstaff (directly or via agent listeners), it performs intelligent model routing.

```
Client / Agent
  │
  ▼
[Brightstaff :9091]
  │
  ├── Resolve model aliases
  ├── Validate model exists in configured providers
  ├── Retrieve conversation state (if using Responses API)
  │
  ├── Call Arch-Router model ──► [Envoy :12001]
  │     (determines best model/provider for the request    ──► LLM Provider
  │      based on routing_preferences in config)
  │
  ├── Forward actual request ──► [Envoy :12001]
  │     (with x-arch-llm-provider-hint header)             ──► LLM Provider
  │
  ▼
[Stream response back with metrics, signal analysis, state capture]
  │
  ▼
Client / Agent
```

### Flow 3: Agent Orchestration (`POST /agents/v1/chat/completions`)

The agentic flow where Brightstaff selects and chains agents based on user intent.

```
Client
  │
  ▼
[Envoy — Agent Listener :configurable]
  │  (path rewrite: /agents/...)
  ▼
[Brightstaff :9091]
  │
  ├── Identify listener from x-arch-agent-listener-name
  ├── Find configured agents for this listener
  │
  ├── If multiple agents:
  │     Call Plano-Orchestrator model ──► [Envoy :12001] ──► LLM
  │     (selects which agents to run and in what order)
  │
  ├── For each selected agent:
  │     │
  │     ├── Run filter chain (pre-processing)
  │     │     └── [Envoy :11000] ──► Filter Service (MCP/HTTP)
  │     │
  │     ├── Invoke agent
  │     │     └── [Envoy :11000] ──► Agent Service (MCP/HTTP)
  │     │
  │     ├── If intermediate agent:
  │     │     Collect full response → feed as input to next agent
  │     │
  │     └── If final agent:
  │           Stream response directly to client
  │
  ▼
Client
```

---

## Brightstaff Service

Brightstaff is a native Rust HTTP server (`0.0.0.0:9091`) built with Axum. It is the **control plane brain** of Plano — while Envoy handles the data plane (proxying, filtering), Brightstaff handles the intelligent decision-making.

### Endpoints

| Method | Path | Handler | Purpose |
|---|---|---|---|
| `POST` | `/v1/chat/completions` | `llm_chat` | LLM passthrough with model routing |
| `POST` | `/v1/messages` | `llm_chat` | Anthropic Messages API compat |
| `POST` | `/v1/responses` | `llm_chat` | OpenAI Responses API with state |
| `POST` | `/agents/v1/chat/completions` | `agent_chat` | Agent orchestration pipeline |
| `POST` | `/agents/v1/messages` | `agent_chat` | Agent orchestration (Messages) |
| `POST` | `/agents/v1/responses` | `agent_chat` | Agent orchestration (Responses) |
| `POST` | `/function_calling` | `function_calling_chat_handler` | Arch-Function tool calling |
| `GET` | `/v1/models` | `list_models` | List configured LLM models |

### Core Components

#### RouterService (LLM Routing)
Uses the **Arch-Router** model — a specialized LLM that determines which provider/model best matches a user's request based on `routing_preferences` defined in config. Constructs a system prompt describing available routes, sends the conversation, and parses a `{"route": "route_name"}` response.

#### OrchestratorService (Agent Selection)
Uses the **Plano-Orchestrator** model to determine which agent(s) should handle a request when multiple agents are available on a listener. Returns an ordered list of agents: `{"route": ["agent1", "agent2"]}`.

#### PipelineProcessor (Agent Execution)
Manages the sequential execution of agent filter chains and agent invocations:
- **MCP agents**: JSON-RPC 2.0 protocol over SSE transport (`initialize` → `notifications/initialized` → `tools/call`)
- **HTTP agents**: Direct POST with message array
- Routes through Envoy at `:11000` using `x-arch-upstream-host` header

#### Function Calling Handler
Specialized handler for the **Arch-Function** model:
- Converts OpenAI tool definitions into prompts
- Parses structured JSON responses (tool_calls, clarifications)
- Includes **hallucination detection** using entropy/varentropy/probability thresholds from logprobs

#### State Management
Manages conversation state for the OpenAI Responses API (`v1/responses`):
- **Memory backend** — `HashMap` behind `Arc<RwLock>` for single-instance dev
- **PostgreSQL backend** — Persistent storage with upsert semantics
- `ResponsesStateProcessor` intercepts streaming responses to capture `response_id` and output items, storing them asynchronously for future conversation chaining via `previous_response_id`

#### Signal Analysis (Observability)
Analyzes conversation patterns for interaction quality:
- Frustration, repetition/looping, escalation requests, positive feedback, repair patterns
- Quality graded as Good / Fair / Poor / Severe
- Concerning signals flag spans with indicators for monitoring

---

## Rust Crate Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     brightstaff (binary)                     │
│                                                             │
│   Native Rust HTTP server — routing, orchestration, state   │
│   Depends on: hermesllm, common (non-WASM parts)           │
└─────────────────────────────────────────────────────────────┘

┌──────────────────────┐    ┌──────────────────────┐
│   prompt_gateway     │    │   llm_gateway         │
│      (WASM)          │    │      (WASM)           │
│                      │    │                       │
│  Intent matching     │    │  Provider routing     │
│  Prompt guards       │    │  Auth injection       │
│  Function calling    │    │  Rate limiting        │
│  API orchestration   │    │  Request/Response     │
│                      │    │  format translation   │
├──────────────────────┤    ├───────────────────────┤
│  depends on: common  │    │  depends on: common,  │
│                      │    │  hermesllm            │
└──────────┬───────────┘    └──────────┬────────────┘
           │                           │
           ▼                           ▼
┌──────────────────────────────────────────────────────────────┐
│                        common (lib)                          │
│                                                             │
│  Configuration types, LlmProviders, HTTP client trait,      │
│  rate limiting (governor), tokenization (tiktoken),         │
│  OpenAI API types, routing, metrics, tracing, constants     │
│  Depends on: hermesllm                                      │
└─────────────────────────────┬───────────────────────────────┘
                              │
                              ▼
┌──────────────────────────────────────────────────────────────┐
│                       hermesllm (lib)                        │
│                                                             │
│  LLM protocol abstraction — cross-provider request/response │
│  translation (OpenAI ↔ Anthropic ↔ Bedrock ↔ Gemini)       │
│  SSE stream parsing, provider model catalog, endpoint       │
│  mapping. No proxy-wasm dependency (pure Rust).             │
└──────────────────────────────────────────────────────────────┘
```

### WASM Compilation

Both `prompt_gateway` and `llm_gateway` compile to `cdylib` targets for `wasm32-wasip1` using the `proxy-wasm` SDK (v0.2.1). Envoy loads them via its V8 WASM runtime. Each filter implements `RootContext` (for config parsing and per-stream creation) and `HttpContext` (for per-request processing).

---

## Deployment Architecture

All components run inside a single container managed by **Supervisord**:

```
┌─────────────────────────────────────────────────────────────┐
│                     Docker Container                         │
│                                                             │
│  ┌─────────────────────────────────────────────────────┐    │
│  │                   Supervisord                        │    │
│  │                                                     │    │
│  │  ┌─────────────┐  ┌───────────────┐  ┌───────────┐ │    │
│  │  │ Brightstaff  │  │  Envoy Proxy  │  │  Log Tail │ │    │
│  │  │  (Rust)      │  │  + WASM       │  │           │ │    │
│  │  │  :9091       │  │  :10000-12001 │  │           │ │    │
│  │  └─────────────┘  └───────────────┘  └───────────┘ │    │
│  └─────────────────────────────────────────────────────┘    │
│                                                             │
│  Startup sequence:                                          │
│   1. config_generator.py validates arch_config.yaml         │
│   2. Renders envoy.template.yaml → envoy.yaml (Jinja2)     │
│   3. Starts Brightstaff + Envoy in parallel                 │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

**Docker multi-stage build:**
1. `deps` — Rust 1.93.0 with `wasm32-wasip1` target, dependency pre-compilation
2. `wasm-builder` — Builds `prompt_gateway.wasm` + `llm_gateway.wasm` (release)
3. `brightstaff-builder` — Builds the `brightstaff` native binary (release)
4. `envoy` — Pulls `envoyproxy/envoy:v1.37.0`
5. `arch` (final) — Python 3.13.6-slim base with Envoy binary, WASM plugins, Brightstaff binary, and the `planoai` CLI

---

## Configuration Pipeline

User-facing configuration flows through a generation pipeline before reaching Envoy and Brightstaff:

```
arch_config.yaml (user-authored)
        │
        ▼
config_generator.py (Python CLI)
  1. Validate against arch_config_schema.yaml (JSON Schema)
  2. Normalize legacy formats (llm_providers → model_providers)
  3. Parse agents, filters, endpoints → infer Envoy clusters
  4. Parse model_providers → validate provider/model format
  5. Auto-add internal models (arch-function, arch-router, plano-orchestrator)
  6. Validate model aliases, routing preferences, prompt target endpoints
        │
        ├──► envoy.yaml (rendered from envoy.template.yaml via Jinja2)
        │      → consumed by Envoy
        │
        └──► arch_config_rendered.yaml
               → consumed by Brightstaff
               → injected into WASM filter configs
```

### Key Config Sections

| Section | Consumed By | Purpose |
|---|---|---|
| `model_providers` | llm_gateway, Brightstaff | LLM provider definitions with models, auth, routing preferences |
| `prompt_targets` | prompt_gateway | Intent-to-API mappings with parameter schemas |
| `prompt_guards` | prompt_gateway | Input guardrails (jailbreak detection) |
| `endpoints` | prompt_gateway, Envoy | Named upstream API endpoint definitions |
| `agents` | Brightstaff, Envoy | Agent service definitions (id, URL, type) |
| `listeners` | Brightstaff, Envoy | Listener configs binding agents to ports |
| `ratelimits` | llm_gateway | Per-model rate limits with token-based quotas |
| `routing` | Brightstaff | LLM routing model/provider config |
| `model_aliases` | Brightstaff | Friendly name → provider/model mappings |
| `state_storage` | Brightstaff | Conversation state backend (memory / postgres) |
| `tracing` | All components | OpenTelemetry config (sampling, OTLP endpoint) |
| `overrides` | prompt_gateway, Brightstaff | Tuning (intent threshold, agent orchestrator toggle) |

---

## Supported LLM Providers

| Provider | Cluster | Auth Method |
|---|---|---|
| OpenAI | api.openai.com | Bearer token |
| Anthropic (Claude) | api.anthropic.com | x-api-key header |
| Google (Gemini) | generativelanguage.googleapis.com | API key in URL |
| Groq | api.groq.com | Bearer token |
| Mistral | api.mistral.ai | Bearer token |
| DeepSeek | api.deepseek.com | Bearer token |
| xAI | api.x.ai | Bearer token |
| Together AI | api.together.xyz | Bearer token |
| MoonshotAI | api.moonshot.ai | Bearer token |
| Zhipu | open.bigmodel.cn | Bearer token |
| Amazon Bedrock | Custom base_url | AWS Sig v4 |
| Azure OpenAI | Custom base_url | Bearer / API key |
| Ollama | Custom base_url | None |
| Katanemo (Arch) | archfc.katanemo.dev | Bearer token |

The `hermesllm` crate handles **cross-provider request/response translation** so clients can use a single API format (typically OpenAI-compatible) regardless of which upstream provider serves the request.
