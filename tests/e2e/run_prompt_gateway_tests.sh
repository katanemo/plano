#!/bin/bash
# Runs remaining prompt/LLM gateway e2e tests (API translation via model listener).
# Requires the plano Docker image to already be built/loaded.
set -e

. ./common_scripts.sh

print_disk_usage

mkdir -p ~/plano_logs
touch ~/plano_logs/modelserver.log

print_debug() {
  log "Received signal to stop"
  log "Printing debug logs for docker"
  log "===================================="
  tail -n 100 ../build.log 2>/dev/null || true
  planoai logs --debug 2>/dev/null | tail -n 100 || true
}

trap 'print_debug' INT TERM ERR

log starting > ../build.log

# Install plano CLI
log "building and installing plano cli"
cd ../../cli
uv sync
uv tool install .
cd -

# Re-sync e2e deps
uv sync

# Start gateway with a model listener config (API translation tests)
log "startup plano gateway with model listener"
cd ../../
planoai down --docker || true
planoai up --docker tests/e2e/config_native_smoke.yaml
cd -

# Run tests
log "running e2e tests for llm/prompt gateway API translation"
uv run pytest test_prompt_gateway.py

# Cleanup
log "shutting down"
planoai down --docker || true
