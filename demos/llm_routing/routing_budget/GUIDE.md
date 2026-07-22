# How-To: See Prompt Caching + Routing Budget in Action

A hands-on guide for running Plano's automatic prompt caching and the routing budget locally, and for measuring the win in evals/benchmarks (e.g. on DigitalOcean models).

There are two independent behaviors to observe:

1. **Automatic prompt caching** — staying on one model keeps the stable prefix
  warm, so per-turn input cost drops sharply across a multi-turn conversation.
2. **Routing budget** — the router still runs every turn, but when it proposes a
  *different* model while the session's cache is plausibly warm, Plano only switches
  while the session's cumulative switch spend stays within `max_overhead_pct`% of what
  staying put would have cost. This is a routing concern and is self-sufficient:
  it needs no `prompt_caching` config (it derives sessions and prices warm
  anchors at cached rates on its own).

---



## 1. Prerequisites

- Plano CLI installed: `pip install planoai` (or `uv sync` from `cli/` for a dev build).
- Provider credentials as env vars, e.g.:
  - `export DIGITALOCEAN_API_KEY=...` (DO SI)
  - `export OPENAI_API_KEY=...`, `export ANTHROPIC_API_KEY=...` (if comparing)
- `curl` + `jq` for poking the endpoint.

---



## 2. Configuration

Start from `[config.yaml](config.yaml)` in this folder. The parts that matter:

```yaml
# Per-model pricing is REQUIRED for the routing budget — the switch cost math needs
# each model's input and cached-input rates. This demo reads DigitalOcean's managed
# pricing catalog, which publishes cached-read rates for its models. The catalog is
# keyed by bare DO model ids, so model_aliases maps them onto the Plano model names
# used in model_providers — without the mapping no rates match and every switch
# decision fails open (reason="no_pricing").
model_metrics_sources:
  - type: cost
    provider: digitalocean
    refresh_interval: 86400
    model_aliases:
      openai-gpt-4o-mini: openai/gpt-4o-mini
      openai-gpt-4o: openai/gpt-4o
      anthropic-claude-4.6-sonnet: anthropic/claude-sonnet-4-6

# OPTIONAL for the routing budget — see below. Needed for §4 (real caching on
# marker-based models when Plano proxies the request).
# prompt_caching:
#   enabled: true

routing:
  routing_budget:                 # no default — presence turns it on
    max_overhead_pct: 20          # bill at most 20% above never-switching
    # replenish_on_rebind: true   # reset running totals when a cold session re-binds
    # cache_read_discount: 0.1    # fallback when a feed omits cache_read
```

`models.dev` works as a drop-in alternative cost source (`provider: models.dev`,
no aliases needed — its keys already match `provider/model` routing names). The
demo models are priced identically on both feeds, so every number in this guide
holds either way.

The routing budget is fully self-sufficient: configuring it turns on implicit
session derivation and prices warm anchors at cached rates on its own —
`prompt_caching` is **not** required. Enable `prompt_caching` for what it adds:
injecting provider cache-control markers when Plano proxies the request (without
markers, marker-based models like `anthropic/*` never actually cache — see §4)
and session affinity when no budget is configured.



### DigitalOcean-hosted models

To route to DO GenAI models themselves (not just price from the DO catalog),
address them with the `digitalocean/` prefix and alias the catalog keys to
those names:

```yaml
model_providers:
  - model: digitalocean/anthropic-claude-4.6-sonnet
    access_key: $DIGITALOCEAN_API_KEY
    default: true
  - model: digitalocean/openai-gpt-4o
    access_key: $DIGITALOCEAN_API_KEY

model_metrics_sources:
  - type: cost
    provider: digitalocean        # DO catalog
    refresh_interval: 86400
    model_aliases:
      anthropic-claude-4.6-sonnet: digitalocean/anthropic-claude-4.6-sonnet
      openai-gpt-4o: digitalocean/openai-gpt-4o
```

> The DO catalog publishes cached-read rates
> (`cache_read_input_price_per_million`), so the gate prices warm anchors at the
> real cached rate. The `cache_read_discount` fallback only kicks in for models
> whose catalog entry omits the field.

---



## 3. Run it

```bash
# From this directory. --with-tracing starts a local OTLP collector on :4317.
planoai up config.yaml --with-tracing

# Tail logs (cache injections, pin events, switch decisions)
planoai logs --follow

# Stop
planoai down
```

