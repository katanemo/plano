#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PIDS=()

log() { echo "$(date '+%F %T') - $*"; }

cleanup() {
    log "Stopping agents..."
    for PID in "${PIDS[@]}"; do
        kill "$PID" 2>/dev/null && log "Stopped process $PID"
    done
    exit 0
}

trap cleanup EXIT INT TERM

export PLANO_URL="${PLANO_URL:-http://localhost:12000}"
export AGENT_PORT="${AGENT_PORT:-8000}"

log "Starting research_agent on port $AGENT_PORT..."
uv run "$SCRIPT_DIR/agent.py" &
PIDS+=($!)

for PID in "${PIDS[@]}"; do
    wait "$PID"
done
