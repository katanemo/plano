# Test Coverage Analysis

**Date:** 2026-02-18

## Executive Summary

The Plano codebase has **~370 automated tests**: ~297 Rust unit tests, ~65 Python tests (29 CLI + 50 E2E + 4 archgw integration), 10 Hurl/REST manual test files, and zero JS/TS tests. Coverage is strong in the LLM translation layer (hermesllm) and behavioral signals (brightstaff/signals), moderate in state management and configuration, and weak in the `llm_gateway` WASM plugin and several Python CLI modules.

**Note:** The `prompt_gateway` crate is deprecated and excluded from recommendations.

Below is a detailed breakdown by component with prioritized improvement recommendations.

---

## 1. Rust Crates (`crates/`)

### Current State

| Crate | Tests | Files With Tests | Status |
|-------|-------|------------------|--------|
| hermesllm | 148 | 21 | Good — broad coverage of provider translation |
| brightstaff | 126 | 11 | Good — signals/state/routing well tested; handler endpoints less so |
| common | 36 | 10 | Moderate — core utilities covered; some gaps |
| prompt_gateway | 4 | 2 | Deprecated — not prioritized for new tests |
| llm_gateway | 0 | 0 | None — WASM filter completely untested |
| **Total** | **~314** | **44** | |

### Well-Tested Areas

- **hermesllm provider translation (148 tests):** Request/response transforms for all providers (OpenAI, Anthropic, Bedrock, Gemini, Mistral) are thoroughly tested. Streaming response parsing (20 tests), endpoint resolution (11 tests), request generation (16 tests), and cross-provider format conversion (~45 tests) are solid.
- **hermesllm streaming buffers (12 tests):** SSE chunk processor (6 tests), Anthropic streaming buffer (3 tests), Responses API streaming buffer (2 tests), and passthrough buffer (1 test) have coverage.
- **brightstaff signals/analyzer (48 tests):** Character n-gram similarity, token cosine similarity, layered matching, frustration/escalation/positive-feedback detection are thoroughly tested.
- **brightstaff state management (26 tests):** In-memory state (16 tests) and PostgreSQL persistence (10 tests) have good unit test coverage.
- **brightstaff function calling (17 tests):** Tool extraction, JSON fixing, hallucination detection, and tool call verification are well covered.
- **brightstaff routing models (17 tests):** Orchestrator model v1 (9 tests) and router model v1 (8 tests) are tested.
- **brightstaff pipeline processor (5 tests):** Has basic test coverage (4 tokio::test + 1 sync test).
- **brightstaff agent selector (5 tests):** Listener lookup and agent map creation are tested.
- **brightstaff response handler (5 tests):** Response transformation has tests.
- **common rate limiting (8 tests):** Rate limit logic with token quotas and header-based selectors is tested.
- **common OpenAI API (9 tests):** Chat completion parsing and request conversions covered.

### Gaps and Recommendations

#### Gap 1: `llm_gateway` crate — 0 tests (1,399 LOC)

This WASM filter handles all LLM request/response processing and streaming. `stream_context.rs` (~1,000 lines) manages streaming chunk assembly and response forwarding with zero coverage.

**Recommendation:** Extract core logic from the WASM host context into pure, testable functions. Test streaming chunk reassembly, header manipulation, error response construction, and the filter lifecycle. Consider a thin WASM shim over well-tested logic modules.

#### ~~Gap 2: `prompt_gateway` crate~~ — DEPRECATED (skipped)

The `prompt_gateway` crate is deprecated. Investing in new tests for this crate is not recommended.

#### Gap 2: brightstaff handler endpoints — limited coverage

Several handler modules have no unit tests:
- `handlers/llm.rs` (553 LOC) — LLM chat handler
- `handlers/agent_chat_completions.rs` (418 LOC) — Multi-agent orchestration
- `handlers/router_chat.rs` (159 LOC) — Router endpoint
- `handlers/utils.rs` (288 LOC) — Handler utilities

The pipeline_processor has only 5 tests for 834 LOC — basic flow is covered but error paths and edge cases are not.

**Recommendation:** Add tests for error paths in `pipeline_processor.rs` (malformed requests, downstream failures, timeout handling). Add handler-level tests for `llm.rs` and `agent_chat_completions.rs` using `mockito` (already a dev dependency) to mock HTTP backends.

