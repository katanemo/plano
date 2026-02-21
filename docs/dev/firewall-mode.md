# Firewall Mode

Firewall mode is a transparent proxy mode where the user's real API keys pass through to the LLM provider untouched. Plano adds observability, usage tracking, and soft spending limits without requiring code changes or key migration.

## Firewall vs Managed Mode

| Aspect | Managed Mode | Firewall Mode |
|---|---|---|
| API keys | Stored server-side, user gets `xproxy_` token | User keeps their own keys |
| Latency overhead | ext_authz + routing + provider auth | ext_authz only (transparent forward) |
| Identity | `xproxy_` token maps to user/project | API key SHA-256 hash maps to project |
| Billing | Synchronous — priced on response | Asynchronous — priced in background |
| Budget enforcement | Per-request, hard block | Soft limit, polled every 1s |
| User code changes | Must use Plano endpoint + token | Only change base URL (keep same key) |

## Architecture

```
Client (real API key in Authorization header)
  │
  ▼
Envoy ext_authz ──► brightstaff /auth/check
  │                    │
  │                    ├─ Hash API key (SHA-256)
  │                    ├─ Lookup in ApiKeyRegistry
  │                    └─ Return headers:
  │                         x-xproxy-firewall-mode: true
  │                         x-xproxy-upstream-url: <provider_url>
  │                         x-xproxy-project-id: <uuid>
  │                         x-xproxy-provider-hint: <provider>
  │                         x-xproxy-api-key-hash: <hash>
  │
  ▼
prompt_gateway.wasm (DLP/PII scanning if enabled)
  │
  ▼
llm_gateway.wasm
  │
  ├─ Detect firewall mode from header
  ├─ Check blocked_projects → 429 if blocked
  ├─ Strip x-xproxy-* headers
  ├─ Extract model name + streaming flag (byte scan)
  ├─ Forward request to upstream (key untouched)
  │
  ▼
LLM Provider (OpenAI, Anthropic, Gemini, ...)
  │
  ▼
llm_gateway.wasm (response path)
  │
  ├─ Byte-scan response for token usage
  ├─ Fire-and-forget POST to /usage/record
  └─ Forward response to client unchanged
```

## Hot Path

The hot path is designed to add minimal latency:

1. **ext_authz** — brightstaff looks up the API key hash in an in-memory `HashMap` (the `ApiKeyRegistry`). No DB query on the hot path.
2. **WASM filter** — detects firewall mode, strips internal headers, does lightweight byte scanning for model/streaming fields. No full JSON parse.
3. **Forward** — request passes to the upstream provider with the original `Authorization` header intact.
4. **Response** — byte-scan for token counts (see [byte-scan-parsing.md](byte-scan-parsing.md)), then fire-and-forget HTTP callout to `/usage/record`.

## Background Pipeline

Four background components handle billing asynchronously:

### UsageFlusher
- Receives `UsageEvent` structs via an `mpsc` channel
- Batches events (up to 1000 or every 10 seconds)
- Inserts into `usage_log` table
- Updates `SpendingCounters` with deltas

### PriceCalculator
- Runs every 10 seconds
- Fetches unpriced usage records from DB (up to 1000 per batch)
- Calculates cost using `PricingRegistry` (custom pricing → global override → Portkey data)
- Updates in-memory spending counters
- Marks records as priced in DB

### BudgetChecker
- Runs every 10 seconds
- Queries all active spending limits from DB
- Compares cumulative spending against limits
- Maintains a `DashSet<Uuid>` of blocked project IDs

### ApiKeyRegistry
- In-memory `HashMap<String, RegisteredKeyInfo>` keyed by key hash
- Reloads from `registered_api_keys` table every 60 seconds
- Used by `/auth/check` on the hot path — no DB query per request

## Extraction Level

Firewall mode uses `ExtractionLevel::UsageOnly`, which selects byte scanning over full serde parsing. See [adaptive-extraction.md](adaptive-extraction.md) for the enum design and benchmark comparison.

## Egress IP Isolation

