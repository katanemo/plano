#!/bin/bash
# ---------------------------------------------------------------------------
# Frontier Model Routing demo — driver script
#
# For each of three intent-biased prompts we:
#   1. Hit POST /routing/v1/chat/completions (Plano's decision-only endpoint)
#      to print the matched route name and the ranked candidate pool.
#   2. Hit POST /v1/chat/completions to actually run the request and print
#      the model that handled it.
#
# Plano runs the orchestrator on every chat completion when top-level
# `routing_preferences` are configured. The `model` field in the request is
# the *fallback* used when no preference matches — we pin it to
# `frontier.fast` so unmatched prompts land on the cheapest tier.
# ---------------------------------------------------------------------------

set -e

GATEWAY=${GATEWAY:-http://localhost:12000}
DECISION_ENDPOINT="$GATEWAY/routing/v1/chat/completions"
CHAT_ENDPOINT="$GATEWAY/v1/chat/completions"

ask() {
  local label="$1"
  local prompt="$2"

  local body
  body="$(jq -n --arg p "$prompt" '{
    "model": "frontier.fast",
    "max_tokens": 256,
    "messages": [{"role":"user","content":$p}]
  }')"

  echo
  echo "=========================================================="
  echo "[$label]"
  echo "prompt: $prompt"
  echo "----------------------------------------------------------"

  # Step 1: decision-only — what would the router pick?
  echo "  routing decision:"
  curl -sS -X POST "$DECISION_ENDPOINT" \
    -H "Content-Type: application/json" \
    -d "$body" \
    | jq '{
        matched_route: .route,
        ranked_models: .models,
        pinned: .pinned
      }' \
    | sed 's/^/    /'

  # Step 2: actually run the request through the chosen model.
  echo "  chat completion:"
  curl -sS -X POST "$CHAT_ENDPOINT" \
    -H "Content-Type: application/json" \
    -d "$body" \
    | jq '{
        routed_to: .model,
        reply: .choices[0].message.content
      }' \
    | sed 's/^/    /'
}

ask "daily conversation -> expects DigitalOcean Sonnet 4.6" \
  "Hey! Give me three fun facts about octopuses I can drop into a dinner conversation."

ask "complex reasoning -> expects OpenAI GPT 5.5" \
  "A train leaves Chicago at 9:14am traveling 72 mph. Another leaves St Louis at 10:02am traveling 65 mph toward Chicago. The cities are 297 miles apart. Walk through the math step by step and give me the time and place they meet."

ask "code generation -> expects Anthropic Opus 4.7" \
  "Write a Rust function that takes a Vec<u8> of UTF-8 bytes and returns a HashMap<char, usize> with grapheme cluster counts. Include unit tests and handle invalid UTF-8 gracefully."

ask "deep analysis -> expects Anthropic Opus 4.7" \
  "Review this Postgres schema for normalization, indexing, and migration risk. Give me a prioritized list of issues:
CREATE TABLE orders (
  id SERIAL PRIMARY KEY,
  customer_email TEXT,
  customer_name TEXT,
  items_json JSONB,
  total NUMERIC,
  created_at TIMESTAMPTZ DEFAULT now()
);"

# ---------------------------------------------------------------------------
# Bonus: pin a routing decision across an agentic loop with X-Model-Affinity.
# Both calls hit the same gateway with the same affinity id, so the second
# call reuses the first call's routing decision instead of reclassifying.
# ---------------------------------------------------------------------------
echo
echo "=========================================================="
echo "[bonus: model affinity across two turns of an agent loop]"
echo "----------------------------------------------------------"

SID="demo-$(date +%s)-$RANDOM"
echo "  X-Model-Affinity: $SID"

turn() {
  local turn_label="$1"
  local prompt="$2"
  echo "  $turn_label:"
  curl -sS -X POST "$CHAT_ENDPOINT" \
    -H "Content-Type: application/json" \
    -H "X-Model-Affinity: $SID" \
    -d "$(jq -n --arg p "$prompt" '{
      "model": "frontier.fast",
      "max_tokens": 128,
      "messages": [{"role":"user","content":$p}]
    }')" \
    | jq '{ routed_to: .model }' \
    | sed 's/^/    /'
}

turn "turn 1 (sets affinity)"  "Plan a small refactor of an auth module — what's the order of operations?"
turn "turn 2 (reuses decision)" "Now write the unit tests for step one."

echo
echo "=========================================================="
echo "Done. Want to inspect routing decisions live? Run:  planoai trace"
echo "=========================================================="