The model listener comes up on **:12000** (per `config.yaml`).

---



## 4. See caching in action (single model)

This section needs `prompt_caching: { enabled: true }` (uncomment it in
`config.yaml`): the model here is Anthropic-family, which only caches when the
request carries cache-control markers, and it's Plano proxying the request —
so Plano must inject them. Send the same large system prompt across several
turns. Plano derives an implicit session from the stable prefix and pins the
model, so turns 2+ read the prefix from the provider cache.

```bash
curl -s localhost:12000/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "digitalocean/anthropic-claude-4.6-sonnet",
    "messages": [
      {"role": "system", "content": "you are an intelligent agent"},
      {"role": "user", "content": "Scaffold the service"}
    ]
  }' | jq '.usage'
```

Watch `usage.prompt_tokens_details.cached_tokens` climb from 0 on turn 1 to
(nearly) the full prefix on later turns, and the billed cost fall accordingly —
this is exactly the ~4× per-turn drop in the caching-ON vs -OFF comparison.

---



## 5. See the routing budget in action (model switch)

The budget is consulted only when the router proposes a model that differs from
the session's warm anchor. Warmth is inferred from how long ago the session was
last used vs. the provider's cache window (no per-call cache-hit signal needed).
To observe it:

- **Vetoed switch (paid, over cap):** with a warm session on an expensive model
and a large context, a switch to a pricier candidate would push the session's total
switch spend past `max_overhead_pct`% of its never-switch baseline → Plano **retains**
the anchor.
- **Paid switch (within cap):** the same switch while the spend still fits under the
cap → Plano **switches** and adds `switch_cost` to the session's cumulative spend.
- **Free switch (cheaper candidate):** a candidate whose *uncached* input rate
undercuts the anchor's *cached* rate → switch cost ≤ 0 → Plano **switches** for free
(the spend is not reduced).
- **Cold session:** the session went idle past the provider cache window → treated
as cold → the router's pick is dispatched with no penalty (and the running totals
reset on `replenish_on_rebind`).

Each decision is emitted to metrics and traces (below) with a `reason` label
(`same_anchor | free | within_cap | over_cap | no_pricing`).

---



## 6. Run it as a routing decision (no proxying)

Everything above sends the full request through Plano, which then calls the
upstream model itself. There's a second entry point that only makes the
*decision* — same session lookup, same warmth inference, same routing budget —
without ever calling an LLM or seeing a response: `/routing` + the same
API path, on the same host:port as the model listener.

This is for callers who want to make the actual upstream call themselves (or
need it embedded in a broader pipeline, e.g. an intelligent-routing layer)
but still want Plano's cache-aware pick and fallback order.

Send a normal request with a **system prompt** and a user message — no affinity
header. Plano derives the session key implicitly from
`hash(system + tools + first user message)`, so you can watch the session pin
and go warm across turns (the zero-config path from §4, now visible on the
decision endpoint via `session_id` / `pinned`). Send `openai/gpt-4o-mini` as
`model`; the router picks the real model from the *message content*.

**Turn 1 — pin the session** with a generation prompt:

```bash
curl -s localhost:12000/routing/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"model": "openai/gpt-4o-mini", "messages": [
    {"role": "system", "content": "You are a senior Rust engineer."},
    {"role": "user", "content": "Write a Rust function that reverses a linked list."}
  ]}' | jq '{model: .models[0], session_id, pinned}'
```

```json
{
  "model": "anthropic/claude-sonnet-4-6",
  "session_id": "implicit:8e76b367cc3a4336",
  "pinned": false
}
```

The router classified this as `code generation` → `anthropic/claude-sonnet-4-6`.
`session_id` is the implicit key Plano derived from `system + tools + first
user message` (deterministic — you'll get the same hash for these exact
payloads), and `pinned` is `false` because this call *creates* the binding.

**Turn 2 — same system prompt + same first message**, one turn later, within ~5 min:

