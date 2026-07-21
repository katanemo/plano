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
input-token cost** of abandoning the anchor's cache and only allows the switch
while the session's cumulative switch spend stays within `max_overhead_pct`% of
what never-switching would have cost — otherwise it retains the warm anchor. The
promise: the conversation bills at most `max_overhead_pct`% above never-switching.

Quality and cost stay separate — the router still picks the best model; the
budget only vetoes switches the session can't afford. Prompt caching
(`prompt_caching.enabled`) is a separate, optional concern that keeps the
upstream cache warm and injects provider cache-control markers.

## Run

```bash
planoai up config.yaml
```

See [config.yaml](config.yaml) for the annotated configuration, and
**[GUIDE.md](GUIDE.md)** for the full hands-on walkthrough — running it as a
routing decision, watching a switch get vetoed vs. allowed, the switch-cost
math, and all the metrics and trace attributes for evals.
