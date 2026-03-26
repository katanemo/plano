#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
export PLANO_URL="${PLANO_URL:-http://localhost:12000}"
export AGENT_PORT="${AGENT_PORT:-8000}"
export AGENT_URL="http://localhost:$AGENT_PORT"

cleanup() {
    [ -n "$AGENT_PID" ] && kill "$AGENT_PID" 2>/dev/null
}
trap cleanup EXIT INT TERM

# Start the agent in the background
"$SCRIPT_DIR/start_agents.sh" &
AGENT_PID=$!

# Run the demo client
uv run "$SCRIPT_DIR/demo.py"
