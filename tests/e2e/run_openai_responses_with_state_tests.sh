#!/bin/bash
# Run OpenAI responses API with state storage e2e tests
# if any of the commands fail, the script will exit
set -e

. ./common_scripts.sh

print_disk_usage

mkdir -p ~/plano_logs
touch ~/plano_logs/modelserver.log

print_debug() {
  log "Received signal to stop"
  log "Printing debug logs for docker"
  log "===================================="
  tail -n 100 ../build.log
  planoai logs --debug | tail -n 100
}

trap 'print_debug' INT TERM ERR

log starting > ../build.log

log building and installing plano cli
log ==================================
cd ../../cli
poetry install
cd -

log building docker image for arch gateway
log ======================================
cd ../../
planoai build
cd -

# Once we build plano we have to install the dependencies again to a new virtual environment.
poetry install

log startup arch gateway with state storage for openai responses api client demo
cd ../../
planoai down
planoai up tests/e2e/config_memory_state_v1_responses.yaml
cd -

log running e2e tests for openai responses api client with state
log ============================================================
poetry run pytest test_openai_responses_api_client_with_state.py

log shutting down the arch gateway service
log =======================================
planoai down
