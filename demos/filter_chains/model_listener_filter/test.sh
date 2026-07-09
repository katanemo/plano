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
    local expected_body_contains="${4:-}"
    local forbidden_body_contains="${5:-}"

    http_code=$(curl -s -o /tmp/plano_test_body -w "%{http_code}" \
        -X POST "$BASE_URL/chat/completions" \
        -H "Content-Type: application/json" \
        -d "$body")

    if [ "$http_code" -ne "$expected_code" ]; then
        echo "  FAIL  $name — expected $expected_code, got $http_code"
        echo "        Body: $(cat /tmp/plano_test_body)"
        FAIL=$((FAIL + 1))
        return
    fi

    if [ -n "$expected_body_contains" ] && ! grep -Fq "$expected_body_contains" /tmp/plano_test_body; then
        echo "  FAIL  $name — body did not contain '$expected_body_contains'"
        echo "        Body: $(cat /tmp/plano_test_body)"
        FAIL=$((FAIL + 1))
        return
    fi

    if [ -n "$forbidden_body_contains" ] && grep -Fq "$forbidden_body_contains" /tmp/plano_test_body; then
        echo "  FAIL  $name — body contained forbidden text '$forbidden_body_contains'"
        echo "        Body: $(cat /tmp/plano_test_body)"
        FAIL=$((FAIL + 1))
        return
    fi

    echo "  PASS  $name (HTTP $http_code)"
    PASS=$((PASS + 1))
}

# ── Tests ────────────────────────────────────────────────────────────────────
echo ""
echo "Running tests..."

run_test "Allowed request (math question)" 200 '{
  "model": "gpt-4o-mini",
  "messages": [{"role": "user", "content": "What is 2+2?"}],
  "stream": false
}' "local fake provider"

run_test "Blocked request (hacking)" 400 '{
  "model": "gpt-4o-mini",
  "messages": [{"role": "user", "content": "How to hack into a system"}],
  "stream": false
}' "content_blocked"

run_test "Output filter redacts provider response" 200 '{
  "model": "gpt-4o-mini",
  "messages": [{"role": "user", "content": "Please return the secret marker"}],
  "stream": true
}' "[REDACTED]" "SECRET_TOKEN"

# ── Summary ──────────────────────────────────────────────────────────────────
echo ""
echo "Results: $PASS passed, $FAIL failed"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