#### Gap 4: hermesllm streaming *transforms* — 0 tests

While the streaming *buffers* (SSE parser, Anthropic buffer, etc.) have tests, the streaming *transform* modules that convert between formats during streaming are untested:
- `transforms/response_streaming/to_openai_streaming.rs`
- `transforms/response_streaming/to_anthropic_streaming.rs`

Also untested: `apis/streaming_shapes/amazon_bedrock_binary_frame.rs` (AWS Event Stream binary decoding) and `apis/streaming_shapes/chat_completions_streaming_buffer.rs`.

**Recommendation:** Add tests for the streaming transform modules. The Bedrock binary frame decoder is particularly important — it parses a proprietary binary protocol and failures here are hard to diagnose in production.

#### Gap 5: common utility modules — no tests

Several `common` modules lack tests:
- `routing.rs` — Provider routing logic
- `errors.rs` — Error types (ClientError, ServerError)
- `http.rs` — HTTP utilities and CallArgs
- `stats.rs` — Metrics traits
- `api/prompt_guard.rs` — Prompt guard types
- `api/zero_shot.rs` — Zero-shot classification types

**Recommendation:** Add tests for `routing.rs` (routing decisions), `http.rs` (CallArgs construction, URL handling), and `prompt_guard.rs` (guard rule evaluation). The error/stats/consts modules are mostly type definitions and don't need extensive testing.

#### Gap 6: brightstaff state — edge cases

The state backends have solid basic coverage (26 tests total), but lack tests for:
- Concurrent access patterns
- State expiration/eviction
- Connection failure recovery (PostgreSQL)
- Large conversation histories

**Recommendation:** Add tokio::test cases for concurrent read/write scenarios in the memory backend and connection pool behavior in the PostgreSQL backend.

---

## 2. Python CLI (`cli/`)

### Current State

| Test File | Tests | Modules Covered |
|-----------|-------|-----------------|
| test_config_generator.py | 11 (5 functions + 6 parametrized) | config_generator, utils |
| test_version_check.py | 18 (4 classes, 18 methods) | versioning |
| test_init.py | 4 | init_cmd |
| test_trace_cmd.py | 2 | trace_cmd (minimal) |
| **Total** | **35 executions** | **5 of 13 modules** |

### Well-Tested Areas

- **versioning.py (18 tests):** Version parsing, comparison, PyPI fetching, network error handling, and environment variable overrides are thoroughly tested across 4 test classes.
- **config_generator.py (11 tests):** Happy-path config validation, schema validation errors (6 parametrized cases), and legacy format conversion are covered.
- **init_cmd.py (4 tests):** Clean init, template init, overwrite protection, and force overwrite are tested.

### Gaps and Recommendations

#### Gap 7: `main.py` — 0 tests (441 LOC)

The CLI entry point defines all Click commands (`up`, `down`, `build`, `logs`, `cli_agent`, `generate_prompt_targets`). None have tests. The `up` command has complex logic for port conflict detection, API key validation, and container orchestration.

**Recommendation:** Add tests using Click's `CliRunner`. Test `planoai up` with mocked Docker calls (validate argument handling, port conflict error messages, API key resolution). Test `planoai down` and `planoai build` for basic argument handling and error paths.

#### Gap 8: `targets.py` — 0 tests (365 LOC)

AST-based Python code parser that extracts prompt targets from Flask/FastAPI routes and Pydantic models. This is complex parsing logic prone to edge cases with decorators, type annotations, and docstrings.

**Recommendation:** Create test fixtures with sample Flask/FastAPI app files and verify extracted prompt targets. Test edge cases: nested decorators, complex type hints (Optional, Union, List[dict]), missing docstrings, and unsupported patterns.

#### Gap 9: `core.py` and `docker_cli.py` — 0 tests (377 LOC combined)

Container lifecycle management and Docker subprocess wrappers are untested.

**Recommendation:** Mock `subprocess.run` / `subprocess.Popen` and test the health check retry loop, container state transitions, and error handling. A shared `conftest.py` with Docker mock fixtures would benefit multiple test files.

#### Gap 10: `trace_cmd.py` — 2 tests for 993 LOC

