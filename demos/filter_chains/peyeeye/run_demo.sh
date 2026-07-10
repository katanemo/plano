#!/bin/bash
set -e

start_demo() {
  if [ -f ".env" ]; then
    echo ".env file already exists. Skipping creation."
  else
    if [ -z "${PEYEEYE_API_KEY:-}" ]; then
      echo "Error: PEYEEYE_API_KEY environment variable is not set for the demo."
      exit 1
    fi
    if [ -z "${OPENAI_API_KEY:-}" ]; then
      echo "Error: OPENAI_API_KEY environment variable is not set for the demo."
      exit 1
    fi

    echo "Creating .env file..."
    {
      echo "PEYEEYE_API_KEY=$PEYEEYE_API_KEY"
      echo "OPENAI_API_KEY=$OPENAI_API_KEY"
    } > .env
    echo ".env file created."
  fi

  echo "Starting Plano with config.yaml..."
  planoai up config.yaml

  echo "Starting Peyeeye filter service..."
  bash start_agents.sh &
}

stop_demo() {
  echo "Stopping Peyeeye filter service..."
  pkill -f start_agents.sh 2>/dev/null || true

  echo "Stopping Plano..."
  planoai down
}

if [ "$1" == "down" ]; then
  stop_demo
else
  start_demo
fi
