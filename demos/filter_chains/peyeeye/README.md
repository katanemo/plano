# Peyeeye PII Filter Chain Demo

Drop-in PII redaction + rehydration for Plano via the [Peyeeye](https://peyeeye.ai) API.

The model never sees raw PII — incoming text is sent to `/v1/redact` and replaced with stable placeholders like `[EMAIL_0]`, `[PERSON_1]`, etc. After the model responds, the placeholders in its output are swapped back to the originals via `/v1/rehydrate`.

## Architecture

```
Client --> Plano (model listener :12000)
              |
              +-- input_filters: peyeeye_redact
              |     -> POST https://api.peyeeye.ai/v1/redact
              |     -> body messages contain [EMAIL_0], [SSN_0], ...
              |
              +-- model_provider: openai/gpt-4o-mini  (or claude, etc.)
              |     -> the LLM only ever sees redacted text
              |
              +-- output_filters: peyeeye_rehydrate
                    -> POST https://api.peyeeye.ai/v1/rehydrate
                    -> placeholders restored to originals
```

## Quick start

```bash
export PEYEEYE_API_KEY=pk_live_...     # https://peyeeye.ai
export OPENAI_API_KEY=sk-...

bash run_demo.sh

# in another terminal
bash test.sh

# stop
bash run_demo.sh down
```

## Try it

```bash
curl http://localhost:12000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "messages": [{"role": "user", "content": "Email me at jane@example.com about SSN 123-45-6789"}],
    "stream": false
  }'
```

The response body comes back with the original email and SSN restored, while the request that hit OpenAI carried `[EMAIL_0]` and `[SSN_0]`.

## Configuration

All configuration is via env vars on the filter service:

| Var | Default | Notes |
|---|---|---|
| `PEYEEYE_API_KEY` | _required_ | get one at peyeeye.ai |
| `PEYEEYE_API_BASE` | `https://api.peyeeye.ai` | override for self-hosted |
| `PEYEEYE_LOCALE` | `auto` | BCP-47 |
| `PEYEEYE_ENTITIES` | _all_ | comma-separated list, e.g. `EMAIL,SSN,CREDIT_CARD` |
| `PEYEEYE_SESSION_MODE` | `stateful` | `stateful` (default) or `stateless` |

In `stateless` mode, Peyeeye returns a sealed `skey_...` blob instead of holding the mapping server-side; this filter caches the blob on the request id and uses it for rehydration. No per-request state is retained on Peyeeye's servers.

## Filter contract

**Input filter (`/redact/{path:path}`)** receives the full raw request body. It walks `messages[].content` (string or multimodal `text` parts) for chat-style endpoints and `input` for the OpenAI Responses API, sends a single batched call to Peyeeye, and writes the redacted text back into the body in place.

**Output filter (`/rehydrate/{path:path}`)** receives the raw LLM response bytes, looks up the cached session id by the request id (`x-request-id`), and rehydrates `choices[].message.content`, Anthropic-style `content[].text`, or Responses-API `output[].content[].text`.

## Behavioral invariants

- **Fail-closed.** If `/v1/redact` returns an unexpected shape, or the count of returned texts doesn't match the count sent, the filter raises a 502 — no unredacted text is ever forwarded to the model.
- **Length-guard.** `len(redacted) == len(sent)` is asserted before zipping the values back into the request.
- **Typed errors.** `PEyeEyeMissingSecrets` covers auth (401, missing key), `PEyeEyeAPIError` covers everything else (timeouts, 4xx, 5xx, parse). Both surface as HTTP errors to Plano.
- **Best-effort cleanup.** Stateful sessions are deleted server-side after rehydration via `DELETE /v1/sessions/{ses_...}`.

## Streaming

Streaming SSE responses are passed through unchanged in this demo — token-by-token rehydration would require buffering or a session-aware streaming endpoint. For now, set `stream: false` on requests routed through this filter chain.

## Tests

```bash
uv sync --group dev
uv run pytest -v
```

The suite mocks the Peyeeye API (`respx`) and exercises:

- redact + rehydrate round trip on chat completions
- redact + rehydrate on `/v1/messages` (Anthropic) and `/v1/responses` (OpenAI)
- the length-guard (redact returns wrong count -> 502)
- the unexpected-shape guard (redact returns non-string/list -> 502)
- the no-PII passthrough (no redactable text -> body unchanged, no session cached)
- the no-cached-session passthrough on rehydrate
- multimodal `[{"type":"text","text":...}]` content lists

## Disclosure

I'm the maintainer of peyeeye.ai. Happy to adjust API surface, naming, or test coverage to match Plano's conventions.
