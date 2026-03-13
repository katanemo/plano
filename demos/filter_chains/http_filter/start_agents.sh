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

export PYTHONPATH=./src

log "Starting input_guards HTTP server on port 10500..."
uv run uvicorn rag_agent.input_guards:app --host 0.0.0.0 --port 10500 &
PIDS+=($!)

log "Starting query_rewriter HTTP server on port 10501..."
uv run uvicorn rag_agent.query_rewriter:app --host 0.0.0.0 --port 10501 &
PIDS+=($!)

log "Starting context_builder HTTP server on port 10502..."
uv run uvicorn rag_agent.context_builder:app --host 0.0.0.0 --port 10502 &
PIDS+=($!)

log "Starting response_generator (OpenAI-compatible) on port 10505..."
uv run uvicorn rag_agent.rag_agent:app --host 0.0.0.0 --port 10505 &
PIDS+=($!)

for PID in "${PIDS[@]}"; do
    wait "$PID"
done
