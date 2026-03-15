# Plano: Intelligent LLM Routing as Infrastructure

---

## Plano

An AI-native proxy and data plane for agentic apps — with built-in orchestration, safety, observability, and smart LLM routing so you stay focused on your agent's core logic.

- **One endpoint, many models** — apps call Plano using standard OpenAI/Anthropic APIs; Plano handles provider selection, keys, and failover
- **Intelligent routing** — a lightweight 1.5B router model classifies user intent and picks the best model per request
- **Platform governance** — centralize API keys, rate limits, guardrails, and observability without touching app code
- **Runs anywhere** — single binary, no dependencies; self-host the router for full data privacy

```
┌───────────┐      ┌─────────────────────────────────┐      ┌──────────────┐
│  Client   │ ──── │  Plano                          │ ──── │  OpenAI      │
│  (any     │      │                                 │      │  Anthropic   │
│  language)│      │  Arch-Router (1.5B model)       │      │  Any Provider│
└───────────┘      │  analyzes intent → picks model  │      └──────────────┘
                   └─────────────────────────────────┘
```

---

## Live Demo: Routing Decision Service

The `/routing/v1/*` endpoints return **routing decisions without calling the LLM** — perfect for inspecting, testing, and validating routing behavior.

---

### Demo 1 — Code Generation Request

```bash
curl -s http://localhost:12000/routing/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "messages": [
      {"role": "user", "content": "Write a Python function that implements binary search"}
    ]
  }'
```

**Response:**
```json
{
  "model": "anthropic/claude-sonnet-4-20250514",
  "route": "code_generation"
}
```

Plano recognized the coding intent and routed to Claude.

---

### Demo 2 — Complex Reasoning Request

```bash
curl -s http://localhost:12000/routing/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "messages": [
      {"role": "user", "content": "Explain the trade-offs between microservices and monolithic architectures"}
    ]
  }'
```

**Response:**
```json
{
  "model": "openai/gpt-4o",
  "route": "complex_reasoning"
}
```

Same endpoint — Plano routed to GPT-4o for reasoning.

---

### Demo 3 — Simple Question (No Match)

```bash
curl -s http://localhost:12000/routing/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "messages": [
      {"role": "user", "content": "What is the capital of France?"}
    ]
  }'
```

**Response:**
```json
{
  "model": "none",
  "route": "null"
}
```

No preference matched — falls back to the default (cheapest) model.

---

### Demo 4 — Anthropic Messages Format

```bash
curl -s http://localhost:12000/routing/v1/messages \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "max_tokens": 1024,
    "messages": [
      {"role": "user", "content": "Create a REST API endpoint in Rust using actix-web that handles user registration"}
    ]
  }'
```

**Response:**
```json
{
  "model": "anthropic/claude-sonnet-4-20250514",
  "route": "code_generation"
}
```

Same routing, Anthropic request format.

---

### Demo 5 — OpenAI Responses API Format

```bash
curl -s http://localhost:12000/routing/v1/responses \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "input": "Build a React component that renders a sortable data table"
  }'
```

**Response:**
```json
{
  "model": "anthropic/claude-sonnet-4-20250514",
  "route": "code_generation"
}
```

Same routing engine, works with the OpenAI Responses API format too.

---

## How Did That Work?

10 lines of YAML. No code.

```yaml
model_providers:

  - model: openai/gpt-4o-mini
    default: true                    # fallback for unmatched requests

  - model: openai/gpt-4o
    routing_preferences:
      - name: complex_reasoning
        description: complex reasoning tasks, multi-step analysis

  - model: anthropic/claude-sonnet-4-20250514
    routing_preferences:
      - name: code_generation
        description: generating new code, writing functions
```

That's the entire routing configuration.

---

## Under the Hood: How Routing Preferences Work

### Writing Good Preferences

Each `routing_preference` has two fields:

| Field | Purpose | Example |
|---|---|---|
| `name` | Route identifier (returned in responses) | `code_generation` |
| `description` | Natural language — tells the router **when** to pick this model | `generating new code, writing functions, or creating boilerplate` |

