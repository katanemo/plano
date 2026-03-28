#!/bin/bash
set -e

PLANO_URL="${PLANO_URL:-http://localhost:12000}"

echo "=== Model Routing Service Demo ==="
echo ""
echo "This demo shows how to use the /routing/v1/* endpoints to get"
echo "routing decisions without actually proxying the request to an LLM."
echo ""
echo "The response includes a ranked 'models' list — use models[0] as the"
echo "primary and fall back to models[1] on 429/5xx errors."
echo ""

# --- Example 1: Code generation (ranked by fastest) ---
echo "--- 1. Code generation query (prefer: fastest) ---"
echo ""
curl -s "$PLANO_URL/routing/v1/chat/completions" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "messages": [
      {"role": "user", "content": "Write a Python function that implements binary search on a sorted array"}
    ]
  }' | python3 -m json.tool
echo ""

# --- Example 2: Complex reasoning (ranked by cheapest) ---
echo "--- 2. Complex reasoning query (prefer: cheapest) ---"
echo ""
curl -s "$PLANO_URL/routing/v1/chat/completions" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "messages": [
      {"role": "user", "content": "Explain the trade-offs between microservices and monolithic architectures, considering scalability, team structure, and operational complexity"}
    ]
  }' | python3 -m json.tool
echo ""

# --- Example 3: Simple query (no routing match) ---
echo "--- 3. Simple query - no routing match (falls back to request model) ---"
echo ""
curl -s "$PLANO_URL/routing/v1/chat/completions" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "messages": [
      {"role": "user", "content": "What is the capital of France?"}
    ]
  }' | python3 -m json.tool
echo ""

# --- Example 4: Anthropic Messages format ---
echo "--- 4. Code generation query (Anthropic format) ---"
echo ""
curl -s "$PLANO_URL/routing/v1/messages" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "max_tokens": 1024,
    "messages": [
      {"role": "user", "content": "Create a REST API endpoint in Rust using actix-web that handles user registration"}
    ]
  }' | python3 -m json.tool
echo ""

# --- Example 5: Inline routing_preferences with prefer:cheapest ---
echo "--- 5. Inline routing_preferences (prefer: cheapest) ---"
echo "    models[] will be sorted by ascending cost from DigitalOcean pricing"
echo ""
curl -s "$PLANO_URL/routing/v1/chat/completions" \
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
  }' | python3 -m json.tool
echo ""

# --- Example 6: Inline routing_preferences with prefer:fastest ---
echo "--- 6. Inline routing_preferences (prefer: fastest) ---"
echo "    models[] will be sorted by ascending P95 latency from Prometheus"
echo ""
curl -s "$PLANO_URL/routing/v1/chat/completions" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "messages": [
      {"role": "user", "content": "Write a quicksort implementation in Go"}
    ],
    "routing_preferences": [
      {
        "name": "coding",
        "description": "code generation, writing functions, debugging",
        "models": ["anthropic/claude-sonnet-4-20250514", "openai/gpt-4o", "openai/gpt-4o-mini"],
        "selection_policy": {"prefer": "fastest"}
      }
    ]
  }' | python3 -m json.tool
echo ""

echo "=== Demo Complete ==="
