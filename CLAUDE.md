# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Plano is an AI-native proxy server and data plane for agentic applications, built on Envoy proxy. It centralizes agent orchestration, LLM routing, observability, and safety guardrails as an out-of-process dataplane.

## Repository Structure

```
plano/
├── crates/                  # Rust workspace (5 crates: WASM plugins + native binary + libraries)
├── cli/                     # Python CLI tool (planoai) for managing Plano lifecycle
├── apps/                    # Next.js 16 / React 19 web applications (Turbo monorepo)
│   ├── www/                 # Main marketing/docs site with Sanity CMS
│   └── katanemo-www/        # Katanemo company website
├── packages/                # Shared TypeScript packages
│   ├── ui/                  # React component library (Radix UI based)
│   ├── shared-styles/       # Global CSS
│   ├── tailwind-config/     # Shared Tailwind v4 config
│   └── tsconfig/            # Shared TypeScript configs
├── config/                  # Configuration templates and schemas
├── tests/                   # E2E, integration, REST API, and HURL tests
├── demos/                   # Sample applications and use-case demos
├── docs/                    # Sphinx documentation source
├── .github/workflows/       # CI/CD (14 workflows)
├── Dockerfile               # Multi-stage production build (Rust 1.93.0, Envoy v1.37.0, Python 3.13)
├── turbo.json               # Turbo monorepo orchestration
├── package.json             # Root npm workspace config
└── .pre-commit-config.yaml  # Pre-commit hooks
```

## Build & Test Commands

### Rust (crates/)

```bash
# Build WASM plugins (must target wasm32-wasip1)
cd crates && cargo build --release --target=wasm32-wasip1 -p llm_gateway -p prompt_gateway

# Build brightstaff binary (native target)
cd crates && cargo build --release -p brightstaff

# Run unit tests
cd crates && cargo test --lib

# Format check
cd crates && cargo fmt --all -- --check

# Lint
cd crates && cargo clippy --locked --all-targets --all-features -- -D warnings
```

### Python CLI (cli/)

```bash
cd cli && uv sync              # Install dependencies
cd cli && uv run pytest -v     # Run tests
cd cli && uv run planoai --help  # Run CLI
```

### JavaScript/TypeScript (apps/, packages/)

```bash
npm run build      # Build all (via Turbo)
npm run lint       # Lint all
npm run dev        # Dev servers
npm run typecheck  # Type check
```

### Pre-commit (runs fmt, clippy, cargo test, black, yaml checks)

```bash
pre-commit run --all-files
```

### Docker

```bash
docker build -t katanemo/plano:latest .
```

### E2E Tests (tests/e2e/)

E2E tests require a built Docker image and API keys. They run via `tests/e2e/run_e2e_tests.sh` which executes three test suites: `test_prompt_gateway.py`, `test_model_alias_routing.py`, and `test_openai_responses_api_client_with_state.py`. Individual suites can be run with `run_prompt_gateway_tests.sh`, `run_model_alias_tests.sh`, or `run_responses_state_tests.sh`.

## Architecture

### Core Data Flow

Requests flow through Envoy proxy with two WASM filter plugins, backed by a native Rust binary:

```
Client → Envoy (prompt_gateway.wasm → llm_gateway.wasm) → Agents/LLM Providers
                              ↕
                         brightstaff (native binary: state, routing, signals, tracing)
```

### Rust Crates (crates/)

All crates share a Cargo workspace. Two compile to `wasm32-wasip1` for Envoy, the rest are native:

- **prompt_gateway** (WASM, `crate-type = ["cdylib"]`) — Proxy-WASM filter for prompt/message processing, guardrails, and filter chains. Key files: `lib.rs` (entry point), `http_context.rs`, `filter_context.rs`, `stream_context.rs`, `metrics.rs`
- **llm_gateway** (WASM, `crate-type = ["cdylib"]`) — Proxy-WASM filter for LLM request/response handling and routing. Same structure as prompt_gateway
- **brightstaff** (native binary) — Core application server. Key subdirectories:
  - `handlers/` — Request handling: `function_calling.rs` (largest file), `agent_chat_completions.rs`, `llm.rs`, `pipeline_processor.rs`, `response_handler.rs`, `jsonrpc.rs`
  - `router/` — Routing: `llm_router.rs`, `plano_orchestrator.rs`, `router_model.rs`/`router_model_v1.rs`, `orchestrator_model.rs`/`orchestrator_model_v1.rs`
  - `state/` — State management with `memory.rs` (in-memory) and `postgresql.rs` (PostgreSQL) backends
  - `signals/` — Signal analysis engine (`analyzer.rs`)
  - `tracing/` — OpenTelemetry instrumentation
- **common** (library) — Shared across all crates: `configuration.rs`, `llm_providers.rs`, `routing.rs`, `ratelimit.rs`, `tokenizer.rs`, `pii.rs`, `tracing.rs`, `http.rs`. Sub-modules: `api/` (hallucination, prompt_guard, zero_shot), `traces/` (span builders, shapes)
- **hermesllm** (library) — Translates LLM API formats between providers. Supported providers via `ProviderId`: OpenAI, Anthropic, Gemini, Mistral, Groq, GitHub, AWS Bedrock, Azure, together.ai, Deepseek. Key types: `ProviderRequest`, `ProviderResponse`, `ProviderStreamResponse`. Sub-modules:
  - `apis/` — Provider-specific API implementations (`openai.rs`, `anthropic.rs`, `amazon_bedrock.rs`, `openai_responses.rs`)
  - `transforms/request/` — Cross-provider request translation (`from_openai.rs`, `from_anthropic.rs`)
  - `transforms/response/` — Cross-provider response translation (`to_openai.rs`, `to_anthropic.rs`, `output_to_input.rs`)
  - `transforms/response_streaming/` — Streaming response translation
  - `apis/streaming_shapes/` — SSE parsing and streaming buffers per provider

