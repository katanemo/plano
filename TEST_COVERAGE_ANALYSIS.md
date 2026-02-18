# Test Coverage Analysis

**Date:** 2026-02-18

## Executive Summary

The Plano codebase has significant test coverage gaps across all components. The Rust crates have ~262 unit tests covering roughly 0.35% of ~75,800 lines of code. The Python CLI has 29 tests covering 4 of 12 modules. The JavaScript/TypeScript apps and packages have **zero tests**. The E2E suite covers the core happy-path flows well but lacks error, edge-case, and performance scenarios.

Below is a prioritized breakdown of gaps and recommendations.

---

## 1. Rust Crates (`crates/`)

### Current State

| Crate | LOC | Tests | Status |
|-------|-----|-------|--------|
| common | 3,912 | 33 | Partial |
| hermesllm | 17,540 | 134 | Partial |
| prompt_gateway | 1,717 | 4 | Critical gap |
| llm_gateway | 1,399 | 0 | Critical gap |
| brightstaff | 13,342 | 91 | Partial |
| **Total** | **~75,800** | **262** | |

### Critical Gaps

**llm_gateway — 0 tests, 1,399 LOC.** This WASM filter handles LLM request/response processing and streaming. It has no tests at all. `stream_context.rs` alone is ~1,000 lines of complex streaming logic with zero coverage.

**prompt_gateway — 4 tests, 1,717 LOC.** The WASM filter for prompt processing and guardrails has near-zero coverage. Untested modules include `filter_context.rs`, `http_context.rs`, `context.rs`, and `metrics.rs`. The intent-matching logic in `stream_context.rs` (~900 lines) has only 1 test.

**brightstaff pipeline and state management — ~2,200 LOC untested.** The core request pipeline (`handlers/pipeline_processor.rs`, 834 lines), state persistence layer (`state/memory.rs`, `state/postgresql.rs`, `state/response_state_processor.rs` — 1,370 lines combined), and key handler endpoints (`handlers/llm.rs`, `handlers/agent_chat_completions.rs`) have no tests.

### Partially Covered Areas Needing More Tests

- **hermesllm streaming transforms** — The non-streaming request/response transforms are well-tested (134 tests), but the streaming buffer modules (`sse.rs`, `amazon_bedrock_binary_frame.rs`, `to_openai_streaming.rs`, `to_anthropic_streaming.rs` — ~5,000 LOC) are untested.
- **common/routing.rs, common/errors.rs, common/http.rs, common/stats.rs, common/tracing.rs** — Utility modules totaling ~560 lines with no coverage.
- **brightstaff router services** — `llm_router.rs` and `plano_orchestrator.rs` (~400 lines) lack tests despite handling routing decisions.

### Recommendations

1. **Add unit tests for llm_gateway.** Start with `stream_context.rs` — test streaming chunk assembly, partial frame handling, error recovery, and the filter lifecycle. A WASM-mocking test harness or extracting the core logic into testable pure functions would help.

2. **Add unit tests for prompt_gateway filter logic.** Test `http_context.rs` request/response handling, `filter_context.rs` lifecycle, and the guardrail filtering paths in `stream_context.rs`.

3. **Test the brightstaff pipeline processor.** This is the central message processing pipeline. Mock the downstream dependencies and test the orchestration logic, error paths, and streaming assembly.

4. **Test state persistence.** Both the in-memory and PostgreSQL backends need tests for basic CRUD, concurrent access, state expiration, and connection failure recovery.

5. **Test hermesllm streaming transforms.** The SSE parser, Bedrock binary frame decoder, and streaming-to-OpenAI/Anthropic converters need unit tests, especially for edge cases like partial frames, malformed chunks, and connection resets.

---

## 2. Python CLI (`cli/`)

### Current State

| Module | LOC | Tested? |
|--------|-----|---------|
| config_generator.py | 514 | Yes |
| versioning.py | 70 | Yes |
| init_cmd.py | 303 | Yes |
| trace_cmd.py | 993 | Minimal (2 tests) |
| main.py | 441 | No |
| targets.py | 365 | No |
| core.py | 234 | No |
| docker_cli.py | 143 | No |
| template_sync.py | 122 | No |
| utils.py | 285 | Partial |

**29 total tests across 4 files. 8 of 12 modules are untested or minimally tested.**

### Critical Gaps

**main.py — 0 tests, 441 LOC.** All CLI commands (`up`, `down`, `build`, `logs`, `cli_agent`, `generate_prompt_targets`) are untested. The `up` command alone contains complex logic for port conflict detection, API key validation, and container orchestration.

**targets.py — 0 tests, 365 LOC.** The AST-based Python code parser for extracting prompt targets from Flask/FastAPI routes and Pydantic models is entirely untested. This is complex parsing logic prone to edge cases.

**core.py — 0 tests, 234 LOC.** Docker container lifecycle management (start, stop, health check retry loop, timeout handling) is untested.

**docker_cli.py — 0 tests, 143 LOC.** All 7 Docker subprocess wrapper functions lack tests.

**trace_cmd.py — 2 tests for 993 LOC.** Only gRPC server bind error handling is tested. The trace collection, OTEL processing, and trace analysis logic are untested.

