# Model Routing Service Demo

Plano is an AI-native proxy and data plane for agentic apps — with built-in orchestration, safety, observability, and intelligent LLM routing.

```
┌───────────┐      ┌─────────────────────────────────┐      ┌──────────────┐
│  Client   │ ───► │  Plano                          │ ───► │  OpenAI      │
│  (any     │      │                                 │      │  Anthropic   │
│  language)│      │  Arch-Router (1.5B model)       │      │  Any Provider│
└───────────┘      │  analyzes intent → picks model  │      └──────────────┘
                   └─────────────────────────────────┘
```

- **One endpoint, many models** — apps call Plano using standard OpenAI/Anthropic APIs; Plano handles provider selection, keys, and failover
- **Intelligent routing** — a lightweight 1.5B router model classifies user intent and picks the best model per request
- **Platform governance** — centralize API keys, rate limits, guardrails, and observability without touching app code
- **Runs anywhere** — single binary; self-host the router for full data privacy

## How Routing Works

The entire routing configuration is plain YAML — no code:

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

When a request arrives, Plano sends the conversation and routing preferences to Arch-Router, which classifies the intent and returns the matching route:

```
1. Request arrives          → "Write binary search in Python"
2. Preferences serialized   → [{"name":"code_generation", ...}, {"name":"complex_reasoning", ...}]
3. Arch-Router classifies   → {"route": "code_generation"}
4. Route → Model lookup     → code_generation → anthropic/claude-sonnet-4-20250514
5. Request forwarded        → Claude generates the response
```

No match? Arch-Router returns `other` → Plano falls back to the default model.

The `/routing/v1/*` endpoints return the routing decision **without** forwarding to the LLM — useful for testing and validating routing behavior before going to production.

## Setup

Make sure you have Plano CLI installed (`pip install planoai` or `uv tool install planoai`).

```bash
export OPENAI_API_KEY=<your-key>
export ANTHROPIC_API_KEY=<your-key>
```

Start Plano:
```bash
cd demos/llm_routing/model_routing_service
planoai up config.yaml
```

## Run the demo

```bash
./demo.sh
```

## Endpoints

All three LLM API formats are supported:

| Endpoint | Format |
|---|---|
| `POST /routing/v1/chat/completions` | OpenAI Chat Completions |
| `POST /routing/v1/messages` | Anthropic Messages |
| `POST /routing/v1/responses` | OpenAI Responses API |

## Example

```bash
curl http://localhost:12000/routing/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "messages": [{"role": "user", "content": "Write a Python function for binary search"}]
  }'
```

Response:
```json
{
    "model": "anthropic/claude-sonnet-4-20250514",
    "route": "code_generation",
    "trace_id": "c16d1096c1af4a17abb48fb182918a88"
}
```

The response tells you which model would handle this request and which route was matched, without actually making the LLM call.

## Session Pinning

Send an `X-Session-Id` header to pin the routing decision for a session. Once a model is selected, all subsequent requests with the same session ID return the same model without re-running routing.

```bash
# First call — runs routing, caches result
curl http://localhost:12000/routing/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "X-Session-Id: my-session-123" \
  -d '{
    "model": "gpt-4o-mini",
    "messages": [{"role": "user", "content": "Write a Python function for binary search"}]
  }'
```

Response (first call):
```json
{
    "model": "anthropic/claude-sonnet-4-20250514",
    "route": "code_generation",
    "trace_id": "c16d1096c1af4a17abb48fb182918a88",
    "session_id": "my-session-123",
    "pinned": false
}
```

```bash
# Second call — same session, returns cached result
curl http://localhost:12000/routing/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "X-Session-Id: my-session-123" \
  -d '{
    "model": "gpt-4o-mini",
    "messages": [{"role": "user", "content": "Now explain merge sort"}]
  }'
```

Response (pinned):
```json
{
    "model": "anthropic/claude-sonnet-4-20250514",
    "route": "code_generation",
    "trace_id": "a1b2c3d4e5f6...",
    "session_id": "my-session-123",
    "pinned": true
}
```

