#!/bin/bash
# Test ChatGPT subscription routing through Plano
# Prerequisites: planoai chatgpt login && planoai up config.yaml

set -e

echo "Testing ChatGPT subscription via Plano Responses API..."
echo ""

curl -s http://localhost:12000/v1/responses \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-5.2",
    "input": "What is 2 + 2? Reply in one word."
  }' | python3 -m json.tool

echo ""
echo "Done."
