# CLAUDE.md

Plano is an AI-native proxy server and data plane for agentic applications, built on Envoy proxy. It centralizes agent orchestration, LLM routing, observability, and safety guardrails as an out-of-process dataplane.

## Build & Test Commands

```bash
# Rust — WASM plugins (must target wasm32-wasip1)
cd crates && cargo build --release --target=wasm32-wasip1 -p llm_gateway -p prompt_gateway

# Rust — brightstaff binary (native target)
cd crates && cargo build --release -p brightstaff

# Rust — tests, format, lint
cd crates && cargo test --lib
cd crates && cargo fmt --all -- --check
cd crates && cargo clippy --locked --all-targets --all-features -- -D warnings

# Python CLI
cd cli && uv sync && uv run pytest -v

# JS/TS (Turbo monorepo)
npm run build && npm run lint && npm run typecheck

# Pre-commit (fmt, clippy, cargo test, black, yaml)
pre-commit run --all-files

# Docker
docker build -t katanemo/plano:latest .
```

E2E tests require a Docker image and API keys: `tests/e2e/run_e2e_tests.sh`

## Architecture

```
Client → Envoy (prompt_gateway.wasm → llm_gateway.wasm) → Agents/LLM Providers
                              ↕
                         brightstaff (native binary: state, routing, signals, tracing)
```

### Crates (crates/)

- **prompt_gateway** (WASM) — Proxy-WASM filter for prompt processing, guardrails, filter chains
- **llm_gateway** (WASM) — Proxy-WASM filter for LLM request/response handling and routing
- **brightstaff** (native) — Core server: handlers, router, signals, state, tracing
- **common** (lib) — Shared: config, HTTP, routing, rate limiting, tokenizer, PII, tracing
- **hermesllm** (lib) — LLM API translation between providers. Key types: `ProviderId`, `ProviderRequest`, `ProviderResponse`, `ProviderStreamResponse`

### Python CLI (cli/planoai/)

Entry point: `main.py`. Built with `rich-click`. Commands: `up`, `down`, `build`, `logs`, `trace`, `init`, `cli_agent`, `generate_prompt_targets`, `chatgpt`, `obs`.

### Config (config/)

- `plano_config_schema.yaml` — JSON Schema for validating user configs
- `envoy.template.yaml` — Jinja2 template → Envoy config
- `supervisord.conf` — Process supervisor for Envoy + brightstaff

### JS Apps (apps/, packages/)

Turbo monorepo with Next.js 16.1 / React 19.2. Not part of the core proxy. Apps: `www`, `katanemo-www`.

## WASM Plugin Rules

Code in `prompt_gateway` and `llm_gateway` runs in Envoy's WASM sandbox:

- **No std networking/filesystem** — use proxy-wasm host calls only
- **No tokio/async** — synchronous, callback-driven. `Action::Pause` / `Action::Continue` for flow control
- **Lifecycle**: `RootContext` → `on_configure`, `create_http_context`; `HttpContext` → `on_http_request/response_headers/body`
- **HTTP callouts**: `dispatch_http_call()` → store context in `callouts: RefCell<HashMap<u32, CallContext>>` → match in `on_http_call_response()`
- **Config**: `Rc`-wrapped, loaded once in `on_configure()` via `serde_yaml::from_slice()`
- **Dependencies must be no_std compatible** (e.g., `governor` with `features = ["no_std"]`)
- **Crate type**: `cdylib` → produces `.wasm`

## Adding a New LLM Provider

1. Add variant to `ProviderId` in `crates/hermesllm/src/providers/id.rs` + `TryFrom<&str>`
2. Create request/response types in `crates/hermesllm/src/apis/` if non-OpenAI format
3. Add variant to `ProviderRequestType`/`ProviderResponseType` enums, update all match arms
4. Add models to `crates/hermesllm/src/bin/provider_models.yaml`
5. Update `SupportedUpstreamAPIs` mapping if needed

## Release Process

Update version (e.g., `0.4.21` → `0.4.22`) in all of these files:

- `.github/workflows/ci.yml`, `build_filter_image.sh`
- `cli/planoai/__init__.py`, `cli/pyproject.toml`
- `docs/source/conf.py`, `docs/source/get_started/quickstart.rst`
- `apps/www/src/components/Hero.tsx` (if version is displayed)

Do NOT change version strings in `*.lock` files or `Cargo.lock`. Commit message: `release X.Y.Z`

## Workflow Preferences

- **Commits:** No `Co-Authored-By`. Short one-line messages. Never push directly to `main` — always feature branch + PR.
- **Branches:** Use `adil/<feature_name>` format.
- **Issues:** When a GitHub issue URL is pasted, fetch all context first. Goal is always a PR with passing tests.

## Key Conventions

- Rust edition 2021, `cargo fmt`, `cargo clippy -D warnings`
- Python: Black. Rust errors: `thiserror` with `#[from]`
- API keys from env vars or `.env`, never hardcoded
- Provider dispatch: `ProviderRequestType`/`ProviderResponseType` enums implementing `ProviderRequest`/`ProviderResponse` traits