### Python CLI (cli/planoai/)

The `planoai` CLI manages the Plano lifecycle. Key commands:
- `planoai up <config.yaml>` — Validate config, check API keys, start Docker container
- `planoai down` — Stop container
- `planoai build` — Build Docker image from repo root
- `planoai logs` — Stream access/debug logs
- `planoai trace` — OTEL trace collection and analysis
- `planoai init` — Initialize new project

Entry point: `cli/planoai/main.py`. Container lifecycle in `core.py`. Docker operations in `docker_cli.py`. Config generation in `config_generator.py`. Trace analysis in `trace_cmd.py`. Templates for init in `cli/planoai/templates/`.

### Configuration System (config/)

- `arch_config_schema.yaml` — JSON Schema (draft-07) for validating user config files
- `envoy.template.yaml` — Jinja2 template rendered into Envoy proxy config
- `supervisord.conf` — Process supervisor for Envoy + brightstaff in the container
- `docker-compose.dev.yaml` — Development Docker Compose setup
- `validate_plano_config.sh` — Config validation script

User configs define: `agents` (id + url), `model_providers` (model + access_key), `listeners` (type: agent/model/prompt, with router strategy), `filters` (filter chains), and `tracing` settings.

### JavaScript Apps (apps/, packages/)

Turbo monorepo with Next.js 16 / React 19 applications and shared packages. Not part of the core proxy — these are marketing/documentation websites.

- **apps/www** (`@katanemo/www`) — Main site with blog, research section, Sanity CMS integration
- **apps/katanemo-www** (`@katanemo/katanemo-www`) — Katanemo company website
- **packages/ui** (`@katanemo/ui`) — Shared React components (Navbar, Footer, Logo, Radix UI primitives)
- **packages/shared-styles** — Global CSS custom properties
- **packages/tailwind-config** — Centralized Tailwind v4 config
- **packages/tsconfig** — Shared TypeScript configs (`base.json`, `nextjs.json`)

## Testing Strategy

### Unit Tests
- **Rust**: `cd crates && cargo test --lib` — runs unit tests across all crates
- **Python CLI**: `cd cli && uv run pytest -v` — tests in `cli/test/` (config generation, init, version checks)

### E2E Tests (tests/e2e/)
Require a running Docker container with API keys. Orchestrated by shell scripts:
- `run_e2e_tests.sh` — runs all three suites
- `test_prompt_gateway.py` — prompt gateway filter behavior
- `test_model_alias_routing.py` — model alias routing
- `test_openai_responses_api_client_with_state.py` — stateful OpenAI Responses API

### Integration Tests (tests/archgw/)
- `test_prompt_gateway.py`, `test_llm_gateway.py` — gateway-level integration tests with Docker Compose

### REST API Tests (tests/rest/)
- `.rest` files for manual API testing against prompt gateway, LLM gateway, model server, and routing

### HURL Tests (tests/hurl/)
- HTTP-level API tests for LLM gateway model selection behaviors

## CI/CD Workflows (.github/workflows/)

- **pre-commit.yml** — Pre-commit hook validation on PRs
- **rust_tests.yml** — Rust unit tests
- **static.yml** — Static analysis
- **plano_tools_tests.yml** — Python CLI tests
- **validate_arch_config.yml** — Configuration schema validation
- **e2e_tests.yml** — Main E2E test suite
- **e2e_plano_tests.yml** — Plano-specific E2E tests
- **e2e_test_currency_convert.yml** / **e2e_test_preference_based_routing.yml** — Demo-specific E2E tests
- **docker-push-main.yml** / **docker-push-release.yml** — Docker Hub image publishing
- **ghrc-push-main.yml** / **ghrc-push-release.yml** — GitHub Container Registry publishing
- **publish-pypi.yml** — PyPI package publishing for planoai CLI

## Toolchain Requirements

- **Rust**: 1.93.0, edition 2021 (must have `wasm32-wasip1` target installed via `rustup target add wasm32-wasip1`)
- **Python**: >=3.10 (3.13 in Docker), managed with `uv`
- **Node.js**: >=18.0.0, npm 10.0.0
- **Docker**: Required for E2E tests and production builds (base image: `envoyproxy/envoy:v1.37.0`)
- **Pre-commit**: Install with `pip install pre-commit && pre-commit install`

## Key Conventions

- Rust edition 2021, formatted with `cargo fmt`, linted with `cargo clippy -D warnings`
- Python formatted with Black
- WASM plugins must target `wasm32-wasip1` — they run inside Envoy, not as native binaries
- The Docker image bundles Envoy + WASM plugins + brightstaff + Python CLI into a single container managed by supervisord
- API keys come from environment variables or `.env` files, never hardcoded
- Brightstaff supports two state backends: in-memory (`state/memory.rs`) and PostgreSQL (`state/postgresql.rs`)
- LLM provider translation is centralized in hermesllm — add new providers there, not in individual crates
- The Cargo workspace is in `crates/`, not at the repo root — always `cd crates` before running cargo commands
- Pre-commit hooks must pass before committing (cargo fmt, clippy, cargo test --lib, black, YAML checks)
