# GPU Free-Tier Arbitrage Demo

This demo package showcases provider-level free-tier-first routing and deterministic fallback using a local Plano endpoint on `localhost:12000`.

## Files

- `config.yaml` - demo Plano config with `arbitrage_policy`
- `demo.rest` - runnable REST requests for IDE REST clients

## Prerequisites

Set API keys for providers used in this demo:

- `OPENAI_API_KEY`
- `GROQ_API_KEY`
- `TOGETHER_API_KEY`

## Run the demo

From this directory:

```bash
planoai up config.yaml
```

Then run requests from `demo.rest` in your REST client.

## What to show during the demo

1. Run `free-tier-first showcase` and verify response success.
2. Inspect logs/traces for provider selection reason and selected candidate.
3. Force a retryable error on the first candidate (for example, temporarily invalid key), then run `fallback showcase`.
4. Verify fallback metadata appears in traces/logs:
   - `routing.selection_reason`
   - `routing.is_fallback`
   - `routing.fallback_trigger`
   - `routing.next_candidate`
   - `routing.upstream_endpoint`
