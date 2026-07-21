#!/bin/bash
set -e

# Routing Budget demo — drives the /routing decision endpoint to show:
#   1. implicit session pinning (same session across turns, going warm)
#   2. the routing budget vetoing an unaffordable model switch
#
# Prereqs: `planoai up config.yaml` running, plus `curl` and `jq`.
# See GUIDE.md for the full walkthrough and how to flip the veto into an allow.

PLANO_URL="${PLANO_URL:-http://localhost:12000}"
METRICS_URL="${METRICS_URL:-http://localhost:9092}"

echo "=== Routing Budget Demo ==="
echo ""
echo "Uses the /routing/v1/chat/completions decision endpoint (no LLM call)."
echo "Watch session_id stay stable and pinned flip false -> true, then watch"
echo "the budget retain the warm anchor instead of following the router."
echo ""

# --- Turn 1: pin the session (a code-generation prompt) ---
echo "--- 1. Turn 1: pin the session (creates the binding) ---"
echo ""
curl -s "$PLANO_URL/routing/v1/chat/completions" \
  -H 'Content-Type: application/json' \
  -d '{"model": "openai/gpt-4o-mini", "messages": [
    {"role": "system", "content": "You are a senior Rust engineer."},
    {"role": "user", "content": "Write a Rust function that reverses a linked list."}
  ]}' | jq '{model: .models[0], session_id, pinned}'
echo ""
echo "    Expect: model=anthropic/claude-sonnet-4-6, an implicit:… session_id, pinned=false"
echo ""

# --- Turn 2: same system prompt + same first message, one turn later ---
echo "--- 2. Turn 2: same session, warm, router proposes a different model ---"
echo ""
curl -s "$PLANO_URL/routing/v1/chat/completions" \
  -H 'Content-Type: application/json' \
  -d '{"model": "openai/gpt-4o-mini", "messages": [
    {"role": "system", "content": "You are a senior Rust engineer."},
    {"role": "user", "content": "Write a Rust function that reverses a linked list."},
    {"role": "assistant", "content": "Here is an idiomatic in-place reversal for a singly linked list:\n\n```rust\ntype Link = Option<Box<Node>>;\n\nstruct Node {\n    val: i32,\n    next: Link,\n}\n\nfn reverse(mut head: Link) -> Link {\n    let mut prev: Link = None;\n    while let Some(mut node) = head {\n        head = node.next.take();\n        node.next = prev;\n        prev = Some(node);\n    }\n    prev\n}\n```\n\nIt walks the list once, moving the next pointer of each node to its predecessor."},
    {"role": "user", "content": "Now explain its time complexity in plain English — no code."}
  ]}' | jq '{model: .models[0], session_id, pinned}'
echo ""
echo "    Expect: SAME session_id as turn 1, pinned=true. If the router proposed"
echo "    openai/gpt-4o (code understanding), the budget vetoed the switch and"
echo "    model stays anthropic/claude-sonnet-4-6 (the warm anchor)."
echo ""

# --- Switch decisions metric ---
echo "--- 3. Switch decisions (why the budget decided what it did) ---"
echo ""
curl -s "$METRICS_URL/metrics" | grep session_switch_decisions || true
echo ""
echo "    over_cap  = switch vetoed, anchor retained"
echo "    free      = cheaper/affordable switch allowed"
echo "    same_anchor = router did not propose a switch this turn"
echo ""

echo "=== Demo Complete ==="
echo ""
echo "To see the switch ALLOWED instead of vetoed: comment out the routing_budget"
echo "block in config.yaml (or raise max_overhead_pct), then 'planoai down &&"
echo "planoai up config.yaml' and re-run — turn 2 will follow the router to"
echo "openai/gpt-4o. See GUIDE.md for details."