Session TTL and max cache size are configurable in `config.yaml`:
```yaml
routing:
  session_ttl_seconds: 600      # default: 600 (10 minutes)
  session_max_entries: 10000    # default: 10000
```

Without the `X-Session-Id` header, routing runs fresh every time (no breaking change).

## Kubernetes Deployment (Self-hosted Arch-Router on GPU)

To run Arch-Router in-cluster using vLLM instead of the default hosted endpoint:

**0. Check your GPU node labels and taints**

```bash
kubectl get nodes --show-labels | grep -i gpu
kubectl get node <gpu-node-name> -o jsonpath='{.spec.taints}'
```

GPU nodes commonly have a `nvidia.com/gpu:NoSchedule` taint — `vllm-deployment.yaml` includes a matching toleration. If you have multiple GPU node pools and need to pin to a specific one, uncomment and set the `nodeSelector` in `vllm-deployment.yaml` using the label for your cloud provider.

**1. Deploy Arch-Router and Plano:**

```bash

# arch-router deployment
kubectl apply -f vllm-deployment.yaml

# plano deployment
kubectl create secret generic plano-secrets \
  --from-literal=OPENAI_API_KEY=$OPENAI_API_KEY \
  --from-literal=ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY

kubectl create configmap plano-config \
  --from-file=plano_config.yaml=config_k8s.yaml \
  --dry-run=client -o yaml | kubectl apply -f -

kubectl apply -f plano-deployment.yaml
```

**3. Wait for both pods to be ready:**

```bash
# Arch-Router downloads the model (~1 min) then vLLM loads it (~2 min)
kubectl get pods -l app=arch-router -w
kubectl rollout status deployment/plano
```

**4. Test:**

```bash
kubectl port-forward svc/plano 12000:12000
./demo.sh
```

To confirm requests are hitting your in-cluster Arch-Router (not just health checks):

```bash
kubectl logs -l app=arch-router -f --tail=0
# Look for POST /v1/chat/completions entries
```

**Updating the config:**

```bash
kubectl create configmap plano-config \
  --from-file=plano_config.yaml=config_k8s.yaml \
  --dry-run=client -o yaml | kubectl apply -f -
kubectl rollout restart deployment/plano
```

## Demo Output

```
=== Model Routing Service Demo ===

--- 1. Code generation query (OpenAI format) ---
{
    "model": "anthropic/claude-sonnet-4-20250514",
    "route": "code_generation",
    "trace_id": "c16d1096c1af4a17abb48fb182918a88"
}

--- 2. Complex reasoning query (OpenAI format) ---
{
    "model": "openai/gpt-4o",
    "route": "complex_reasoning",
    "trace_id": "30795e228aff4d7696f082ed01b75ad4"
}

--- 3. Simple query - no routing match (OpenAI format) ---
{
    "model": "none",
    "route": null,
    "trace_id": "ae0b6c3b220d499fb5298ac63f4eac0e"
}

--- 4. Code generation query (Anthropic format) ---
{
    "model": "anthropic/claude-sonnet-4-20250514",
    "route": "code_generation",
    "trace_id": "26be822bbdf14a3ba19fe198e55ea4a9"
}

--- 7. Session pinning - first call (fresh routing decision) ---
{
    "model": "anthropic/claude-sonnet-4-20250514",
    "route": "code_generation",
    "trace_id": "f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6",
    "session_id": "demo-session-001",
    "pinned": false
}

--- 8. Session pinning - second call (same session, pinned) ---
    Notice: same model returned with "pinned": true, routing was skipped
{
    "model": "anthropic/claude-sonnet-4-20250514",
    "route": "code_generation",
    "trace_id": "a9b8c7d6e5f4a3b2c1d0e9f8a7b6c5d4",
    "session_id": "demo-session-001",
    "pinned": true
}

--- 9. Different session gets its own fresh routing ---
{
    "model": "openai/gpt-4o",
    "route": "complex_reasoning",
    "trace_id": "1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d",
    "session_id": "demo-session-002",
    "pinned": false
}

=== Demo Complete ===
```