```bash
curl -s localhost:12000/routing/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"model": "openai/gpt-4o-mini", "messages": [
    {"role": "system", "content": "You are a senior Rust engineer."},
    {"role": "user", "content": "Write a Rust function that reverses a linked list."},
    {"role": "assistant", "content": "Here is an idiomatic in-place reversal for a singly linked list:\n\n```rust\ntype Link = Option<Box<Node>>;\n\nstruct Node {\n    val: i32,\n    next: Link,\n}\n\nfn reverse(mut head: Link) -> Link {\n    let mut prev: Link = None;\n    while let Some(mut node) = head {\n        head = node.next.take();\n        node.next = prev;\n        prev = Some(node);\n    }\n    prev\n}\n```\n\nIt walks the list once, moving the next pointer of each node to its predecessor."},
    {"role": "user", "content": "Now explain its time complexity in plain English — no code."}
  ]}' | jq '{model: .models[0], session_id, pinned}'
```

```json
{
  "model": "anthropic/claude-sonnet-4-6",
  "session_id": "implicit:8e76b367cc3a4336",
  "pinned": true
}
```

The `session_id` is **identical** to turn 1 — the head of the prompt
(`system` + first user message) didn't change, so the implicit key stays stable
as history grows — and `pinned` is now `true`: the session is warm and stuck to
its anchor. **That's the pinning.**

**Now the budget:** here the router *did* read turn 2 as a different route
(`code understanding` → `openai/gpt-4o`), so with the anchor warm on
`claude-sonnet-4-6` the budget evaluated the switch — and vetoed it. The
brightstaff log shows exactly why:

```text
switch vetoed — would exceed session overhead cap, retaining anchor
  anchor=anthropic/claude-sonnet-4-6 candidate=openai/gpt-4o
  switch_cost_in_usd=3.96e-5 switch_spend_in_usd=0.0 overhead_ceiling_in_usd=1.08e-6
```

The switch would cost ~$3.96e-5 to re-read the context on `gpt-4o`, but only
~$1.08e-6 of overhead was affordable (`max_overhead_pct`% of the still-tiny
one-turn baseline) — so `.models[0]` stays `anthropic/claude-sonnet-4-6`. Confirm
it with the metric:

```bash
curl -s localhost:9092/metrics | grep session_switch_decisions
```

```text
brightstaff_session_switch_decisions_total{decision="retained",reason="over_cap"} 1
```

To see the **other** side, remove the `routing_budget` block (or set
`max_overhead_pct` very high), restart, and repeat — turn 2 now returns
`"model": "openai/gpt-4o"` and the metric reads `decision="allowed",reason="free"`.
That before/after — same calls, one config line — is the whole point: the
router's quality pick wins *unless* the budget says the warm cache it burns
isn't worth it.

> **If you see `same_anchor`,** the router classified both turns the same way,
> so no switch was proposed — inherent to appended conversations, since the
> router weighs the whole thread. To force `candidate ≠ anchor`
> deterministically, pin the session with an explicit header instead
> (`-H 'X-Model-Affinity: budget-demo'`) and send two *standalone* one-line
> prompts that each route to a different model (an "explain this code" prompt,
> then a "write a function" prompt). The budget behaves identically regardless
> of how the session key was derived — `route()` doesn't branch on it.

### Interoperability with the full-proxy path

Because this endpoint **shares the same session cache and the same
`session_router::route()` logic** as the full-proxy path, the two are fully
interoperable: a session pinned via `/routing` is honored by a later
`/v1/chat/completions` call (and vice versa), including the exact same
`max_overhead_pct` gating. This is also why warmth here is inferred purely
from idle-time vs. the provider's cache window rather than a cache-hit signal
— this path never has a provider response to read one from.

---



## 7. Observability (for evals & benchmarks)

**Prometheus metrics** — brightstaff exposes `/metrics` on **:9092**
(Envoy admin/stats on **:9901/stats**):


| Metric                                                                             | What it tells you                                                    |
| ---------------------------------------------------------------------------------- | -------------------------------------------------------------------- |
| `brightstaff_session_switch_decisions_total{decision="allowed"|"retained",reason}` | How often the budget let a switch through vs. vetoed it, and why      |
| `brightstaff_prompt_cache_requests_total{provider,model,outcome="hit"|"miss"}`     | Real provider cache hit rate                                         |
| `brightstaff_session_cache_events_total{outcome}`                                  | Session binding lookups/stores                                      |


```bash
curl -s localhost:9092/metrics | grep -E 'session_switch_decisions|prompt_cache_requests'
```

**Traces** — run with `--with-tracing` and inspect the routing span per request:

- `plano.cache.warm` — whether the session's cache was considered warm this turn
- `plano.cache.idle_ms` — how long since the session was last used
- `plano.switch.cost_in_usd` — actual input-token cost of the proposed switch (output excluded)
- `plano.switch.candidate_warm_tokens` — context the candidate still has cached from an earlier visit this session (a return to a warm model re-reads only the delta, so its `cost_in_usd` is far lower than a full re-ingest)
- `plano.switch.overhead_ceiling_in_usd` — overhead ceiling (`max_overhead_pct`% x baseline) when the switch was evaluated
- `plano.switch.decision` — `allowed` or `retained`
- `plano.session.overhead_pct` — cumulative switching overhead consumed, as a % of the never-switch baseline (compare directly to `max_overhead_pct`)
- `plano.session.switch_spend_in_usd` — cumulative $ actually spent on switches this session
- `plano.session.baseline_in_usd` — cumulative $ staying on the anchor would have cost (the denominator)
- `plano.session.switches` — switches taken so far this session
- `plano.session.total_cost_in_usd` — cumulative *actual* conversation cost (input +
  output), priced from the catalog and refined from real usage each turn (reflects cost
  through the previous turn, since this turn isn't billed yet at decision time)
- `plano.switch.counterfactual_route` — on a `retained` decision, the route the gate
  *would* have taken had the switch been allowed (only when `record_counterfactual: true`)
- `plano.session_id`, `plano.route.name`

Per-request cost also lands on each `plano(llm)` span (sum by `plano.session_id` for the
conversation total, or read `plano.session.total_cost_in_usd` off the routing span):

- `llm.usage.input_cost_usd` — uncached input at the input rate, cached reads at the
  cached rate, cache creation at the plain input rate
- `llm.usage.output_cost_usd` — completion tokens x output rate
- `llm.usage.total_cost_usd` — input + output

**Grafana** — a ready dashboard + compose live in `config/grafana/`
(`docker compose up` there, using `prometheus_scrape.yaml`).

---



## 8. A/B methodology (baseline vs treatment)

The cleanest benchmark is same-workload, caching off vs on — the exact shape of
the caching-ON/OFF comparison:

- **Baseline (no caching):** send requests with header `X-Plano-Cache: off`
(disables implicit pinning + marker injection per request), or run with
`prompt_caching` absent/disabled.
- **Treatment (caching on):** the config in this folder with the
`prompt_caching` block uncommented.

Compare, over an identical multi-turn eval set:

- total `prompt_tokens` billed at the uncached vs cached rate,
- `cached_tokens` ratio (cache hit rate),
- total USD cost,
- and — for routing-heavy workloads — `session_switch_decisions_total` and the
per-request `plano.switch.*` attributes to confirm switches happen only when
affordable.

```bash
# Baseline call (caching bypassed)
curl -s localhost:12000/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -H 'X-Plano-Cache: off' \
  -d '{ ... }' | jq '.usage'
```

---



## 9. Knobs to sweep


| Setting                                          | Effect                                                                          |
| ------------------------------------------------ | ------------------------------------------------------------------------------- |
| `routing.routing_budget.max_overhead_pct`        | Switching overhead cap as a % of never-switching (higher = quality-first, more switching) |
| `routing.routing_budget.replenish_on_rebind`     | Reset the running baseline/spend totals when a cold session re-binds            |
| `routing.routing_budget.cache_read_discount`     | Assumed cached rate for models whose feed entry omits a cached-read rate       |
| `routing.routing_budget.record_counterfactual`   | Emit `plano.switch.counterfactual_route` on vetoed switches (the road not taken)|
| `prompt_caching.session_ttl_seconds`             | Session binding GC lifetime                                                     |
| `prompt_caching.min_prefix_tokens`               | Minimum stable-prefix size before markers are injected                          |
| Header `X-Model-Affinity: <id>`                  | Explicit session key (overrides the implicit prefix hash)                       |
| Header `X-Plano-Cache: off`                      | Per-request bypass for baseline runs                                            |


---



## Notes

- Caching **never** changes which model routing selects — the router still makes
the quality call; the overhead cap only vetoes a switch that the session can't afford.
- The routing budget is independent of prompt caching (it lives under `routing`,
needs no `prompt_caching` config, and always prices warm anchors at cached rates)
and is fully opt-in with **no baked-in cap**: configuring it without a
`max_overhead_pct` (or without a cost source) fails startup with a clear message.