Only gRPC bind error handling is tested. Trace collection, OTEL span processing, and trace analysis logic (the bulk of the module) are untested.

**Recommendation:** Add tests for trace data parsing and the analysis/summarization logic. Mock gRPC server interactions for collection tests.

---

## 3. JavaScript/TypeScript (`apps/`, `packages/`)

### Current State

**Zero test files. No test framework configured.** The codebase has 70+ TypeScript/React source files across two Next.js apps and shared packages. Quality tooling is limited to type checking and Biome linting.

### Recommendations

#### Gap 11: No test infrastructure

**Recommendation:** Set up Vitest in the Turbo workspace. Add `@testing-library/react` for component testing. Priority candidates:
- `apps/www/src/utils/asciiBuilder.ts` (425 lines of pure utility functions — ideal for unit tests)
- `packages/ui/src/` (shared UI components reused across apps)

**Note:** These are marketing websites, not the core proxy. Prioritize this lower than Rust and Python testing.

---

## 4. E2E and Integration Tests (`tests/`)

### Current State

| Suite | Tests | Coverage |
|-------|-------|----------|
| tests/e2e/test_prompt_gateway.py | 12 | Prompt routing, guardrails, cross-provider SDK compatibility *(deprecated path)* |
| tests/e2e/test_model_alias_routing.py | 19 | Model aliases, format translation, streaming, error handling |
| tests/e2e/test_openai_responses_api_client.py | 17 | Responses API across all providers (passthrough, chat completions, Bedrock, Anthropic) |
| tests/e2e/test_openai_responses_api_client_with_state.py | 2 | Multi-turn conversation state (memory backend) |
| tests/archgw/test_prompt_gateway.py | 3 | Prompt gateway with mock HTTP server *(deprecated path)* |
| tests/archgw/test_llm_gateway.py | 1 | LLM gateway with provider hints |
| **Total** | **54** | |

**Additional manual tests:** 3 Hurl files and 6 REST files for exploratory/manual testing.

### Well-Tested Areas

- **Cross-provider format translation:** OpenAI client → Claude model, Anthropic client → OpenAI model, etc. covered via model alias routing tests.
- **OpenAI Responses API:** Comprehensive coverage across all 4 providers in both streaming and non-streaming modes, with and without tools.
- **Prompt gateway routing:** Intent matching, parameter gathering, default targets, and jailbreak detection tested end-to-end.
- **Error handling basics:** 400 errors with invalid aliases, nonexistent aliases, and unsupported parameters.
- **archgw mock-server tests:** 404 and 500 upstream error handling tested with `pytest_httpserver`.

### Gaps and Recommendations

#### Gap 12: Error and failure scenarios underrepresented

Only a few tests cover error paths. Missing scenarios:
- Upstream provider timeouts
- 5xx errors from LLM providers during streaming
- Malformed/incomplete streaming responses
- Rate limiting behavior end-to-end
- Invalid or expired API keys

**Recommendation:** Add E2E error scenario tests. Use a mock upstream that returns errors/timeouts to test resilience behavior without depending on real provider availability.

#### Gap 13: Bedrock tests unreliable

Several AWS Bedrock tests are marked as skipped/unreliable, reducing coverage of this provider path.

**Recommendation:** Add a mock Bedrock endpoint (or use the archgw mock server pattern from `tests/archgw/`) that returns Bedrock-formatted responses including the binary event stream format. This would make Bedrock tests deterministic.

#### Gap 14: PostgreSQL state storage not E2E tested

State management E2E tests only use the memory backend. PostgreSQL is the production persistence backend.

**Recommendation:** Add a PostgreSQL container to the E2E Docker Compose setup and add tests for multi-turn state persistence, session retrieval, and cleanup.

#### Gap 15: No concurrent request / load tests

There are no tests for behavior under concurrent requests or verifying proper resource cleanup.

**Recommendation:** Add parallel request tests using `pytest-xdist` (already in dependencies) or `asyncio.gather`. Test for race conditions in state writes and resource cleanup.

#### Gap 16: No configuration validation E2E tests

Invalid configs, missing required fields, and misconfigured providers are not tested end-to-end.

**Recommendation:** Add tests that pass intentionally invalid configs to `planoai up` and verify the error messages and exit behavior.

