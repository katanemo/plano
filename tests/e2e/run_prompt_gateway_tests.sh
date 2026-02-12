#!/bin/bash
# Runs the prompt_gateway e2e test suite.
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

# Start weather_forecast service (needed for prompt_gateway tests)
log "building and running weather_forecast service"
cd ../../demos/samples_python/weather_forecast/
docker compose up weather_forecast_service --build -d
cd -

# Start gateway with prompt_gateway config
log "startup arch gateway with function calling demo"
cd ../../
planoai down || true
planoai up demos/samples_python/weather_forecast/config.yaml
cd -

# Run tests
log "running e2e tests for prompt gateway"
uv run pytest test_prompt_gateway.py

# Cleanup
log "shutting down"
planoai down || true
cd ../../demos/samples_python/weather_forecast
docker compose down
cd -
