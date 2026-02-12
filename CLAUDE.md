# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Plano is an AI-native proxy server and data plane for agentic applications, built on Envoy proxy. It centralizes agent orchestration, LLM routing, observability, and safety guardrails as an out-of-process dataplane.

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

E2E tests require a built Docker image and API keys. They run via `tests/e2e/run_e2e_tests.sh` which executes three test suites: `test_prompt_gateway.py`, `test_model_alias_routing.py`, and `test_openai_responses_api_client_with_state.py`.

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

- **prompt_gateway** (WASM) — Proxy-WASM filter for prompt/message processing, guardrails, and filter chains
- **llm_gateway** (WASM) — Proxy-WASM filter for LLM request/response handling and routing
- **brightstaff** (native binary) — Core application server: handlers, router, signals, state management, tracing
- **common** (library) — Shared across all crates: configuration, LLM provider abstractions, HTTP utilities, routing logic, rate limiting, tokenizer, PII detection, tracing
- **hermesllm** (library) — Translates LLM API formats between providers (OpenAI, Anthropic, Gemini, Mistral, Grok, AWS Bedrock, Azure, together.ai). Key types: `ProviderId`, `ProviderRequest`, `ProviderResponse`, `ProviderStreamResponse`

### Python CLI (cli/planoai/)

The `planoai` CLI manages the Plano lifecycle. Key commands:
- `planoai up <config.yaml>` — Validate config, check API keys, start Docker container
- `planoai down` — Stop container
- `planoai build` — Build Docker image from repo root
- `planoai logs` — Stream access/debug logs
- `planoai trace` — OTEL trace collection and analysis
- `planoai init` — Initialize new project

Entry point: `cli/planoai/main.py`. Container lifecycle in `core.py`. Docker operations in `docker_cli.py`.

### Configuration System (config/)

- `arch_config_schema.yaml` — JSON Schema (draft-07) for validating user config files
- `envoy.template.yaml` — Jinja2 template rendered into Envoy proxy config
- `supervisord.conf` — Process supervisor for Envoy + brightstaff in the container

User configs define: `agents` (id + url), `model_providers` (model + access_key), `listeners` (type: agent/model/prompt, with router strategy), `filters` (filter chains), and `tracing` settings.

### JavaScript Apps (apps/, packages/)

Turbo monorepo with Next.js 16 / React 19 applications and shared packages (UI components, Tailwind config, TypeScript config). Not part of the core proxy — these are web applications.

## Key Conventions

- Rust edition 2021, formatted with `cargo fmt`, linted with `cargo clippy -D warnings`
- Python formatted with Black
- WASM plugins must target `wasm32-wasip1` — they run inside Envoy, not as native binaries
- The Docker image bundles Envoy + WASM plugins + brightstaff + Python CLI into a single container managed by supervisord
- API keys come from environment variables or `.env` files, never hardcoded
