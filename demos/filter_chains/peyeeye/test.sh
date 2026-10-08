#!/usr/bin/env bash
set -euo pipefail

BASE_URL="http://localhost:12000"
PASS=0
FAIL=0

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

run_test() {
    local name="$1"
    local path="$2"
    local expected_code="$3"
    local body="$4"

    http_code=$(curl -s -o /tmp/peyeeye_test_body -w "%{http_code}" \
        -X POST "$BASE_URL$path" \
        -H "Content-Type: application/json" \
        -d "$body")

    if [ "$http_code" -eq "$expected_code" ]; then
        echo "  PASS  $name (HTTP $http_code)"
        PASS=$((PASS + 1))
    else
        echo "  FAIL  $name -- expected $expected_code, got $http_code"
        echo "        Body: $(cat /tmp/peyeeye_test_body)"
        FAIL=$((FAIL + 1))
    fi
}

echo ""
echo "=== /v1/chat/completions ==="

run_test "Non-streaming with PII" /v1/chat/completions 200 '{
  "model": "gpt-4o-mini",
  "messages": [{"role": "user", "content": "Email me at jane@example.com"}],
  "stream": false
}'

run_test "No PII" /v1/chat/completions 200 '{
  "model": "gpt-4o-mini",
  "messages": [{"role": "user", "content": "What is 2+2?"}],
  "stream": false
}'

echo ""
echo "=== /v1/messages (Anthropic) ==="

run_test "Non-streaming with PII (SSN)" /v1/messages 200 '{
  "model": "claude-sonnet-4-20250514",
  "max_tokens": 256,
  "messages": [{"role": "user", "content": "My SSN is 123-45-6789"}]
}'

echo ""
echo "Results: $PASS passed, $FAIL failed"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
