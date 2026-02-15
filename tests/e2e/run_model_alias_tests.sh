#!/bin/bash
# Runs the model_alias_routing + openai responses API e2e test suites.
# These share the same gateway config so they run together.
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

# Start gateway with model alias routing config
log "startup plano gateway with model alias routing demo"
cd ../../
planoai down || true
planoai up demos/llm_routing/model_alias_routing/config_with_aliases.yaml
cd -

# Run both test suites that share this config in a single pytest invocation
log "running e2e tests for model alias routing + openai responses api"
uv run pytest -n auto test_model_alias_routing.py test_openai_responses_api_client.py

# Cleanup
log "shutting down"
planoai down || true
