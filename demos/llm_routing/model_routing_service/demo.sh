#!/bin/bash
set -e

PLANO_URL="${PLANO_URL:-http://localhost:12000}"

echo "=== Model Routing Service Demo ==="
echo ""
echo "This demo shows how to use the /routing/v1/* endpoints to get"
echo "routing decisions without actually proxying the request to an LLM."
echo ""

# --- Example 1: OpenAI Chat Completions format ---
echo "--- 1. Code generation query (OpenAI format) ---"
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

# --- Example 2: Complex reasoning query ---
echo "--- 2. Complex reasoning query (OpenAI format) ---"
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
echo "--- 3. Simple query - no routing match (OpenAI format) ---"
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

# --- Example 5: Inline routing policy in request body ---
echo "--- 5. Inline routing_policy (no config needed) ---"
echo ""
curl -s "$PLANO_URL/routing/v1/chat/completions" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "messages": [
      {"role": "user", "content": "Write a quicksort implementation in Go"}
    ],
    "routing_policy": [
      {
        "model": "openai/gpt-4o",
        "routing_preferences": [
          {"name": "coding", "description": "code generation, writing functions, debugging"}
        ]
      },
      {
        "model": "openai/gpt-4o-mini",
        "routing_preferences": [
          {"name": "general", "description": "general questions, simple lookups, casual conversation"}
        ]
      }
    ]
  }' | python3 -m json.tool
echo ""

# --- Example 6: Inline routing policy with Anthropic format ---
echo "--- 6. Inline routing_policy (Anthropic format) ---"
echo ""
curl -s "$PLANO_URL/routing/v1/messages" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "max_tokens": 1024,
    "messages": [
      {"role": "user", "content": "What is the weather like today?"}
    ],
    "routing_policy": [
      {
        "model": "openai/gpt-4o",
        "routing_preferences": [
          {"name": "coding", "description": "code generation, writing functions, debugging"}
        ]
      },
      {
        "model": "openai/gpt-4o-mini",
        "routing_preferences": [
          {"name": "general", "description": "general questions, simple lookups, casual conversation"}
        ]
      }
    ]
  }' | python3 -m json.tool
echo ""

echo "=== Demo Complete ==="
