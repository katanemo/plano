#!/bin/bash
set -eu

# load demo name from arguments
if [ $# -eq 0 ]; then
  echo "No demo names provided. Please provide demo names as arguments."
  # print usage
  echo "Usage: $0 <demo_name1> <demo_name2> ..."
  exit 1
fi

# extract demo names from arguments
DEMOS="$@"

echo "Running tests for demos: $DEMOS"

run_hurl_with_retries() {
  local demo_name="$1"
  local max_attempts=1
  local attempt=1

  if [ "$demo_name" = "llm_routing/preference_based_routing" ]; then
    max_attempts=3
  fi

  while true; do
    if hurl hurl_tests/*.hurl; then
      return 0
    fi

    if [ "$attempt" -ge "$max_attempts" ]; then
      return 1
    fi

    attempt=$((attempt + 1))
    echo "hurl failed for $demo_name, retrying (attempt $attempt/$max_attempts) ..."
    sleep 2
  done
}

for demo in $DEMOS
do
  echo "******************************************"
  echo "Running tests for $demo ..."
  echo "****************************************"
  cd ../../$demo
  echo "starting plano"
  planoai up --docker config.yaml
  echo "starting docker containers"
  # only execute docker compose if demo is llm_routing/preference_based_routing
  if [ "$demo" == "llm_routing/preference_based_routing" ]; then
    echo "starting docker compose for $demo"
    docker compose -f docker-compose.yaml up -d 2>&1 > /dev/null
  else
    echo "skipping docker compose for $demo"
  fi
  echo "starting hurl tests"
  if ! run_hurl_with_retries "$demo"; then
    echo "Hurl tests failed for $demo"
    echo "docker logs for plano:"
    docker logs plano | tail -n 100
    exit 1
  fi
  echo "stopping docker containers and plano"
  planoai down --docker
  docker compose down -v
  cd ../../shared/test_runner
done
