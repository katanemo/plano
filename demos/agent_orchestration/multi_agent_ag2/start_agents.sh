#!/bin/bash
set -e

# LLM calls route through Plano's gateway (not directly to OpenAI)
export LLM_GATEWAY_ENDPOINT="http://localhost:12000/v1"

PIDS=()

cleanup() {
  echo "Stopping agents..."
  for pid in "${PIDS[@]}"; do
    kill "$pid" 2>/dev/null || true
  done
}
trap cleanup EXIT INT TERM

echo "Starting AG2 Research Agent on port 10530..."
(cd ag2 && uv run python research_agent.py) &
AGENT_PID=$!
PIDS+=($AGENT_PID)

echo "AG2 Research Agent started (PID: $AGENT_PID)"

# Wait for agent to be ready
sleep 3
echo "Agents ready."

wait
