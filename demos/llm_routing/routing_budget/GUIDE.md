# How-To: See Prompt Caching + Routing Budget in Action

A hands-on guide for running Plano's automatic prompt caching and the routing budget locally, and for measuring the win in evals/benchmarks (e.g. on DigitalOcean models).

There are two independent behaviors to observe:

1. **Automatic prompt caching** — staying on one model keeps the stable prefix
  warm, so per-turn input cost drops sharply across a multi-turn conversation.
2. **Routing budget** — the router still runs every turn, but when it proposes a
  *different* model while the session's cache is plausibly warm, Plano only switches
  while the session's cumulative budget covers the input-cost of abandoning that
  cache. This is a routing concern and works whether or not prompt caching is on.

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
# each model's input and cached-input rates.
model_metrics_sources:
  - type: cost
    provider: models.dev          # publishes real cache_read rates
    refresh_interval: 86400

prompt_caching:
  enabled: true                   # automatic caching + session affinity (separate concern)

routing:
  routing_budget:                 # no default — presence turns it on
    seed_usd: 0.50                # cumulative budget for quality-driven switches
    # replenish_on_rebind: true   # re-seed when a cold session re-binds
    # cache_read_discount: 0.1    # fallback when a feed omits cache_read
```

The routing budget lives under `routing` and is independent of prompt caching — it
applies whether or not `prompt_caching.enabled` is set.



### DigitalOcean variant

Address DO GenAI models with the `digitalocean/` prefix and point the cost feed
at the DO catalog (or keep `models.dev`, which publishes cached-read rates the
DO catalog doesn't):

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
```

> The DO catalog does not publish a cached-read rate, so for DO-only setups the
> gate falls back to `input_rate × cache_read_discount`. For exact cached rates,
> add a `models.dev` cost source instead.

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

Send the same large system prompt across several turns. With caching enabled,
Plano derives an implicit session from the stable prefix and pins the model, so
turns 2+ read the prefix from the provider cache.

```bash
curl -s localhost:12000/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "digitalocean/anthropic-claude-4.6-sonnet",
    "messages": [
      {"role": "system", "content": "<paste a few thousand tokens of stable context>"},
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

- **Vetoed switch (paid, over budget):** with a warm session on an expensive model
and a large context, a switch to a pricier candidate costs more than the session's
remaining budget → Plano **retains** the anchor.
- **Paid switch (within budget):** the same switch while budget remains → Plano
**switches** and debits `switch_cost` from the session budget.
- **Free switch (cheaper candidate):** a candidate whose *uncached* input rate
undercuts the anchor's *cached* rate → switch cost ≤ 0 → Plano **switches** for free
(the budget is not credited back).
- **Cold session:** the session went idle past the provider cache window → treated
as cold → the router's pick is dispatched with no budget penalty (and the budget
re-seeds on `replenish_on_rebind`).

Each decision is emitted to metrics and traces (below) with a `reason` label
(`same_anchor | free | within_budget | over_budget | no_pricing`).

---



## 6. Observability (for evals & benchmarks)

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
- `plano.switch.threshold_in_usd` — budget remaining when the switch was evaluated
- `plano.switch.decision` — `allowed` or `retained`
- `plano.session.budget_remaining_in_usd` — switch budget left after this turn
- `plano.session.switches` — switches taken so far this session
- `plano.switch.counterfactual_route` — on a `retained` decision, the route the gate
  *would* have taken had the switch been allowed (only when `record_counterfactual: true`)
- `plano.session_id`, `plano.route.name`

**Grafana** — a ready dashboard + compose live in `config/grafana/`
(`docker compose up` there, using `prometheus_scrape.yaml`).

---



## 7. A/B methodology (baseline vs treatment)

The cleanest benchmark is same-workload, caching off vs on — the exact shape of
the caching-ON/OFF comparison:

- **Baseline (no caching):** send requests with header `X-Plano-Cache: off`
(disables implicit pinning + marker injection per request), or run with
`prompt_caching.enabled: false`.
- **Treatment (caching on):** default config in this folder.

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



## 8. Knobs to sweep


| Setting                                          | Effect                                                                          |
| ------------------------------------------------ | ------------------------------------------------------------------------------- |
| `routing.routing_budget.seed_usd`                | Cumulative budget per session (higher = quality-first, more switching)          |
| `routing.routing_budget.replenish_on_rebind`     | Re-seed the budget when a cold session re-binds                                 |
| `routing.routing_budget.cache_read_discount`     | Assumed cached rate when a feed omits `cache_read` (DO fallback)                |
| `routing.routing_budget.record_counterfactual`   | Emit `plano.switch.counterfactual_route` on vetoed switches (the road not taken)|
| `prompt_caching.session_ttl_seconds`             | Session binding GC lifetime                                                     |
| `prompt_caching.min_prefix_tokens`               | Minimum stable-prefix size before markers are injected                          |
| Header `X-Model-Affinity: <id>`                  | Explicit session key (overrides the implicit prefix hash)                       |
| Header `X-Plano-Cache: off`                      | Per-request bypass for baseline runs                                            |


---



## Notes

- Caching **never** changes which model routing selects — the router still makes
the quality call; the budget only vetoes a switch that the session can't afford.
- The routing budget is independent of prompt caching (it lives under `routing`) and
is fully opt-in with **no baked-in budget**: configuring it without a `seed_usd`
(or without a cost source) fails startup with a clear message.
