#!/bin/bash
set -e

echo "=== OpenClaw + Plano Routing Demo ==="

# Check prerequisites
command -v docker >/dev/null || { echo "Error: Docker not found"; exit 1; }

# Check/create .env file
if [ -f ".env" ]; then
  echo ".env file already exists"
else
  if [ -z "${MOONSHOT_API_KEY:-}" ]; then
    echo "Error: MOONSHOT_API_KEY not set"
    exit 1
  fi
  if [ -z "${ANTHROPIC_API_KEY:-}" ]; then
    echo "Error: ANTHROPIC_API_KEY not set"
    exit 1
  fi
  echo "Creating .env file..."
  echo "MOONSHOT_API_KEY=$MOONSHOT_API_KEY" > .env
  echo "ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY" >> .env
fi

start_demo() {
  echo "Starting Plano..."
  planoai up --service plano --foreground
}

stop_demo() {
  planoai down
}

if [ "${1:-}" == "down" ]; then
  stop_demo
else
  start_demo
  echo ""
  echo "=== Plano is running on http://localhost:12000 ==="
  echo ""
  echo "Configure OpenClaw to use Plano as its LLM endpoint:"
  echo '  In ~/.openclaw/openclaw.json, set:'
  echo '    { "agent": { "model": "kimi-k2.5", "baseURL": "http://127.0.0.1:12000/v1" } }'
  echo ""
  echo "Then run: openclaw onboard --install-daemon"
fi
