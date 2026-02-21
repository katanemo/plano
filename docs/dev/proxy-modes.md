# Proxy Modes

Plano supports two proxy modes that determine how requests are authenticated, routed, and billed.

## Managed Proxy (Original)

The managed proxy mode is the original operating mode. Plano owns the provider API keys and issues its own `xproxy_` tokens to users.

- **Identity**: User authenticates with an `xproxy_` token, which maps to a user, project, and pipe in the database
- **Auth**: ext_authz validates the token against the DB on every request
- **Routing**: Pipe-based routing selects the provider and model based on configuration
- **Key storage**: Provider API keys are stored server-side; users never see them
- **Billing**: Synchronous — cost is calculated immediately on response, counters updated before returning
- **Budget enforcement**: Hard block — if the budget is exceeded, the request is rejected inline

## Firewall Proxy (New)

Firewall mode is a transparent pass-through proxy. Users keep their own API keys and only change their base URL to point at Plano.

- **Identity**: API key is SHA-256 hashed; the hash maps to a project via `ApiKeyRegistry`
- **Auth**: ext_authz looks up the key hash in an in-memory registry (no DB query on hot path)
- **Routing**: Request is forwarded to the provider's upstream URL as registered; no pipe-based routing
- **Key storage**: User's API key passes through to the provider untouched
- **Billing**: Asynchronous — usage is recorded via fire-and-forget callout, priced in background by `PriceCalculator`
- **Budget enforcement**: Soft limit — `BudgetChecker` runs every 10s, WASM polls blocked list every 1s

See [firewall-mode.md](firewall-mode.md) for the full architecture and background pipeline details.

## Comparison

| Aspect | Managed | Firewall |
|---|---|---|
| Latency overhead | ext_authz + routing + provider auth | ext_authz only |
| User code changes | New endpoint + `xproxy_` token | Change base URL only |
| Identity mechanism | `xproxy_` token → user/project/pipe | API key SHA-256 hash → project |
| Billing timing | Synchronous (inline) | Asynchronous (background) |
| Key storage | Server-side | User-side (pass-through) |
| Budget enforcement | Hard block (per-request) | Soft limit (~11s delay) |
| Provider selection | Pipe-based routing | Direct forward to registered upstream |
| Token extraction | Full response parsing (hermesllm) | Byte scanning (zero-alloc) |

## When to Use Which

### Managed Mode
- Multi-tenant SaaS platforms where provider keys must be centralized
- Applications that need pipe-based routing across multiple providers/models
- Scenarios requiring hard budget enforcement (no overshoot)
- When you want Plano to handle provider authentication entirely

### Firewall Mode
- Teams that want observability and spending controls without changing their code
- Existing applications that already use provider APIs directly
- Environments where users must retain control of their own API keys
- Quick onboarding — only a base URL change is required
