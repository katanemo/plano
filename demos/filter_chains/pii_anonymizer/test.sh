#!/usr/bin/env bash
set -euo pipefail

BASE_URL="http://localhost:12000"
PASS=0
FAIL=0

# ── Wait for Plano to be ready ──────────────────────────────────────────────
echo "Waiting for Plano to be ready..."
for i in $(seq 1 30); do
    if curl -sf "$BASE_URL/v1/models" > /dev/null 2>&1; then
        echo "Plano is ready."
        break
    fi
    if [ "$i" -eq 30 ]; then
        echo "ERROR: Plano did not become ready in time."
        exit 1
    fi
    sleep 2
done

# ── Helper ───────────────────────────────────────────────────────────────────
run_test() {
    local name="$1"
    local path="$2"
    local expected_code="$3"
    local body="$4"

    http_code=$(curl -s -o /tmp/plano_test_body -w "%{http_code}" \
        -X POST "$BASE_URL$path" \
        -H "Content-Type: application/json" \
        -d "$body")

    if [ "$http_code" -eq "$expected_code" ]; then
        echo "  PASS  $name (HTTP $http_code)"
        PASS=$((PASS + 1))
    else
        echo "  FAIL  $name — expected $expected_code, got $http_code"
        echo "        Body: $(cat /tmp/plano_test_body)"
        FAIL=$((FAIL + 1))
    fi
}

# ── /v1/chat/completions ─────────────────────────────────────────────────────
echo ""
echo "=== /v1/chat/completions ==="

run_test "Non-streaming with PII (email + phone)" /v1/chat/completions 200 '{
  "model": "gpt-4o-mini",
  "messages": [{"role": "user", "content": "Contact me at john@example.com or call 555-123-4567"}],
  "stream": false
}'

run_test "Streaming with PII (SSN)" /v1/chat/completions 200 '{
  "model": "gpt-4o-mini",
  "messages": [{"role": "user", "content": "My SSN is 123-45-6789, please help me file taxes"}],
  "stream": true
}'

run_test "No PII (clean message)" /v1/chat/completions 200 '{
  "model": "gpt-4o-mini",
  "messages": [{"role": "user", "content": "What is 2+2?"}],
  "stream": false
}'

run_test "Multiple PII types" /v1/chat/completions 200 '{
  "model": "gpt-4o-mini",
  "messages": [{"role": "user", "content": "Email: test@test.com, SSN: 987-65-4321, Card: 4111 1111 1111 1111"}],
  "stream": false
}'

# ── /v1/responses ────────────────────────────────────────────────────────────
echo ""
echo "=== /v1/responses ==="

run_test "Non-streaming with PII (email)" /v1/responses 200 '{
  "model": "gpt-4o-mini",
  "input": "My email is jane@example.com — can you summarize it?"
}'

run_test "Non-streaming with PII (credit card)" /v1/responses 200 '{
  "model": "gpt-4o-mini",
  "input": "I need help disputing a charge on card 4111 1111 1111 1111"
}'

run_test "No PII" /v1/responses 200 '{
  "model": "gpt-4o-mini",
  "input": "What is the capital of France?"
}'

# ── /v1/messages (Anthropic) ─────────────────────────────────────────────────
echo ""
echo "=== /v1/messages ==="

run_test "Non-streaming with PII (phone)" /v1/messages 200 '{
  "model": "claude-sonnet-4-20250514",
  "max_tokens": 256,
  "messages": [{"role": "user", "content": "Call me at 555-867-5309 to discuss my account"}]
}'

run_test "Non-streaming with PII (SSN)" /v1/messages 200 '{
  "model": "claude-sonnet-4-20250514",
  "max_tokens": 256,
  "messages": [{"role": "user", "content": "My SSN is 123-45-6789"}]
}'

run_test "No PII" /v1/messages 200 '{
  "model": "claude-sonnet-4-20250514",
  "max_tokens": 256,
  "messages": [{"role": "user", "content": "Hello, how are you?"}]
}'

# ── Summary ──────────────────────────────────────────────────────────────────
echo ""
echo "Results: $PASS passed, $FAIL failed"
echo ""
echo "To verify PII was anonymized before reaching the LLM, check the terminal running start_agents.sh"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
