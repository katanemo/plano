# #!/bin/bash
# set -e

# WAIT_FOR_PIDS=()

# log() {
#   timestamp=$(python3 -c 'from datetime import datetime; print(datetime.now().strftime("%Y-%m-%d %H:%M:%S,%f")[:23])')
#   message="$*"
#   echo "$timestamp - $message"
# }

# cleanup() {
#     log "Caught signal, terminating all user processes ..."
#     for PID in "${WAIT_FOR_PIDS[@]}"; do
#         if kill $PID 2> /dev/null; then
#             log "killed process: $PID"
#         fi
#     done
#     exit 1
# }

# trap cleanup EXIT

# log "Starting input_guards agent on port 10500/mcp..."
# uv run python -m rag_agent --rest-server --host 0.0.0.0 --rest-port 10500 --agent input_guards &
# WAIT_FOR_PIDS+=($!)

# log "Starting query_rewriter agent on port 10501/mcp..."
# uv run python -m rag_agent --rest-server --host 0.0.0.0 --rest-port 10501 --agent query_rewriter &
# WAIT_FOR_PIDS+=($!)

# log "Starting context_builder agent on port 10502/mcp..."
# uv run python -m rag_agent --rest-server --host 0.0.0.0 --rest-port 10502 --agent context_builder &
# WAIT_FOR_PIDS+=($!)

# # log "Starting response_generator agent on port 10400..."
# # uv run python -m rag_agent --host 0.0.0.0 --port 10400 --agent response_generator &
# # WAIT_FOR_PIDS+=($!)

# log "Starting response_generator agent on port 10505..."
# uv run python -m rag_agent --rest-server --host 0.0.0.0 --rest-port 10505 --agent response_generator &
# WAIT_FOR_PIDS+=($!)

# for PID in "${WAIT_FOR_PIDS[@]}"; do
#     wait "$PID"
# done




#!/bin/bash
set -e

export PYTHONPATH=/app/src

pids=()

log() { echo "$(date '+%F %T') - $*"; }

log "Starting input_guards HTTP server on :10500"
uv run uvicorn rag_agent.input_guards:app --host 0.0.0.0 --port 10500 &
pids+=($!)

log "Starting query_rewriter HTTP server on :10501"
uv run uvicorn rag_agent.query_rewriter:app --host 0.0.0.0 --port 10501 &
pids+=($!)

log "Starting context_builder HTTP server on :10502"
uv run uvicorn rag_agent.context_builder:app --host 0.0.0.0 --port 10502 &
pids+=($!)

log "Starting response_generator (OpenAI-compatible) on :10505"
uv run uvicorn rag_agent.rag_agent:app --host 0.0.0.0 --port 10505 &
pids+=($!)

for PID in "${pids[@]}"; do
    wait "$PID"
done
