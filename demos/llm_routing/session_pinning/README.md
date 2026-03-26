# Session Pinning Demo

> Consistent model selection for agentic loops using `X-Session-Id`.

## Why Session Pinning?

When an agent runs in a loop — research → analyse → implement → evaluate → summarise — each step hits Plano's router independently. Because prompts vary in intent, the router may select **different models** for each step, fragmenting context mid-session.

**Session pinning** solves this: send an `X-Session-Id` header and the first request runs routing as usual, caching the decision. Every subsequent request with the same session ID returns the **same model**, without re-running the router.

```
Without pinning                          With pinning (X-Session-Id)
─────────────────                        ──────────────────────────
Step 1 → claude-sonnet  (code_gen)       Step 1 → claude-sonnet  ← routed
Step 2 → gpt-4o         (reasoning)      Step 2 → claude-sonnet  ← pinned ✓
Step 3 → claude-sonnet  (code_gen)       Step 3 → claude-sonnet  ← pinned ✓
Step 4 → gpt-4o         (reasoning)      Step 4 → claude-sonnet  ← pinned ✓
Step 5 → claude-sonnet  (code_gen)       Step 5 → claude-sonnet  ← pinned ✓
       ↑ model switches every step                ↑ one model, start to finish
```

---

## Quick Start

```bash
# 1. Set API keys
export OPENAI_API_KEY=<your-key>
export ANTHROPIC_API_KEY=<your-key>

# 2. Start Plano
cd demos/llm_routing/session_pinning
planoai up config.yaml

# 3. Run the demo (uv manages dependencies automatically)
./demo.sh          # or: uv run demo.py
```

---

## What the Demo Does

A **Database Research Agent** investigates whether to use PostgreSQL or MongoDB
for an e-commerce platform. It runs 5 steps, each building on prior findings via
accumulated message history. Steps alternate between `code_generation` and
`complex_reasoning` intents so Plano routes to different models without pinning.

| Step | Task | Intent |
|:----:|------|--------|
| 1 | List technical requirements | code_generation → claude-sonnet |
| 2 | Compare PostgreSQL vs MongoDB | complex_reasoning → gpt-4o |
| 3 | Write schema (CREATE TABLE) | code_generation → claude-sonnet |
| 4 | Assess scalability trade-offs | complex_reasoning → gpt-4o |
| 5 | Write final recommendation report | code_generation → claude-sonnet |

The demo runs the loop **twice** against `/v1/chat/completions` using the
[OpenAI SDK](https://github.com/openai/openai-python):

1. **Without pinning** — no `X-Session-Id`; models alternate per step
2. **With pinning** — `X-Session-Id` header included; model is pinned from step 1

Each step makes real LLM calls. Step 5's report explicitly references findings
from earlier steps, demonstrating why coherent context requires a consistent model.

### Expected Output

```
  Run 1: WITHOUT Session Pinning
  ─────────────────────────────────────────────────────────────────────
  step 1  [claude-sonnet-4-20250514]  List requirements
          "Critical requirements: 1. ACID transactions for order integrity…"

  step 2  [gpt-4o                 ]  Compare databases    ← switched
          "PostgreSQL excels at joins and ACID guarantees…"

  step 3  [claude-sonnet-4-20250514]  Write schema        ← switched
          "CREATE TABLE orders (\n  id SERIAL PRIMARY KEY…"

  step 4  [gpt-4o                 ]  Assess scalability   ← switched
          "At high write volume, PostgreSQL row-level locking…"

  step 5  [claude-sonnet-4-20250514]  Write report        ← switched
          "RECOMMENDATION: PostgreSQL is the right choice…"

  ✗  Without pinning: model switched 4 time(s) — gpt-4o, claude-sonnet-4-20250514


  Run 2: WITH Session Pinning  (X-Session-Id: a1b2c3d4…)
  ─────────────────────────────────────────────────────────────────────
  step 1  [claude-sonnet-4-20250514]  List requirements
          "Critical requirements: 1. ACID transactions for order integrity…"

  step 2  [claude-sonnet-4-20250514]  Compare databases
          "Building on the requirements I just outlined: PostgreSQL…"

  step 3  [claude-sonnet-4-20250514]  Write schema
          "Following the comparison above, here is the PostgreSQL schema…"

  step 4  [claude-sonnet-4-20250514]  Assess scalability
          "Given the schema I designed, PostgreSQL's row-level locking…"

  step 5  [claude-sonnet-4-20250514]  Write report
          "RECOMMENDATION: Based on my analysis of requirements, comparison…"

  ✓  With pinning: claude-sonnet-4-20250514 held for all 5 steps

  ══ Final Report (pinned session) ═════════════════════════════════════
  RECOMMENDATION: Based on my analysis of requirements, the head-to-head
  comparison, the schema I designed, and the scalability trade-offs…
  ══════════════════════════════════════════════════════════════════════
```

### How It Works

Session pinning is implemented in brightstaff. When `X-Session-Id` is present:

1. **First request** — routing runs normally, result is cached keyed by session ID
2. **Subsequent requests** — cache hit skips routing and returns the cached model instantly

The `X-Session-Id` header is forwarded transparently; no changes to your OpenAI
SDK calls beyond adding the header.

```python
from openai import OpenAI

client = OpenAI(base_url="http://localhost:12000/v1", api_key="EMPTY")

session_id = str(uuid.uuid4())

response = client.chat.completions.create(
    model="gpt-4o-mini",
    messages=[{"role": "user", "content": prompt}],
    extra_headers={"X-Session-Id": session_id},  # pin the session
)
```

---

## Configuration

Session pinning is configurable in `config.yaml`:

```yaml
routing:
  session_ttl_seconds: 600      # How long a pinned session lasts (default: 10 min)
  session_max_entries: 10000    # Max cached sessions before LRU eviction
```

Without the `X-Session-Id` header, routing runs fresh every time — no breaking
change to existing clients.

---

## See Also

- [Model Routing Service Demo](../model_routing_service/) — curl-based examples of the routing endpoint
