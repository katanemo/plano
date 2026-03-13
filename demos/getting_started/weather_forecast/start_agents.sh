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

log "Starting weather_forecast_service on port 18083..."
uv run uvicorn main:app --host 0.0.0.0 --port 18083 &
PIDS+=($!)

for PID in "${PIDS[@]}"; do
    wait "$PID"
done
