#!/bin/bash
set -e

PIDS=()

log() { echo "$(date '+%F %T') - $*"; }

cleanup() {
    log "Stopping agents..."
    for PID in "${PIDS[@]}"; do
        kill $PID 2>/dev/null && log "Stopped process $PID"
    done
    exit 0
}

trap cleanup EXIT INT TERM

export LLM_GATEWAY_ENDPOINT=http://localhost:12000/v1

log "Starting langchain weather_agent on port 10510..."
uv run python langchain/weather_agent.py &
PIDS+=($!)

log "Starting crewai flight_agent on port 10520..."
uv run python crewai/flight_agent.py &
PIDS+=($!)

for PID in "${PIDS[@]}"; do
    wait "$PID"
done