The `description` is the key lever. Write it like you're explaining to a colleague when to use this model:

```yaml
# Good — specific, descriptive
routing_preferences:
  - name: code_generation
    description: generating new code snippets, writing functions, creating boilerplate, or refactoring existing code

# Too vague — overlaps with everything
routing_preferences:
  - name: code
    description: anything related to code
```

Tips:
- **Be specific** — "multi-step mathematical proofs and formal logic" beats "hard questions"
- **Describe the task, not the model** — focus on what the user is asking for
- **Avoid overlap** — if two preferences match the same request, the router has to guess
- **One model can have multiple preferences** — good at both code and math? List both

---

### How Arch-Router Uses Them

When a request arrives, Plano constructs a prompt for the 1.5B Arch-Router model:

```xml
You are a helpful assistant designed to find the best suited route.

<routes>
[
  {"name": "complex_reasoning", "description": "complex reasoning tasks, multi-step analysis"},
  {"name": "code_generation", "description": "generating new code, writing functions"}
]
</routes>

<conversation>
[{"role": "user", "content": "Write a Python function that implements binary search"}]
</conversation>

Your task is to decide which route best suits the user intent...
```

The router classifies the intent and responds:
```json
{"route": "code_generation"}
```

Plano maps `code_generation` back to the model that owns it → `anthropic/claude-sonnet-4-20250514`.

---

### The Full Flow

```
1. Request arrives          → "Write binary search in Python"
2. Preferences serialized   → [{"name":"code_generation", ...}, {"name":"complex_reasoning", ...}]
3. Arch-Router classifies   → {"route": "code_generation"}
4. Route → Model lookup     → code_generation → anthropic/claude-sonnet-4-20250514
5. Request forwarded        → Claude generates the response
```

No match? Arch-Router returns `{"route": "other"}` → Plano falls back to the default model.

---

### What Powers the Routing

**Arch-Router** — a purpose-built 1.5B parameter model for intent classification.

- Runs locally (Ollama) or hosted — no data leaves your network
- Sub-100ms routing decisions
- Handles multi-turn conversations (automatically truncates to fit context)
- Based on preference-aligned routing research

---

## Multi-Format Support

Same routing engine, any API format:

| Endpoint | Format |
|---|---|
| `/routing/v1/chat/completions` | OpenAI Chat Completions |
| `/routing/v1/messages` | Anthropic Messages |
| `/routing/v1/responses` | OpenAI Responses API |

---

## Inline Routing Policy

Clients can override routing at request time — no config change needed:

```json
{
  "model": "gpt-4o-mini",
  "messages": [{"role": "user", "content": "Write quicksort in Go"}],
  "routing_policy": [
    {
      "model": "openai/gpt-4o",
      "routing_preferences": [
        {"name": "coding", "description": "code generation and debugging"}
      ]
    },
    {
      "model": "openai/gpt-4o-mini",
      "routing_preferences": [
        {"name": "general", "description": "simple questions and conversation"}
      ]
    }
  ]
}
```

Platform sets defaults. Teams override when needed.

---

## Beyond Routing

Plano is a full AI data plane:

- **Guardrails** — prompt/response filtering, PII detection
- **Observability** — OpenTelemetry tracing, per-request metrics
- **Rate Limiting** — token-aware rate limiting per model
- **Multi-Provider** — OpenAI, Anthropic, Azure, Gemini, Groq, DeepSeek, Ollama, and more
- **Model Aliases** — `arch.fast.v1` → `gpt-4o-mini` (swap providers without client changes)

---

## Key Takeaways

1. **No SDK required** — standard API, any language, any framework
2. **Semantic routing** — plain English preferences, not hand-coded rules
3. **Self-hosted router** — 1.5B model runs locally, no data leaves the network
4. **Inspect before you route** — decision-only endpoints for testing and CI/CD
5. **Platform governance** — centralized keys, aliases, and routing policies

---

## Try It

```bash
pip install planoai
export OPENAI_API_KEY=...
export ANTHROPIC_API_KEY=...
plano up -f config.yaml
bash demo.sh
```

**GitHub:** github.com/katanemo/plano
