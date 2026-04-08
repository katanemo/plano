# Model Affinity Demo

> Consistent model selection for agentic loops using `X-Model-Affinity`.

## Why Model Affinity?

When an agent runs in a loop — calling tools, reasoning about results, calling more tools — each LLM request hits Plano's router independently. Because prompts vary in intent (tool selection looks like code generation, reasoning about results looks like complex analysis), the router may select **different models** for each turn, fragmenting context mid-session.

**Model affinity** solves this: send an `X-Model-Affinity` header and the first request runs routing as usual, caching the decision. Every subsequent request with the same affinity ID returns the **same model**, without re-running the router.

```
Without affinity                         With affinity (X-Model-Affinity)
────────────────                         ───────────────────────────────
Turn 1 → claude-sonnet  (tool calls)     Turn 1 → claude-sonnet  ← routed
Turn 2 → gpt-4o         (reasoning)      Turn 2 → claude-sonnet  ← pinned ✓
Turn 3 → claude-sonnet  (tool calls)     Turn 3 → claude-sonnet  ← pinned ✓
Turn 4 → gpt-4o         (reasoning)      Turn 4 → claude-sonnet  ← pinned ✓
Turn 5 → claude-sonnet  (final answer)   Turn 5 → claude-sonnet  ← pinned ✓
       ↑ model switches every turn                ↑ one model, start to finish
```

---

## Quick Start

```bash
# 1. Set API keys
export OPENAI_API_KEY=<your-key>
export ANTHROPIC_API_KEY=<your-key>

# 2. Start Plano
cd demos/llm_routing/model_affinity
planoai up config.yaml

# 3. Run the demo (uv manages dependencies automatically)
./demo.sh          # or: uv run demo.py
```

---

## What the Demo Does

A **database selection agent** investigates whether to use PostgreSQL or MongoDB
for an e-commerce platform. It runs a real tool-calling loop: the LLM decides
which tools to call, receives simulated results, and continues until it has
enough data to recommend a database.

Available tools:
- `get_db_benchmarks` — fetch performance data for a workload type
- `get_case_studies` — retrieve real-world e-commerce case studies
- `check_feature_support` — check if a database supports a specific feature

The demo runs the **same agent loop twice**:

1. **Without affinity** — no `X-Model-Affinity`; models may switch between turns
2. **With affinity** — `X-Model-Affinity` header included; model is pinned from turn 1

Each turn is a separate `POST /v1/chat/completions` request to Plano using the
[OpenAI SDK](https://github.com/openai/openai-python). The demo prints the
model used on each turn so you can see the difference.

### Expected Output

```
  Run 1: WITHOUT Model Affinity
  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    turn 1  [claude-sonnet-4-20250514     ]  get_db_benchmarks, get_db_benchmarks
    turn 2  [gpt-4o                       ]  get_case_studies, get_case_studies     ← switched
    turn 3  [claude-sonnet-4-20250514     ]  check_feature_support                 ← switched
    turn 4  [gpt-4o                       ]  final answer                          ← switched

  ✗  Without affinity: model switched 3 time(s)


  Run 2: WITH Model Affinity  (X-Model-Affinity: a1b2c3d4…)
  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    turn 1  [claude-sonnet-4-20250514     ]  get_db_benchmarks, get_db_benchmarks
    turn 2  [claude-sonnet-4-20250514     ]  get_case_studies, get_case_studies
    turn 3  [claude-sonnet-4-20250514     ]  check_feature_support
    turn 4  [claude-sonnet-4-20250514     ]  final answer

  ✓  With affinity: claude-sonnet-4-20250514 for all 4 turns
```

### How It Works

Model affinity is implemented in brightstaff. When `X-Model-Affinity` is present:

1. **First request** — routing runs normally, result is cached keyed by the affinity ID
2. **Subsequent requests** — cache hit skips routing and returns the cached model instantly

The `X-Model-Affinity` header is forwarded transparently; no changes to your OpenAI
SDK calls beyond adding the header.

```python
from openai import OpenAI
import uuid

client = OpenAI(base_url="http://localhost:12000/v1", api_key="EMPTY")

affinity_id = str(uuid.uuid4())

response = client.chat.completions.create(
    model="gpt-4o-mini",
    messages=[{"role": "user", "content": prompt}],
    extra_headers={"X-Model-Affinity": affinity_id},
)
```

---

## Configuration

Model affinity is configurable in `config.yaml`:

```yaml
routing:
  session_ttl_seconds: 600      # How long affinity lasts (default: 10 min)
  session_max_entries: 10000    # Max cached sessions (upper limit: 10000)
```

Without the `X-Model-Affinity` header, routing runs fresh every time — no breaking
change to existing clients.

---

## Advanced: Agent Server Demo

The `agent.py` file is a FastAPI-based agent server that demonstrates a more
complex pattern: an external agent service that forwards `X-Model-Affinity`
on all outbound calls to Plano. Use `start_agents.sh` to run it.

## See Also

- [Model Routing Service Demo](../model_routing_service/) — curl-based examples of the routing endpoint
