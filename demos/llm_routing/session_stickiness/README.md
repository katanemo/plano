# Session Stickiness + Cache-Regret Cost Gate

Preference-based routing with automatic prompt caching, plus a cost gate that
protects warm provider caches when a re-route is being considered.

## The problem

Provider prompt caches are per-model. When intelligent routing moves a
conversation to a different model, the new model re-ingests the full context at
its **uncached** input rate. In input-heavy, append-only workloads (coding
agents especially), a nominally cheaper model can end up more expensive than
the cached rate you abandoned.

## What the gate does

Plano already sticks: a warm session pin with a stable prompt prefix
short-circuits routing entirely. That path is untouched. The gate acts only
when a re-route is already triggered — the pin expired — and the previous
model's cache is plausibly still warm (it actually observed cache hits and the
prompt prefix has not drifted). At that boundary Plano computes the input-cost
regret of switching:

```
regret_usd = context_tokens x (candidate_uncached_input - previous_cached_input) / 1M
```

- **Regret <= 0** — the candidate's uncached rate undercuts the warm model's
  cached rate. Losing the cache costs nothing; switch freely.
- **Regret > 0** — compared against your `switch_cost` threshold. Within it,
  the switch proceeds; above it, Plano retains the previous model and the warm
  cache.

The math is input-only by design: output-token savings are never credited,
because output counts are unknowable before generation (reasoning models can
emit many times more tokens for the same task). Quality and cost stay separate
— the router still picks the best model; the gate only vetoes unaffordable
switches.

## Configuration

See [config.yaml](config.yaml). Requirements:

- `prompt_caching.enabled: true` (the gate builds on session affinity)
- a cost source in `model_metrics_sources` (per-model rates feed the regret math)
- a `switch_cost` threshold — there is no default; startup fails without one

Two threshold forms:

| Form | Meaning |
|---|---|
| `max_regret_usd` | Absolute USD ceiling per switch (e.g. `0.10` = ten cents) |
| `max_regret_pct_of_cached` | Ceiling relative to what staying on the warm model would cost (e.g. `200` = 2x) |

## Observability

Every gate decision is visible:

- Metric: `brightstaff_session_switch_decisions_total{decision="allowed"|"retained"}`
- Span attributes: `plano.switch.regret_usd`, `plano.switch.threshold_usd`,
  `plano.switch.decision`

## Run

```bash
planoai up config.yaml
```
