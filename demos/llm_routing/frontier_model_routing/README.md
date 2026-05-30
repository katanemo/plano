# Frontier Model Routing: Sonnet 4.6 + GPT 5.5 + Opus 4.7

A worked example of using Plano to route across the three current frontier
LLMs from three different providers — without your application caring which
model handled any given request, and with **per-route fallbacks** so a
provider outage never takes the demo down.

| Tier             | Primary model                          | Provider           | What it's great at                                       |
| ---------------- | -------------------------------------- | ------------------ | -------------------------------------------------------- |
| `frontier.fast`  | `anthropic-claude-sonnet-4-6`          | DigitalOcean       | Daily driver — chat, summaries, drafts, light reasoning  |
| `frontier.smart` | `gpt-5.5`                              | OpenAI             | Multi-step reasoning, math, tool/function calling        |
| `frontier.max`   | `claude-opus-4-7`                      | Anthropic          | Code, deep analysis, long-context evaluation, refactors  |

The same prompt picks the right model automatically — Plano's preference
aligned router (Plano-Orchestrator) reads the user's intent and dispatches to
the route whose `routing_preferences` description best matches. Each route
is backed by an **ordered candidate pool**, so when the primary provider
returns a `429`/`5xx` the next entry in the pool serves the request.

```
                        ┌────────────────────────────────────┐
client ──── /v1 ───▶    │  Plano gateway (port 12000)        │
(OpenAI / Anthropic /   │   ├── Plano-Orchestrator (router)  │
 Claude Desktop / SDK)  │   └── Envoy + brightstaff          │
                        └────────────────────────────────────┘
                              │              │             │
                  ┌───────────┘              │             └────────────┐
                  ▼                          ▼                          ▼
       DigitalOcean Gradient AI       OpenAI                    Anthropic
   anthropic-claude-sonnet-4-6      gpt-5.5                  claude-opus-4-7
   (daily conversation route)   (complex reasoning)      (code + deep analysis)
```

## Why this layout

- **Cost-quality fit per request.** Casual prompts go to Sonnet 4.6 on
  DigitalOcean (cheaper inference, still excellent quality); complex
  reasoning goes to GPT 5.5; code and deep analysis go to Opus 4.7.
- **Provider diversity = resilience.** Every route lists a fallback model
  from a different provider — if Anthropic rate-limits Opus, Plano hands
  the next request in that route to GPT 5.5 with no client changes.
- **Zero client changes.** The OpenAI SDK, Anthropic SDK, Claude Desktop,
  Codex CLI, and curl all hit the same `:12000` endpoint and use the same
  alias names. Switching `frontier.max` from Opus to whatever ships next
  is a one-line config change.

## The new routing-preferences architecture (v0.4.0)

This demo uses Plano's **top-level `routing_preferences`** block — the
canonical shape since `v0.4.0`. The older inline form (preferences nested
under each `model_provider`) is auto-migrated by the Plano CLI but emits a
deprecation warning. The top-level shape gives each route an ordered
candidate pool, which is what makes per-route fallbacks possible.

```yaml
routing_preferences:
  - name: code generation
    description: writing new functions, classes, scripts, or boilerplate; implementing APIs; producing unit tests
    models:
      - anthropic/claude-opus-4-7        # primary
      - openai/gpt-5.5                   # fallback on 429/5xx
```

What changes vs. the v0.3.0 inline style:

| Capability                                | v0.3.0 inline | v0.4.0 top-level |
| ----------------------------------------- | :-----------: | :--------------: |
| Multiple models can serve the same route  |       no      |        yes       |
| Explicit primary + ranked fallback chain  |       no      |        yes       |
| Per-request override via request body     |       no      |        yes       |
| Decision-only endpoint (`/routing/v1/...`)|       no      |        yes       |
| `X-Model-Affinity` header for agent loops |       no      |        yes       |

## Prerequisites

