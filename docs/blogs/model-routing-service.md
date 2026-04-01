# Intent-Aware LLM Routing at Infrastructure Speed: How We Built a Purpose-Built 1.5B Router

*Adil Hafeez | April 2026*

---

Every team running multi-model LLM infrastructure eventually hits the same problem: you have five providers, each with different cost and latency profiles, and the right model for a coding question is not the right model for a summarization task. How do you route each request to the best model — without adding seconds of latency or dollars of cost to every call?

We built [Plano](https://github.com/katanemo/plano), an open-source AI-native proxy built on Envoy, to solve this at the infrastructure layer. This post is a deep dive into one specific piece: the **Model Routing Service** — how we use a purpose-built 1.5B parameter model to classify user intent in ~50ms and rank candidate models using live cost and latency data.

## The Routing Problem

When your application talks to multiple LLM providers, you need a routing decision on every request. Teams typically reach for one of three approaches, and each breaks down in a predictable way.

**Keyword and regex matching** is the first instinct. Match "write a function" to the code model, "explain" to the chat model. It's fast — effectively zero latency — but brittle. "Can you code this up?" doesn't match "write a function," and maintenance cost scales linearly with your vocabulary. Every new phrasing requires a new rule.

**Using a frontier model as a classifier** is the next step. Send the user's message to GPT-4 or Claude with a system prompt like "classify this as code_generation or general_question." It works well — frontier models are excellent at intent classification. But you're spending $0.01–0.03 and 500ms–2s per classification call. You're paying a frontier model just to decide which frontier model to call.

**Static rules and load balancing** ignore semantics entirely. Round-robin across a model pool, or route by endpoint path. A complex reasoning question and a simple chat message hit the same model. You're either overpaying (sending everything to the expensive model) or underserving (sending everything to the cheap one).

The gap is clear: we needed something that understands intent like a frontier model but runs at infrastructure speed and infrastructure cost.

## Arch-Router: A Purpose-Built 1.5B Classification Model

Rather than repurposing a general-purpose LLM, we trained a dedicated model for one job: given a conversation and a set of route descriptions, return the name of the best-matching route.

[Arch-Router](https://huggingface.co/katanemo/Arch-Router-1.5B) is a 1.5B parameter model fine-tuned specifically for routing classification. It's not a chat model — it doesn't generate prose, explain its reasoning, or handle follow-up questions. It reads a conversation, compares it against route descriptions, and emits a JSON object: `{"route": "code_generation"}` or `{"route": "other"}` if nothing matches.

**Why 1.5B parameters?** We evaluated models across three orders of magnitude. At 125M parameters, accuracy drops sharply on ambiguous queries — "help me with this code" could be generation or debugging, and smaller models can't reliably distinguish based on conversational context. At 7B+ parameters, accuracy improves marginally (<2% on our benchmark) but latency doubles and GPU memory requirements triple. 1.5B is the inflection point: accurate enough for production routing, small enough to run on a single GPU with 30% memory utilization.

For deployment, we quantize to **Q4_K_M GGUF format**, which keeps GPU memory at ~2GB and enables serving via [vLLM](https://github.com/vllm-project/vllm) with prefix caching enabled. The quantized model maintains classification accuracy within 1% of the full-precision version on our routing benchmark.

### How the Prompt Works

The system prompt uses XML-tagged route descriptions — a deliberate choice over JSON because small models handle XML boundary tokens more reliably:

```
You are a helpful assistant designed to find the best suited route.
You are provided with route description within <routes></routes> XML tags:
<routes>
{routes}
</routes>

<conversation>
{conversation}
</conversation>

Your task is to decide which route is best suit with user intent on the
conversation in <conversation></conversation> XML tags. Follow the instruction:
1. If the latest intent from user is irrelevant or user intent is full filled,
   response with other route {"route": "other"}.
2. You must analyze the route descriptions and find the best match route for
   user latest intent.
3. You only response the name of the route that best matches the user's request,
   use the exact name in the <routes></routes>.

Based on your analysis, provide your response in the following JSON formats if
you decide to match any route:
{"route": "route_name"}
```

The `{routes}` placeholder is populated from the YAML configuration — each route has a name and a natural-language description. The `{conversation}` placeholder gets the user's messages, with system messages and tool calls filtered out to focus on user intent. We cap input at 2048 tokens; routing decisions should be based on recent context, not entire conversation histories.

This is binary classification per route, not N-way. The model evaluates each route description against the conversation and picks the best match. If nothing fits, it returns `"other"` and the request falls through to the default model.

We also trained a variant called **Plano-Orchestrator** for multi-agent scenarios, where the model returns an array of matching routes: `{"route": ["research_agent", "code_agent"]}`. Same architecture, different training objective.

## The Ranking Engine: Live Cost and Latency Data

Knowing the right *route* is only half the problem. Within a route, you might have three candidate models — and the best one depends on whether you're optimizing for cost or latency right now. Static ordering doesn't cut it because model pricing changes, latency drifts with load, and rate limits shift availability.

Plano's `ModelMetricsService` continuously fetches cost and latency data from external sources, then ranks candidate models at request time.

The core ranking function is straightforward:

```rust
pub async fn rank_models(&self, models: &[String], policy: &SelectionPolicy) -> Vec<String> {
    match policy.prefer {
        SelectionPreference::Cheapest => {
            let data = self.cost.read().await;
            rank_by_ascending_metric(models, &data)
        }
        SelectionPreference::Fastest => {
            let data = self.latency.read().await;
            rank_by_ascending_metric(models, &data)
        }
        SelectionPreference::Random => shuffle(models),
        SelectionPreference::None => models.to_vec(),
    }
}
```

Models with no metric data get appended last — they're still available as fallback but won't be preferred. The system logs a warning both at startup and per-request when a model has no data, so you can catch misconfigurations early.

### Metrics Sources

**Cost data** is fetched from DigitalOcean's public Gen-AI pricing API, which requires no authentication and returns input/output pricing per million tokens for all models in the catalog. We compute a single cost scalar as `input_price_per_million + output_price_per_million` — only relative ordering matters, not absolute numbers.

**Latency data** comes from Prometheus. You provide a PromQL query that returns an instant vector with a `model_name` label — typically a P95 histogram quantile over your actual traffic. The system re-fetches on a configurable interval (default: 60s for latency, 3600s for cost).

A `model_aliases` map bridges naming differences. DigitalOcean's catalog uses `openai-gpt-4o`; your config might use `openai/gpt-4o`. The alias map handles this without changing your routing configuration.

### Fail-Fast Validation

Plano validates metric source configuration at startup and exits with a clear error if the setup is inconsistent:

| Condition | Error |
|---|---|
| `prefer: cheapest` with no cost source | `requires a cost metrics source` |
| `prefer: fastest` with no latency source | `requires a latency metrics source` |

This is a deliberate design choice. Misconfigured routing that silently falls back to default ordering is worse than a startup crash — you'd spend hours debugging why your "cheapest" policy is serving GPT-4o before GPT-4o-mini.

## Architecture: Why Envoy, WASM, and Async Rust

The routing service doesn't exist in isolation. It runs inside Plano's three-layer architecture, and the choice of each layer directly affects routing performance.

```
Client ──► Envoy (llm_gateway.wasm) ──► Brightstaff ──► LLM Providers
                                             │
                                        Arch-Router (1.5B)
                                        Metrics Service
```

### Layer 1: Envoy as Transport Substrate

We don't implement TLS, connection pooling, retries, circuit breaking, or HTTP/2 multiplexing. Envoy does all of this, battle-tested across deployments at Google, Lyft, and thousands of other production environments. Building a custom HTTP server to handle LLM traffic would mean reimplementing solved infrastructure problems — and getting them wrong in subtle ways under load.

Envoy's threading model matters here: one event-loop worker per CPU core, each connection pinned to a single worker. There's no lock contention in the hot path. For streaming LLM responses — which are long-lived, chunked HTTP connections — this model scales naturally. We're building on Envoy because we were early contributors to the project and understand its extension points deeply.

### Layer 2: LLM Gateway (WASM Plugin)

The `llm_gateway.wasm` filter runs inside Envoy's process — not as a sidecar, not as a separate service. It handles format translation between providers (OpenAI, Anthropic, Gemini, Mistral, Groq, DeepSeek, xAI, Bedrock) at wire speed with zero network hop.

The WASM sandbox imposes a strict constraint: **no std networking, no tokio, no async runtime**. Everything is `dispatch_http_call()` with a callback. All dependencies must be `no_std`-compatible. This is painful to develop against, but it produces a cleaner separation between I/O and logic — and the resulting binary is tiny (single-digit MBs) with a predictable memory footprint.

The format translation layer is powered by `hermesllm`, our Rust crate for LLM API abstraction. Adding a new provider means implementing `ProviderRequest` and `ProviderResponse` traits — the router and gateway don't need to change.

### Layer 3: Brightstaff (Native Async Rust)

The routing logic — `RouterService`, `ModelMetricsService`, OTEL tracing — lives in Brightstaff, a native Rust binary running on the Tokio async runtime alongside Envoy. One lightweight Tokio task per request, not one OS thread. This handles thousands of concurrent routing decisions on modest hardware.

**Why Rust?** In a proxy that handles streaming LLM responses, garbage collector pauses cause visible stutter in token delivery. Go's GC pause (typically 0.1-1ms) is fine for most applications but noticeable in a token stream delivering chunks every 20-50ms. Rust's ownership model eliminates this class of bugs entirely — no GC, no pauses, predictable latency.

## Running the Model Routing Service

Here's the complete setup from our [demo](https://github.com/katanemo/plano/tree/main/demos/llm_routing/model_routing_service). The config defines two routes with different ranking strategies and two metrics sources:

```yaml
version: v0.4.0

listeners:
  - type: model
    name: model_listener
    port: 12000

model_providers:
  - model: openai/gpt-4o-mini
    access_key: $OPENAI_API_KEY
    default: true
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
  - model: anthropic/claude-sonnet-4-20250514
    access_key: $ANTHROPIC_API_KEY

routing_preferences:
  - name: complex_reasoning
    description: complex reasoning tasks, multi-step analysis, or detailed explanations
    models:
      - openai/gpt-4o
      - openai/gpt-4o-mini
    selection_policy:
      prefer: cheapest

  - name: code_generation
    description: generating new code, writing functions, or creating boilerplate
    models:
      - anthropic/claude-sonnet-4-20250514
      - openai/gpt-4o
    selection_policy:
      prefer: fastest

model_metrics_sources:
  - type: cost
    provider: digitalocean
    refresh_interval: 3600
    model_aliases:
      openai-gpt-4o: openai/gpt-4o
      openai-gpt-4o-mini: openai/gpt-4o-mini
      anthropic-claude-sonnet-4: anthropic/claude-sonnet-4-20250514

  - type: latency
    provider: prometheus
    url: http://localhost:9090
    query: model_latency_p95_seconds
    refresh_interval: 60
```

Start the metrics infrastructure and Plano:

```bash
# Start Prometheus + mock metrics server
docker compose up -d

# Start Plano
planoai up config.yaml
```

### Code Generation: Ranked by Latency

A coding request hits the `code_generation` route. With `prefer: fastest`, the metrics service checks P95 latencies from Prometheus — Claude-Sonnet at 0.85s beats GPT-4o at 1.20s:

```bash
curl -s http://localhost:12000/routing/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "messages": [
      {"role": "user", "content": "Write a Python function that implements binary search on a sorted array"}
    ]
  }'
```

```json
{
    "models": ["anthropic/claude-sonnet-4-20250514", "openai/gpt-4o"],
    "route": "code_generation",
    "trace_id": "c16d1096c1af4a17abb48fb182918a88"
}
```

### Complex Reasoning: Ranked by Cost

A reasoning request hits `complex_reasoning` with `prefer: cheapest`. DigitalOcean pricing puts GPT-4o-mini ($0.75/M tokens) well ahead of GPT-4o ($25/M):

```bash
curl -s http://localhost:12000/routing/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "messages": [
      {"role": "user", "content": "Explain the trade-offs between microservices and monolithic architectures, considering scalability, team structure, and operational complexity"}
    ]
  }'
```

```json
{
    "models": ["openai/gpt-4o-mini", "openai/gpt-4o"],
    "route": "complex_reasoning",
    "trace_id": "..."
}
```

### Per-Request Overrides

Config-level preferences set the default, but individual requests can override them with an inline `routing_preferences` field. This is stripped from the request before forwarding upstream — downstream providers never see it:

```bash
curl -s http://localhost:12000/routing/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "messages": [
      {"role": "user", "content": "Summarize the key differences between TCP and UDP"}
    ],
    "routing_preferences": [
      {
        "name": "general",
        "description": "general questions, explanations, and summaries",
        "models": ["openai/gpt-4o", "openai/gpt-4o-mini"],
        "selection_policy": {"prefer": "cheapest"}
      }
    ]
  }'
```

The response includes a ranked `models` array. The client pattern is simple — use `models[0]`, fall back to `models[1]` on 429 or 5xx:

```python
response = plano.routing_decision(request)

for model in response["models"]:
    try:
        result = call_llm(model, messages)
        break  # success
    except (RateLimitError, ServerError):
        continue  # try next
```

The `/routing/v1/*` endpoints return routing decisions without forwarding to the LLM — useful for testing routing behavior, integrating with existing orchestration code, or implementing custom fallback logic.

## Production Deployment: Self-Hosted on Kubernetes

For teams that need routing decisions to stay within their cluster — regulatory requirements, data sovereignty, or simply avoiding external API dependencies — Arch-Router can be self-hosted using vLLM.

The deployment uses an init container to download quantized weights from HuggingFace, then serves the model via vLLM's OpenAI-compatible endpoint:

```yaml
initContainers:
  - name: download-model
    image: python:3.11-slim
    command:
      - sh
      - -c
      - |
        pip install huggingface_hub[cli] && \
        python -c "from huggingface_hub import snapshot_download; \
          snapshot_download('katanemo/Arch-Router-1.5B.gguf', \
          local_dir='/models/Arch-Router-1.5B.gguf')"
containers:
  - name: vllm
    image: vllm/vllm-openai:latest
    command:
      - vllm
      - serve
      - /models/Arch-Router-1.5B.gguf/Arch-Router-1.5B-Q4_K_M.gguf
      - "--served-model-name"
      - "Arch-Router"
      - "--gpu-memory-utilization"
      - "0.3"
      - "--enable-prefix-caching"
    resources:
      requests:
        nvidia.com/gpu: "1"
        memory: "4Gi"
```

GPU requirements are modest: a single L4 or L40S with 30% memory utilization. Prefix caching is enabled because route descriptions are constant across requests — the system prompt prefix is computed once and reused, cutting inference latency further.

The Plano config points to the in-cluster service:

```yaml
overrides:
  llm_routing_model: plano/Arch-Router

model_providers:
  - model: plano/Arch-Router
    base_url: http://arch-router:10000
```

For teams that don't want to manage GPU infrastructure, DigitalOcean's [GPU Droplets](https://www.digitalocean.com/products/gpu-droplets) provide single-click deployment of vLLM with NVIDIA L40S GPUs — spin up the Arch-Router as a managed inference endpoint without provisioning bare metal.

## What We Learned

Building and operating this in production surfaced a few non-obvious lessons:

**Purpose-built models beat general-purpose models for classification — if you have the training data.** A 1.5B model fine-tuned on routing decisions outperforms GPT-4 few-shot prompting on our benchmark, at 1/30th the cost and 1/20th the latency. The key is that routing is a narrow, well-defined task. You don't need a model that can write poetry to decide whether a query is about code or about cooking.

**Startup validation prevents an entire class of silent bugs.** Early versions logged warnings for misconfigured metrics sources. Users didn't notice the warnings, deployed to production, and spent hours debugging why "cheapest" routing wasn't actually routing by cost. Crashing at startup is better UX than silent degradation.

**The WASM no_std constraint produces cleaner code.** Not being able to reach for tokio or std::net forces a callback-driven architecture where every I/O operation is explicit. The resulting code is harder to write but trivially auditable — you can trace every external call from the code alone, without understanding a runtime.

**Live metrics ranking is more useful than static config because model performance drifts.** Provider latency varies by 2-3x throughout the day based on traffic patterns. A model that's "fastest" at 2am is often the slowest at 2pm. Refreshing Prometheus data every 60 seconds catches these shifts; static config doesn't.

---

The Model Routing Service is open source as part of [Plano](https://github.com/katanemo/plano). The complete demo, including Docker Compose, Kubernetes manifests, and example scripts, is at [`demos/llm_routing/model_routing_service/`](https://github.com/katanemo/plano/tree/main/demos/llm_routing/model_routing_service).