### Recommendations

6. **Add CLI command tests using Click's CliRunner.** Test `planoai up`, `planoai down`, and `planoai build` with mocked Docker operations. Verify argument validation, error messages, and exit codes.

7. **Add tests for targets.py.** Test Flask route extraction, FastAPI route extraction, Pydantic model field parsing, type annotation handling, and edge cases (nested decorators, complex type hints, missing docstrings).

8. **Add tests for core.py with mocked subprocess/Docker calls.** Test the health check retry loop, container state transitions (not found → start, running → restart), timeout behavior, and port forwarding.

9. **Add a shared conftest.py** with common fixtures for environment setup, temporary config files, and Docker mocking.

---

## 3. JavaScript/TypeScript (`apps/`, `packages/`)

### Current State

**Zero test files. No test framework configured. No test scripts in any package.json.**

The codebase has 70+ TypeScript/React source files across two Next.js apps (`apps/www`, `apps/katanemo-www`) and shared packages (`packages/ui`, `packages/shared-styles`).

Quality tooling is limited to type checking (`tsc --noEmit`) and linting (Biome).

### Notable Untested Code

- **`apps/www/src/utils/asciiBuilder.ts`** (425 lines) — Pure utility functions for ASCII diagram generation (`calculateCenterPadding`, `createArrow`, `buildBox`, `fixDiagramSpacing`, `createFlowDiagram`). This is the most testable code in the frontend.
- **`packages/ui/src/`** — 5 shared UI components (Navbar, Footer, Logo, Button, Dialog) used across apps.
- **`apps/www/src/app/api/contact/route.ts`** — API route handler.

### Recommendations

10. **Set up Vitest** (or Jest) in the Turbo workspace with a root-level `test` script. Add `@testing-library/react` for component testing.

11. **Add unit tests for `asciiBuilder.ts`.** These are pure functions with clear inputs and outputs — ideal first candidates.

12. **Add component tests for shared `packages/ui` components.** These are reused across apps and should have rendering and interaction tests.

Note: The JS/TS apps are marketing websites, not the core proxy. Prioritize this lower than Rust and Python testing.

---

## 4. E2E Tests (`tests/e2e/`)

### Current State

~40 active tests across 4 test files, covering:
- OpenAI and Anthropic SDK integration (streaming and non-streaming)
- Model alias routing and format translation
- Function calling end-to-end flows
- OpenAI Responses API (v1/responses)
- Conversation state management (memory backend)
- Cross-provider format translation (OpenAI client → Claude model, etc.)

### Gaps

**Error and failure scenarios are underrepresented.** Only 2 tests cover error handling (400 errors with aliases). There are no tests for:
- Upstream provider unavailability or timeouts
- Malformed request payloads
- Rate limiting behavior
- Invalid API keys
- Partial stream failures or disconnections

**Bedrock tests are all skipped.** 6 AWS Bedrock tests are marked as unreliable and skipped, leaving this provider path untested in CI.

**PostgreSQL state storage is untested.** State management E2E tests only use the memory backend. The PostgreSQL backend (which is the production path) has no E2E coverage.

**No concurrent request testing.** There are no tests validating behavior under concurrent load or verifying resource cleanup.

**No configuration validation E2E tests.** Invalid config files, missing required fields, and config hot-reload are not tested end-to-end.

### Recommendations

13. **Add E2E error scenario tests.** Test upstream timeouts, 5xx errors from providers, malformed responses, and rate limit responses. These are the scenarios most likely to cause production incidents.

14. **Fix or replace the skipped Bedrock tests.** If Bedrock is flaky, consider using a mock provider or stub that mimics the Bedrock binary event stream format.

15. **Add PostgreSQL state storage E2E tests.** Use a PostgreSQL container in Docker Compose and test state persistence, multi-turn retrieval, and state cleanup.

16. **Add concurrent request tests.** Use `pytest-xdist` (already in dependencies) to validate behavior under parallel requests.

---

## Priority Summary

| Priority | Area | Recommendation |
|----------|------|----------------|
| **P0** | Rust: llm_gateway | Add unit tests for streaming response handling (#1) |
| **P0** | Rust: prompt_gateway | Add unit tests for filter logic and guardrails (#2) |
| **P0** | Rust: brightstaff pipeline | Test the core pipeline processor (#3) |
| **P1** | Rust: state persistence | Test memory and PostgreSQL backends (#4) |
| **P1** | Rust: streaming transforms | Test hermesllm streaming modules (#5) |
| **P1** | Python: CLI commands | Test main.py commands with CliRunner (#6) |
| **P1** | Python: targets.py | Test AST parsing logic (#7) |
| **P1** | E2E: error scenarios | Test upstream failures, timeouts, rate limits (#13) |
| **P2** | Python: core.py | Test Docker lifecycle management (#8) |
| **P2** | E2E: PostgreSQL state | Test production state backend (#15) |
| **P2** | E2E: Bedrock | Fix skipped Bedrock tests (#14) |
| **P3** | JS/TS: test setup | Set up Vitest, test utilities (#10, #11) |
| **P3** | E2E: concurrency | Add parallel request tests (#16) |
