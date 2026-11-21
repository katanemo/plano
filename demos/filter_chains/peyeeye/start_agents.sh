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

if [ -z "${PEYEEYE_API_KEY:-}" ]; then
    log "ERROR: PEYEEYE_API_KEY is not set."
    exit 1
fi

log "Starting Peyeeye filter service on port 10502..."
uv run uvicorn peyeeye:app --host 0.0.0.0 --port 10502 &
PIDS+=($!)

for PID in "${PIDS[@]}"; do
    wait "$PID"
done