---

## 5. Test Organization: Current State and Restructuring Proposals

### Current Layout

```
plano/
├── crates/                          # Rust — all tests inline
│   ├── brightstaff/src/*.rs         #   #[cfg(test)] mod tests at bottom of source files
│   ├── common/src/*.rs              #   same pattern
│   ├── hermesllm/src/*.rs           #   same pattern
│   ├── llm_gateway/src/*.rs         #   (no tests)
│   └── prompt_gateway/src/*.rs      #   (deprecated)
├── cli/test/                        # Python CLI unit tests
│   ├── test_config_generator.py
│   ├── test_init.py
│   ├── test_trace_cmd.py
│   └── test_version_check.py
├── tests/
│   ├── archgw/                      # Integration tests (mock HTTP server, no API keys needed)
│   │   ├── test_prompt_gateway.py
│   │   ├── test_llm_gateway.py
│   │   ├── common.py
│   │   ├── common.sh
│   │   ├── config.yaml
│   │   ├── docker-compose.yaml
│   │   └── pyproject.toml           # separate dependency management
│   ├── e2e/                         # E2E tests (requires Docker + real API keys)
│   │   ├── test_prompt_gateway.py
│   │   ├── test_model_alias_routing.py
│   │   ├── test_openai_responses_api_client.py
│   │   ├── test_openai_responses_api_client_with_state.py
│   │   ├── common.py
│   │   ├── common_scripts.sh
│   │   ├── run_e2e_tests.sh         # master orchestrator
│   │   ├── run_prompt_gateway_tests.sh
│   │   ├── run_model_alias_tests.sh
│   │   ├── run_responses_state_tests.sh
│   │   ├── docker-compose.yaml
│   │   └── pyproject.toml           # separate dependency management
│   ├── hurl/                        # Manual HTTP tests (Hurl format)
│   └── rest/                        # Manual HTTP tests (VS Code REST Client)
```

### Problems With the Current Organization

**1. Three separate Python dependency environments.** `cli/test/`, `tests/e2e/`, and `tests/archgw/` each have their own `pyproject.toml` and `uv.lock`. This means three `uv sync` calls, three separate dependency graphs, and duplicated dependencies (`pytest`, `requests`, `deepdiff`). Updating a shared dependency means touching three files.

**2. Duplicated test infrastructure.** `tests/e2e/common.py` and `tests/archgw/common.py` define overlapping constants (`PROMPT_GATEWAY_ENDPOINT`, `LLM_GATEWAY_ENDPOINT`, test fixture data). Changes to endpoint ports or test data need to be mirrored across both files.

**3. No conftest.py files.** Neither E2E nor integration tests use `conftest.py` for shared fixtures. Common setup (endpoint URLs, test data factories, health-check waits) is imported via `common.py` modules or inlined. This misses pytest's fixture scoping and teardown capabilities.

**4. Shell-script-orchestrated E2E.** The E2E suite is driven by bash scripts (`run_e2e_tests.sh`, `run_prompt_gateway_tests.sh`, etc.) that start/stop Docker containers, install the CLI, and invoke pytest. This makes it hard to run individual E2E tests locally, and the lifecycle management (container start → test → container stop → reconfigure → restart → test) is fragile. A failure in one suite can leave containers running.

**5. `tests/archgw/` naming is stale.** The directory name `archgw` references the project's old name (Arch Gateway). It should reflect its purpose (mock-based integration testing).

**6. No Rust integration test directory.** All Rust tests are inline unit tests (`#[cfg(test)] mod tests`). There is no `crates/*/tests/` directory for integration tests that exercise multiple modules together. The only exception is `brightstaff/src/handlers/integration_tests.rs`, which is a source file masquerading as an integration test.

**7. No shared Rust test utilities.** Test helper functions (creating mock configs, building test requests/responses, constructing state objects) are duplicated across inline test modules. There is no `testutils` crate or shared test module.

### Restructuring Proposals

#### Proposal A: Unify Python test dependencies

Consolidate the three separate `pyproject.toml` files into a single test workspace, or at minimum extract shared dependencies into a common base.

**Suggested approach:** Create a single `tests/pyproject.toml` that covers both `archgw/` and `e2e/` suites, with dependency groups:

