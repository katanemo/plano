#!/bin/bash
# Run model alias routing e2e tests
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

log startup arch gateway with model alias routing demo
cd ../../
planoai down
planoai up demos/use_cases/model_alias_routing/config_with_aliases.yaml
cd -

log running e2e tests for model alias routing
log ========================================
poetry run pytest test_model_alias_routing.py

log shutting down the arch gateway service
log =======================================
planoai down
