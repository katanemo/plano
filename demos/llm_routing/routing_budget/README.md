# Routing Budget

Preference-based routing with a cumulative per-session **routing budget** that
protects warm provider caches, plus automatic prompt caching. The budget is a
routing concern configured under `routing` — independent of prompt caching.

## The problem

Provider prompt caches are per-model. When intelligent routing moves a
conversation to a different model, the new model re-ingests the full context at
its **uncached** input rate. In input-heavy, append-only workloads (coding
agents especially), a nominally cheaper model can end up more expensive than
the cached rate you abandoned.

## What the routing budget does

The router runs every turn (routing stays cache-blind). When it proposes a model
that differs from the session's warm anchor, Plano computes the **actual
input-token cost** of abandoning the anchor's cache:

```
switch_cost_in_usd = context_tokens x (candidate_uncached_input - anchor_cached_input) / 1M
```

- **Switch cost <= 0** — the candidate's uncached rate undercuts the anchor's
  cached rate. Losing the cache costs nothing; switch freely (and, with
  `credit_negative`, credit the budget back).
- **Switch cost > 0** — drawn from the session's remaining budget. If the budget
  covers it, the switch proceeds and the budget is debited; otherwise Plano
  retains the anchor and its warm cache.

Warmth is inferred from how long ago the session was last used vs. the
provider's cache window — no per-call cache-hit signal is required, so the same
decision works on both the full-proxy and `/routing` decision paths.

The math is **input-only by design**: output-token cost is deliberately excluded,
because output length is unknowable before generation. Quality and cost stay
separate — the router still picks the best model; the budget only vetoes switches
the session can't afford.

## Configuration

See [config.yaml](config.yaml). Requirements:

- a cost source in `model_metrics_sources` (per-model rates feed the switch cost math)
- a `routing.routing_budget` block — there is no default; presence turns it on and
  startup fails without a `seed_usd` (or without a cost source)

`routing.routing_budget` fields:

| Field | Meaning |
|---|---|
| `seed_usd` | Cumulative budget (USD) per session. `0` = never pay to switch |
| `replenish_on_rebind` | Re-seed the budget when a cold session re-binds (default true) |
| `credit_negative` | Credit the budget back on outright-cheaper switches (default true) |
| `cache_read_discount` | Assumed cached rate when a feed omits `cache_read` (default 0.1) |
| `record_counterfactual` | Record the switch that was vetoed, as a trace attribute (default false) |

Prompt caching (`prompt_caching.enabled`) is a separate, optional concern that keeps
the upstream cache warm and injects provider cache-control markers.

## Observability

Every decision is visible:

- Metric: `brightstaff_session_switch_decisions_total{decision="allowed"|"retained",reason}`
  (`reason` ∈ `same_anchor | free | within_budget | over_budget | no_pricing`)
- Span attributes: `plano.cache.warm`, `plano.cache.idle_ms`,
  `plano.switch.cost_in_usd`, `plano.switch.threshold_in_usd`, `plano.switch.decision`,
  `plano.session.budget_remaining_in_usd`, `plano.session.switches`

## Run

```bash
planoai up config.yaml
```
