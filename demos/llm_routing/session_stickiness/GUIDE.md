# How-To: See Prompt Caching + Session Stickiness in Action

A hands-on guide for running Plano's automatic prompt caching and the cache-regret cost gate locally, and for measuring the win in evals/benchmarks (e.g. on DigitalOcean models).

There are two behaviors to observe:

1. **Automatic prompt caching** — staying on one model keeps the stable prefix
  warm, so per-turn input cost drops sharply across a multi-turn conversation.
2. **Session stickiness + cache-regret cost gate** — when the router would
  re-route to a *different* model, Plano only switches if the input-cost of
   abandoning the warm cache stays within a threshold you define.

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
# Per-model pricing is REQUIRED for the cost gate — the regret math needs each
# model's input and cached-input rates.
model_metrics_sources:
  - type: cost
    provider: models.dev          # publishes real cache_read rates
    refresh_interval: 86400

prompt_caching:
  enabled: true                   # automatic caching + session affinity (opt-in)

  session_stickiness:
    enabled: true
    switch_cost:                  # no default — you must set one
      type: max_regret_usd        # or: max_regret_pct_of_cached
      value: 0.10
    # cache_read_discount: 0.1    # fallback when a feed omits cache_read
```



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



## 5. See the cost gate in action (model switch)

The gate fires only on a **re-route** (the pin expired or the prefix drifted)
where the previous model's cache was warm. To observe it:

- **Vetoed switch (Case 3):** with a warm pin on an expensive model and a large
context, a re-route to a pricier candidate produces regret above your
`max_regret_usd` → Plano **retains** the previous model.
- **Allowed switch (Case 1):** a re-route to a model whose *uncached* input rate
undercuts the previous model's *cached* rate → regret ≤ 0 → Plano **switches**.
- **Free switch (drift):** change the system prompt (stable prefix changes) → the
old cache is already cold → Plano re-routes with no gate penalty.

Each decision is logged (`switch vetoed …` / `switch allowed …`) and emitted to
metrics and traces (below).

---



## 6. Observability (for evals & benchmarks)

**Prometheus metrics** — brightstaff exposes `/metrics` on **:9092**
(Envoy admin/stats on **:9901/stats**):


| Metric                                                                         | What it tells you                                                                              |
| ------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------- |
| `brightstaff_session_switch_decisions_total{decision="allowed"|"retained"}`    | How often the gate let a switch through vs. vetoed it                                          |
| `brightstaff_prompt_cache_requests_total{provider,model,outcome="hit"|"miss"}` | Real provider cache hit rate                                                                   |
| `brightstaff_session_pin_events_total{event}`                                  | Pin lifecycle: `implicit_commit`, `refresh`, `prefix_drift`, `stale_hint`, `validation_failed` |
| `brightstaff_session_cache_events_total{outcome}`                              | Session pin lookups/stores                                                                     |


```bash
curl -s localhost:9092/metrics | grep -E 'session_switch_decisions|prompt_cache_requests'
```

**Traces** — run with `--with-tracing` and inspect the routing span per request:

- `plano.switch.regret_usd` — estimated input-cost regret of the proposed switch
- `plano.switch.threshold_usd` — the resolved ceiling it was compared against
- `plano.switch.decision` — `allowed` or `retained`
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


| Setting                                               | Effect                                                                                    |
| ----------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| `prompt_caching.session_stickiness.switch_cost.value` | How much cache regret you'll tolerate per switch (higher = quality-first, more switching) |
| `switch_cost.type`                                    | Absolute USD vs. percent-of-cached (scales with context size)                             |
| `cache_read_discount`                                 | Assumed cached rate when a feed omits `cache_read` (DO fallback)                          |
| `record_counterfactual`                               | Emit `plano.switch.counterfactual_route` on vetoed switches (the road not taken)          |
| `prompt_caching.session_ttl_seconds`                  | Pin lifetime; align with the provider's cache window                                      |
| `prompt_caching.min_prefix_tokens`                    | Minimum stable-prefix size before markers are injected                                    |
| Header `X-Model-Affinity: <id>`                       | Explicit session pin (overrides the implicit prefix hash)                                 |
| Header `X-Plano-Cache: off`                           | Per-request bypass for baseline runs                                                      |


---



## Notes

- Caching **never** changes which model routing selects — the router still makes
the quality call; the gate only vetoes a switch that isn't affordable.
- The gate is fully opt-in and has **no baked-in threshold**: enabling it without
a `switch_cost` (or without a cost source) fails startup with a clear message.