Different API keys can route through different outbound IPs for network-level isolation. The `egress_ip` field on `registered_api_keys` maps to Envoy clusters with distinct `bind_config.source_address`. See [adaptive-extraction.md](adaptive-extraction.md#egress-ip-isolation) for details.

## Identity

In firewall mode, identity is derived from the API key hash:

- The user's API key is SHA-256 hashed
- One key hash = one project in the registry
- `RegisteredKeyInfo` maps a hash to: `project_id`, `provider`, `upstream_url`, `display_name`, `is_active`
- Keys are registered via the API before first use

## Custom Pricing

Cost calculation follows a priority chain:

1. **Project override** — custom per-model pricing set for a specific project
2. **Global override** — custom pricing applied across all projects
3. **Portkey data** — default pricing from the Portkey pricing database

The `PricingRegistry` in `PriceCalculator` resolves pricing using `calculate_cost_with_custom()`.

## Soft Spending Limits

Budget enforcement in firewall mode is intentionally soft (eventually consistent):

1. `BudgetChecker` queries DB for spending limits every 10 seconds
2. `SpendingCounters` (in-memory `DashMap` with `AtomicI64` values) track cumulative spend
3. When a project exceeds its limit, its UUID is added to the blocked set
4. `FilterContext` in the WASM filter polls `GET /budget/blocked` every 1 second
5. On the next request, `StreamContext` checks the `blocked_projects` set
6. If blocked, returns HTTP 429 with:
   ```json
   {"error": "spending_limit_exceeded", "message": "project has exceeded its spending limit"}
   ```

The maximum enforcement delay is ~11 seconds (10s BudgetChecker cycle + 1s WASM poll cycle).

## Provider Support

| Provider | Input Tokens Field | Output Tokens Field | Default |
|---|---|---|---|
| OpenAI (and compatible) | `prompt_tokens` | `completion_tokens` | Yes |
| Anthropic | `input_tokens` | `output_tokens` | |
| Gemini / Google | `promptTokenCount` | `candidatesTokenCount` | |

For streaming responses, OpenAI-compatible providers have `stream_options: {"include_usage": true}` injected to ensure the final SSE chunk includes token counts.

## Key Files

| File | Purpose |
|---|---|
| `crates/llm_gateway/src/stream_context.rs` | Core firewall mode logic, byte scanning, usage extraction |
| `crates/llm_gateway/src/filter_context.rs` | Budget polling (`GET /budget/blocked` every 1s) |
| `crates/brightstaff/src/billing/flusher.rs` | `UsageFlusher` — batches and inserts usage events |
| `crates/brightstaff/src/billing/price_calculator.rs` | `PriceCalculator` — async pricing of usage records |
| `crates/brightstaff/src/billing/budget_checker.rs` | `BudgetChecker` — spending limit enforcement |
| `crates/brightstaff/src/billing/counters.rs` | `SpendingCounters` — in-memory atomic spend tracking |
| `crates/brightstaff/src/registry.rs` | `ApiKeyRegistry` — API key hash → project lookup |
| `crates/brightstaff/src/handlers/auth_check.rs` | `/auth/check` — firewall mode auth endpoint |
| `crates/brightstaff/src/handlers/usage_record.rs` | `/usage/record` — usage recording endpoint |
| `crates/brightstaff/src/handlers/budget_blocked.rs` | `/budget/blocked` — blocked projects endpoint |
| `crates/common/src/consts.rs` | Header name constants |

## Testing Locally

### Prerequisites
- `brightstaff` binary built (`cd crates && cargo build -p brightstaff`)
- A database (PostgreSQL) with the schema migrated
- An LLM provider API key

### Steps

1. **Register an API key**
   ```bash
   curl -X POST http://localhost:8080/api-keys/register \
     -H "Content-Type: application/json" \
     -d '{
       "api_key": "sk-your-real-key",
       "provider": "openai",
       "upstream_url": "https://api.openai.com",
       "display_name": "my-test-key"
     }'
   ```

2. **Verify auth check**
   ```bash
   curl -X GET http://localhost:8080/auth/check \
     -H "Authorization: Bearer sk-your-real-key" \
     -H "x-xproxy-mode: firewall"
   ```
   Should return 200 with `x-xproxy-firewall-mode: true` and other routing headers.

3. **Make a proxied request** (through Envoy)
   ```bash
   curl -X POST http://localhost:10000/v1/chat/completions \
     -H "Authorization: Bearer sk-your-real-key" \
     -H "Content-Type: application/json" \
     -d '{
       "model": "gpt-4",
       "messages": [{"role": "user", "content": "Hello"}]
     }'
   ```

4. **Verify usage was recorded**
   ```bash
   curl http://localhost:8080/usage/recent?project_id=<project-uuid>
   ```

5. **Check budget status**
   ```bash
   curl http://localhost:8080/budget/blocked
   ```
   Returns `{"blocked": []}` when no projects are over limit.