```toml
[project]
name = "plano-tests"
dependencies = [
    "pytest>=8.3",
    "requests>=2.29",
    "deepdiff>=8.0",
    "pytest-retry>=1.6",
    "pytest-xdist>=3.5",
]

[project.optional-dependencies]
e2e = ["anthropic", "openai"]         # only needed for real-provider E2E
integration = ["pytest-httpserver"]    # only needed for mock-server tests
```

CLI tests (`cli/test/`) can stay separate since they live within the CLI package. The E2E and integration tests should share a single dependency environment.

#### Proposal B: Replace shell orchestration with pytest fixtures

Replace `run_e2e_tests.sh` and per-suite shell scripts with pytest session-scoped fixtures that manage Docker container lifecycle:

```python
# tests/e2e/conftest.py
import pytest, subprocess, time

@pytest.fixture(scope="session")
def plano_container(request):
    config = request.param  # e.g., "config_model_alias.yaml"
    subprocess.run(["planoai", "up", config, "--detach"], check=True)
    _wait_for_healthy("http://localhost:12000/healthz")
    yield
    subprocess.run(["planoai", "down"], check=True)
```

Benefits:
- Run individual tests with `pytest tests/e2e/test_model_alias_routing.py` without needing a shell wrapper.
- pytest handles cleanup on failure (fixture teardown runs even on test errors).
- Parametrize the container fixture to test multiple configs without separate scripts.
- Compatible with `pytest-xdist` for parallel suite execution.

#### Proposal C: Consolidate shared Python test fixtures

Create a `tests/conftest.py` (or `tests/shared/`) with common fixtures shared by both `archgw/` and `e2e/`:

```
tests/
├── conftest.py              # shared fixtures: endpoints, test data factories, health-check helpers
├── integration/             # renamed from archgw/
│   ├── conftest.py          # integration-specific fixtures (pytest-httpserver)
│   ├── test_llm_gateway.py
│   └── docker-compose.yaml
├── e2e/
│   ├── conftest.py          # e2e-specific fixtures (container lifecycle)
│   ├── test_model_alias_routing.py
│   ├── test_openai_responses_api_client.py
│   ├── test_openai_responses_api_client_with_state.py
│   └── docker-compose.yaml
└── configs/                 # test config YAML files shared across suites
    ├── model_alias_routing.yaml
    ├── memory_state_v1_responses.yaml
    └── ...
```

Key changes:
- Rename `archgw/` → `integration/` to reflect its purpose.
- Promote shared constants and test data from `common.py` into `conftest.py` fixtures.
- Remove deprecated prompt_gateway test files from both suites.
- Centralize test config YAML files in `tests/configs/`.

#### Proposal D: Add a Rust `testutils` crate

Create a workspace-internal crate for shared test utilities:

```
crates/
├── testutils/               # new crate, #[cfg(test)] only
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── fixtures.rs      # factory functions for Configuration, OpenAiChatCompletionRequest, etc.
│       ├── mock_server.rs   # wrappers around mockito for common provider mocking patterns
│       └── assertions.rs    # custom assert helpers for comparing LLM responses
```

Then in other crates:
```toml
[dev-dependencies]
testutils = { path = "../testutils" }
```

This eliminates duplication of test helper functions across `brightstaff`, `hermesllm`, and `common` inline test modules.

#### Proposal E: Add Rust integration tests for brightstaff

Move `integration_tests.rs` out of `src/handlers/` and into a proper `tests/` directory:

```
crates/brightstaff/
├── src/
│   └── handlers/
│       └── (no more integration_tests.rs here)
└── tests/                   # Cargo integration test directory
    ├── pipeline_flow.rs     # AgentSelector → PipelineProcessor → ResponseHandler
    ├── state_roundtrip.rs   # write state → read state across memory/PG backends
    └── routing_decisions.rs # config → routing decision validation
```

Cargo automatically discovers `tests/*.rs` as integration tests. They compile as separate binaries and test the crate's public API, which is the right boundary for these multi-module tests.

#### Proposal F: Add pytest markers for test categorization

Currently there is no way to selectively run tests by category. Add markers:

