#!/usr/bin/env bash
set -euo pipefail

BASE_URL="http://localhost:12000/v1"
PASS=0
FAIL=0

# ── Wait for Plano to be ready ──────────────────────────────────────────────
echo "Waiting for Plano to be ready..."
for i in $(seq 1 30); do
    if curl -sf "$BASE_URL/models" > /dev/null 2>&1; then
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
    local expected_code="$2"
    local body="$3"

    http_code=$(curl -s -o /tmp/plano_test_body -w "%{http_code}" \
        -X POST "$BASE_URL/chat/completions" \
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

# ── Tests ────────────────────────────────────────────────────────────────────
echo ""
echo "Running tests..."

run_test "Non-streaming with PII (email + phone)" 200 '{
  "model": "gpt-4o-mini",
  "messages": [{"role": "user", "content": "Contact me at john@example.com or call 555-123-4567"}],
  "stream": false
}'

run_test "Streaming with PII (SSN)" 200 '{
  "model": "gpt-4o-mini",
  "messages": [{"role": "user", "content": "My SSN is 123-45-6789, please help me file taxes"}],
  "stream": true
}'

run_test "No PII (clean message)" 200 '{
  "model": "gpt-4o-mini",
  "messages": [{"role": "user", "content": "What is 2+2?"}],
  "stream": false
}'

run_test "Multiple PII types" 200 '{
  "model": "gpt-4o-mini",
  "messages": [{"role": "user", "content": "Email: test@test.com, Phone: 555-867-5309, SSN: 987-65-4321, Card: 4111 1111 1111 1111"}],
  "stream": false
}'

# ── Summary ──────────────────────────────────────────────────────────────────
echo ""
echo "Results: $PASS passed, $FAIL failed"
echo ""
echo "To verify PII was anonymized before reaching the LLM, check filter logs:"
echo "  docker compose logs pii-service"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