- **Plano CLI** — `uv tool install planoai` or `pip install planoai`
- API keys for all three providers:

  | Env var             | Where to get it                                                          |
  | ------------------- | ------------------------------------------------------------------------ |
  | `DO_API_KEY`        | <https://cloud.digitalocean.com/account/api/tokens> (Gradient AI access) |
  | `OPENAI_API_KEY`    | <https://platform.openai.com/api-keys>                                   |
  | `ANTHROPIC_API_KEY` | <https://console.anthropic.com/>                                         |

## Quick start

```bash
export DO_API_KEY=...
export OPENAI_API_KEY=...
export ANTHROPIC_API_KEY=...

cd demos/llm_routing/frontier_model_routing
./run_demo.sh
```

`run_demo.sh` writes a local `.env`, then runs `planoai up config.yaml`.
Plano daemonizes and is ready when the script returns.

To shut down:

```bash
./run_demo.sh down
```

## Try it

### Let Plano pick the right tier

```bash
./test.sh
```

The script does two things for each prompt:

1. Calls `POST /routing/v1/chat/completions` — Plano's **decision-only**
   endpoint — to print the matched route name and the ranked candidate
   pool for that prompt.
2. Calls `POST /v1/chat/completions` to actually run the request and
   prints the model that handled it.

A healthy run resolves like this:

```
[daily conversation -> expects DigitalOcean Sonnet 4.6]
  matched route:  daily conversation
  ranked models:  ["digitalocean/anthropic-claude-sonnet-4-6","openai/gpt-5.5"]
  routed_to:      digitalocean/anthropic-claude-sonnet-4-6

[complex reasoning -> expects OpenAI GPT 5.5]
  matched route:  complex reasoning
  ranked models:  ["openai/gpt-5.5","anthropic/claude-opus-4-7"]
  routed_to:      openai/gpt-5.5

[code generation -> expects Anthropic Opus 4.7]
  matched route:  code generation
  ranked models:  ["anthropic/claude-opus-4-7","openai/gpt-5.5"]
  routed_to:      anthropic/claude-opus-4-7
```

The trick: every request is sent with `model: frontier.fast`, but Plano runs
the orchestrator on every chat completion when `routing_preferences` are
configured and overrides the `model` when a preference matches. The
`frontier.fast` value is the explicit fallback used when no preference
matches — so casual prompts stay on the cheap tier and only "real" reasoning
or code work escalates to GPT 5.5 or Opus 4.7.

Want to watch the router decide live? In a second terminal:

```bash
planoai trace
```

You'll see the orchestrator's route selection for each request, including
the matched preference, ranked models, and response time.

### Inspect the routing decision without burning a token

The `/routing/v1/...` endpoint returns the routing decision **without
calling the upstream model**. Useful for previewing classification, building
a UI, or wiring fallback logic into a custom client.

```bash
curl -sS -X POST http://localhost:12000/routing/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "frontier.fast",
    "messages": [{"role":"user","content":"refactor this function to remove the global"}]
  }' | jq .
```

```json
{
  "models": ["anthropic/claude-opus-4-7", "openai/gpt-5.5"],
  "route": "code generation",
  "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736",
  "pinned": false
}
```

Use `models[0]` as the primary; retry with `models[1]` on `429` / `5xx`.

### Pin a route across an agent loop with `X-Model-Affinity`

In a tool-using agent loop a single user task may produce a dozen LLM
calls. Their topics drift (tool selection looks like code, summarising
results looks like analysis), and the router would otherwise route each
turn independently — bouncing between providers and invalidating their
KV caches. Pin the decision once with an arbitrary session id:

```bash
SID=$(uuidgen)

curl -sS -X POST http://localhost:12000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "X-Model-Affinity: $SID" \
  -d '{"model":"frontier.fast","messages":[{"role":"user","content":"start a refactor of the auth module"}]}'

# every subsequent call with the same SID skips routing and reuses the
# cached model decision until the session TTL (10 min by default) expires.
curl -sS -X POST http://localhost:12000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "X-Model-Affinity: $SID" \
  -d '{"model":"frontier.fast","messages":[{"role":"user","content":"now write the unit tests"}]}'
```

