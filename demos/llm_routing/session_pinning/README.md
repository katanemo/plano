# Session Pinning Demo

> Consistent model selection for agentic loops using `X-Session-Id`.

## Why Session Pinning?

When an agent runs in a loop — research → plan → implement → review → refine — each iteration hits Plano's router independently. Since the prompts vary in intent, the router may select **different models** for each step, breaking consistency mid-workflow.

**Session pinning** solves this: send an `X-Session-Id` header and the first request runs routing as usual, caching the decision. Every subsequent request with the same session ID returns the **same model** instantly (`"pinned": true`), without re-running the router.

```
Without pinning                          With pinning (X-Session-Id)
─────────────────                        ───────────────────────────
Step 1 → Claude (code_generation)        Step 1 → Claude (code_generation) ← routed
Step 2 → GPT-4o (complex_reasoning)      Step 2 → Claude (pinned ✓)
Step 3 → Claude (code_generation)        Step 3 → Claude (pinned ✓)
Step 4 → GPT-4o (complex_reasoning)      Step 4 → Claude (pinned ✓)
Step 5 → Claude (code_generation)        Step 5 → Claude (pinned ✓)
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

# 3. Run the demo
./demo.sh          # or: python3 demo.py
```

---

## What the Demo Does

The script simulates an agent building a task management app in **5 iterative steps**, deliberately mixing intents:

| Step | Prompt | Intent |
|:----:|--------|--------|
| 1 | Design a REST API schema for a task management app… | code generation |
| 2 | Analyze SQL vs NoSQL trade-offs for this system… | complex reasoning |
| 3 | Write the SQLAlchemy database models… | code generation |
| 4 | Review the API design for security vulnerabilities… | complex reasoning |
| 5 | Implement JWT authentication middleware… | code generation |

It runs this loop **twice** against the `/routing/v1/chat/completions` endpoint (routing decisions only — no actual LLM calls):

1. **Without pinning** — no `X-Session-Id` header; models switch between steps
2. **With pinning** — `X-Session-Id` header included; the model selected in step 1 is reused for all 5 steps

### Expected Output

```
══════════════════════════════════════════════════════════════════
  Run 1: WITHOUT Session Pinning
──────────────────────────────────────────────────────────────────
  Step 1: Design a REST API schema…        → anthropic/claude-sonnet-4-20250514
  Step 2: Analyze SQL vs NoSQL…            → openai/gpt-4o
  Step 3: Write SQLAlchemy models…         → anthropic/claude-sonnet-4-20250514
  Step 4: Review API for security…         → openai/gpt-4o
  Step 5: Implement JWT auth…              → anthropic/claude-sonnet-4-20250514

  ✗ Models varied: anthropic/claude-sonnet-4-20250514, openai/gpt-4o

══════════════════════════════════════════════════════════════════
  Run 2: WITH Session Pinning (X-Session-Id: a1b2c3d4-…)
──────────────────────────────────────────────────────────────────
  Step 1: Design a REST API schema…        → anthropic/claude-sonnet-4-20250514  (pinned=false)
  Step 2: Analyze SQL vs NoSQL…            → anthropic/claude-sonnet-4-20250514  (pinned=true)
  Step 3: Write SQLAlchemy models…         → anthropic/claude-sonnet-4-20250514  (pinned=true)
  Step 4: Review API for security…         → anthropic/claude-sonnet-4-20250514  (pinned=true)
  Step 5: Implement JWT auth…              → anthropic/claude-sonnet-4-20250514  (pinned=true)

  ✓ All 5 steps routed to anthropic/claude-sonnet-4-20250514
```

---

## Configuration

Session pinning is configurable in `config.yaml`:

```yaml
routing:
  session_ttl_seconds: 600      # How long a pinned session lasts (default: 10 min)
  session_max_entries: 10000    # Max cached sessions before LRU eviction
```

Without the `X-Session-Id` header, routing runs fresh every time — no breaking change to existing clients.

---

## See Also

- [Model Routing Service Demo](../model_routing_service/) — curl-based examples of the routing endpoint and session pinning
