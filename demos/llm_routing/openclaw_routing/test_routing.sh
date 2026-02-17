#!/usr/bin/env bash
set -euo pipefail

PLANO_URL="http://localhost:12000/v1/chat/completions"

echo "=== Testing Plano Routing Decisions ==="
echo ""

# Scenario 1: General conversation -> should route to Kimi K2.5
echo "--- Scenario 1: General Conversation (expect: Kimi K2.5) ---"
curl -s "$PLANO_URL" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "kimi-k2.5",
    "messages": [{"role": "user", "content": "Hey! What is the weather like today? Can you tell me a fun fact?"}]
  }' | jq '{model: .model, content: .choices[0].message.content[:100]}'
echo ""

# Scenario 2: Agentic task -> should route to Kimi K2.5
echo "--- Scenario 2: Agentic Task (expect: Kimi K2.5) ---"
curl -s "$PLANO_URL" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "kimi-k2.5",
    "messages": [{"role": "user", "content": "Schedule a reminder for tomorrow at 9am to review the pull request, then send a message to the team Slack channel about the deployment."}]
  }' | jq '{model: .model, content: .choices[0].message.content[:100]}'
echo ""

# Scenario 3: Code generation -> should route to Claude
echo "--- Scenario 3: Code Generation (expect: Claude) ---"
curl -s "$PLANO_URL" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "kimi-k2.5",
    "messages": [{"role": "user", "content": "Write a Python function that implements a rate limiter using the token bucket algorithm with async support."}]
  }' | jq '{model: .model, content: .choices[0].message.content[:100]}'
echo ""

# Scenario 4: Testing/evaluation -> should route to Claude
echo "--- Scenario 4: Testing & Evaluation (expect: Claude) ---"
curl -s "$PLANO_URL" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "kimi-k2.5",
    "messages": [{"role": "user", "content": "Write unit tests for this authentication middleware. Test edge cases: expired tokens, malformed headers, missing credentials, and concurrent requests."}]
  }' | jq '{model: .model, content: .choices[0].message.content[:100]}'
echo ""

# Scenario 5: Complex reasoning -> should route to Claude
echo "--- Scenario 5: Complex Reasoning (expect: Claude) ---"
curl -s "$PLANO_URL" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "kimi-k2.5",
    "messages": [{"role": "user", "content": "Analyze the trade-offs between using WebSockets vs SSE vs long-polling for real-time notifications in a distributed messaging system with 10K concurrent users."}]
  }' | jq '{model: .model, content: .choices[0].message.content[:100]}'
echo ""

echo "=== Check Plano logs for MODEL_RESOLUTION details ==="
echo "Run: docker logs plano 2>&1 | grep MODEL_RESOLUTION"