TTL and cache size are configurable under `routing:` in `config.yaml`.

### Override the routing policy per-request

Sometimes one caller needs a different policy without redeploying the
gateway. Send `routing_preferences` inline in the request body — it is
stripped before forwarding upstream:

```bash
curl -sS -X POST http://localhost:12000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "frontier.fast",
    "messages": [{"role":"user","content":"draft me a haiku about Postgres"}],
    "routing_preferences": [
      {
        "name": "creative writing",
        "description": "poetry, fiction, lyrical or playful prose",
        "models": ["anthropic/claude-opus-4-7", "openai/gpt-5.5"]
      }
    ]
  }' | jq .
```

### Pin a request to a specific tier (skip routing)

For prompts that don't match any preference description, the requested
model is what serves the request. Pin to a tier by sending its alias
directly:

```bash
# DigitalOcean Sonnet 4.6 — fast and cheap
curl -sS -X POST http://localhost:12000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model":"frontier.fast","messages":[{"role":"user","content":"hello"}]}' | jq .

# OpenAI GPT 5.5
curl -sS -X POST http://localhost:12000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model":"frontier.smart","messages":[{"role":"user","content":"hello"}]}' | jq .

# Anthropic Opus 4.7
curl -sS -X POST http://localhost:12000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model":"frontier.max","messages":[{"role":"user","content":"hello"}]}' | jq .
```

### From a Claude-native client (Anthropic Messages API)

Plano translates between OpenAI and Anthropic shapes, so the same gateway
serves both client SDKs:

```bash
curl -sS -X POST http://localhost:12000/v1/messages \
  -H "Content-Type: application/json" \
  -H "anthropic-version: 2023-06-01" \
  -H "x-api-key: test-key" \
  -d '{
    "model": "frontier.max",
    "max_tokens": 512,
    "messages": [{"role":"user","content":"explain CAP theorem like I have a CS undergrad background"}]
  }' | jq .
```

### From Claude Desktop

Once Plano is up, point Claude Desktop at it with one command:

```bash
planoai launch claude-desktop --config config.yaml
```

Claude Desktop will switch into third-party gateway mode pointed at
`http://localhost:12000`, auto-discover the three model aliases via
`/v1/models`, and let you pick `frontier.fast` / `.smart` / `.max` from the
in-app model selector. To revert: `planoai launch claude-desktop --restore`.

### From Codex CLI

```bash
planoai launch codex
codex --model frontier.smart   # or frontier.fast / frontier.max
```

### From the Claude Code CLI

```bash
planoai launch claude-cli
```

The CLI will use Plano as its Anthropic endpoint; ask it for code-heavy work
and it'll resolve to Opus 4.7 automatically.

## Config walkthrough

[`config.yaml`](config.yaml) declares each provider once, then declares
**top-level routing preferences** that reference those providers by their
full `<provider>/<model>` name. Each route owns an ordered `models` pool —
primary first, fallbacks next.

```yaml
model_providers:
  - model: digitalocean/anthropic-claude-sonnet-4-6
    access_key: $DO_API_KEY
    default: true                         # used when no preference matches
  - model: openai/gpt-5.5
    access_key: $OPENAI_API_KEY
  - model: anthropic/claude-opus-4-7
    access_key: $ANTHROPIC_API_KEY

routing_preferences:
  - name: code generation
    description: writing new functions, classes, scripts, or boilerplate; implementing APIs; producing unit tests; refactoring code
    models:
      - anthropic/claude-opus-4-7         # primary
      - openai/gpt-5.5                    # fallback on 429 / 5xx

  - name: deep analysis
    description: long-form analysis, architecture review, security review, evaluating tradeoffs, structured critique
    models:
      - anthropic/claude-opus-4-7
      - openai/gpt-5.5

  - name: complex reasoning
    description: multi-step reasoning, mathematical problem solving, structured planning, tool and function calling, data extraction
    models:
      - openai/gpt-5.5
      - anthropic/claude-opus-4-7

  - name: daily conversation
    description: general chat, casual Q&A, summaries, drafting messages, quick rewrites
    models:
      - digitalocean/anthropic-claude-sonnet-4-6
      - openai/gpt-5.5

model_aliases:
  frontier.fast:  { target: anthropic-claude-sonnet-4-6 }
  frontier.smart: { target: gpt-5.5 }
  frontier.max:   { target: claude-opus-4-7 }
```