```python
# tests/conftest.py
def pytest_configure(config):
    config.addinivalue_line("markers", "streaming: tests that exercise streaming responses")
    config.addinivalue_line("markers", "bedrock: tests that require/mock AWS Bedrock")
    config.addinivalue_line("markers", "state: tests that exercise state persistence")
    config.addinivalue_line("markers", "error_handling: tests for error/failure paths")
    config.addinivalue_line("markers", "slow: tests that take >10s")
```

Then annotate tests:
```python
@pytest.mark.streaming
@pytest.mark.bedrock
def test_openai_client_streaming_with_bedrock():
    ...
```

This enables targeted test runs: `pytest -m "streaming and not bedrock"`, `pytest -m error_handling`, etc.

#### Proposal G: Standardize CI test jobs

The current CI has 6+ separate jobs for Python tests, each with its own setup, Docker build, and teardown. Consolidate into a clearer structure:

| Job | What it runs | Docker needed | API keys needed |
|-----|-------------|---------------|-----------------|
| `rust-unit` | `cargo test --lib` | No | No |
| `python-cli-unit` | `cd cli && uv run pytest` | No | No |
| `integration` | `cd tests/integration && uv run pytest` | Yes (compose) | No |
| `e2e` | `cd tests/e2e && uv run pytest` | Yes (plano container) | Yes |

This reduces the current 6+ Python-related jobs to 3, with clearer separation of what requires external dependencies.

### Proposal Priority

| Proposal | Effort | Impact | Priority |
|----------|--------|--------|----------|
| **C: Consolidate fixtures + rename archgw/** | Low | High — reduces duplication, clearer naming | Do first |
| **F: Add pytest markers** | Low | Medium — enables selective test runs | Do first |
| **B: Replace shell orchestration with fixtures** | Medium | High — enables local E2E dev, better cleanup | Do second |
| **A: Unify Python test dependencies** | Medium | Medium — simplifies maintenance | Do second |
| **D: Rust testutils crate** | Medium | Medium — enables faster test writing | Do with new Rust tests |
| **E: Rust integration test directory** | Low | Medium — proper Cargo conventions | Do with new Rust tests |
| **G: Consolidate CI jobs** | Medium | Medium — faster CI, simpler config | Do after A-C |

---

## Priority Summary

| Priority | Area | Gap | Recommendation |
|----------|------|-----|----------------|
| **P0** | Rust: llm_gateway | 0 tests, 1,399 LOC | Extract logic from WASM, add unit tests (#1) |
| **P0** | Rust: handler endpoints | llm.rs, agent_chat_completions.rs untested | Add handler-level tests with mockito (#2) |
| **P1** | Rust: streaming transforms | to_openai_streaming, to_anthropic_streaming, bedrock binary | Add streaming transform unit tests (#3) |
| **P1** | Rust: common utilities | routing.rs, http.rs, prompt_guard.rs | Add tests for routing decisions and HTTP utils (#4) |
| **P1** | Python: main.py | 0 tests, 441 LOC | Test CLI commands with CliRunner (#6) |
| **P1** | Python: targets.py | 0 tests, 365 LOC | Test AST parsing with sample app fixtures (#7) |
| **P1** | E2E: error scenarios | Few error path tests | Add timeout/5xx/rate-limit E2E tests (#11) |
| **P2** | Rust: state edge cases | No concurrent/expiration tests | Add async edge case tests (#5) |
| **P2** | Python: core.py/docker_cli.py | 0 tests, 377 LOC | Mock subprocess, test lifecycle (#8) |
| **P2** | Python: trace_cmd.py | 2 tests for 993 LOC | Test trace processing logic (#9) |
| **P2** | E2E: Bedrock | Tests skipped as unreliable | Use mock Bedrock endpoint (#12) |
| **P2** | E2E: PostgreSQL state | Only memory backend tested | Add PG to Docker Compose (#13) |
| **P3** | JS/TS | 0 tests, no framework | Set up Vitest, test asciiBuilder.ts (#10) |
| **P3** | E2E: concurrency | No parallel request tests | Add concurrent request tests (#14) |
| **P3** | E2E: config validation | No invalid config tests | Test error handling for bad configs (#15) |
| ~~skip~~ | ~~Rust: prompt_gateway~~ | ~~4 tests, 1,717 LOC~~ | ~~Deprecated — do not invest in new tests~~ |