A few things to call out:

1. **Preference *descriptions* drive routing accuracy.** They're embedded
   into the orchestrator's prompt; vague descriptions = vague routing.
   Following the [LLM Routing best practices](../../../docs/source/guides/llm_router.rst):
   - keep names specific and non-overlapping,
   - prefer noun-centric descriptors over imperative phrasing,
   - always include a generic "domain"-style route — here that's
     `daily conversation` pinned to the cheapest tier — so unmatched
     prompts still land somewhere deliberate.
2. **Ordered `models`** is a candidate pool. `models[0]` is the primary;
   anything after it is a fallback that the client (or Plano's retry
   logic) tries on `429`/`5xx`. Mix providers across the pool so a single
   provider outage doesn't break the route.
3. **The `default: true` provider** is the safety net for prompts the
   orchestrator can't confidently classify (e.g. one-word "thanks!").
4. **Aliases** decouple your callers from provider/model strings. When the
   next Sonnet ships, change the alias target — every caller picks it up
   instantly.

## Tracing

`tracing.random_sampling: 100` in the config enables full OTLP tracing. Open
a second terminal and run:

```bash
planoai trace
```

Each routed call shows up with the matched preference, ranked candidate
pool, selected model, end-to-end latency, and per-stage spans (router
decision, provider call, streaming chunks).

## Cost framing

A rough mix of 60% conversation, 30% reasoning, 10% deep code work — say
1,000 prompts/day at 1k input + 500 output tokens each — illustrates why
this layout pays off. Exact numbers depend on per-provider pricing the day
you read this; the point is that calling Opus 4.7 for casual chat is wasted
spend, and falling back to a small model on complex code is wasted output.
Plano's job is to let each provider do what it's best at, and to fail over
to the next entry in `models` when the primary throttles.

## Customizing

- **Swap a provider:** change the model string and `access_key`. e.g.
  point `frontier.smart` at `azure_openai/gpt-5.5` by replacing the OpenAI
  block with an Azure block, then update the matching entries inside
  `routing_preferences[].models`.
- **Add fallbacks:** append more entries to any route's `models` list.
  The orchestrator returns the full ranked pool, and Plano (or your
  client) walks it on `429`/`5xx`.
- **Add a new route:** add another entry under `routing_preferences` with
  a noun-centric description and its own `models` pool. No code change,
  no client change — every existing caller benefits immediately.
- **Per-call policy override:** ship a `routing_preferences` field in the
  request body to override the config for that one call (see the curl
  example above).
- **Self-host the orchestrator:** see
  [`../preference_based_routing/plano_config_local.yaml`](../preference_based_routing/plano_config_local.yaml)
  for an Ollama-backed orchestrator. Drop the `overrides.llm_routing_model`
  block into this config and you're off the hosted Plano-Orchestrator.

## Files

| File                                          | Purpose                                                                |
| --------------------------------------------- | ---------------------------------------------------------------------- |
| [`config.yaml`](config.yaml)                  | Plano configuration (top-level routing_preferences + aliases)          |
| [`run_demo.sh`](run_demo.sh)                  | Bring the demo up/down (`./run_demo.sh [down]`)                        |
| [`test.sh`](test.sh)                          | Per-prompt routing decision + chat completion across all three routes  |
| [`test.rest`](test.rest)                      | REST Client snippets for VS Code / IntelliJ                            |

## Stopping

```bash
./run_demo.sh down   # or: planoai down
```
