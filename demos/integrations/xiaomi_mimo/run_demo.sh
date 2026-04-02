#!/bin/bash
set -e

start_demo() {
  if [ -f ".env" ]; then
    echo ".env file already exists. Skipping creation."
  else
    if [ -z "$MIMO_API_KEY" ]; then
      echo "Error: MIMO_API_KEY environment variable is not set for the demo."
      exit 1
    fi

    echo "Creating .env file..."
    echo "MIMO_API_KEY=$MIMO_API_KEY" > .env
    echo ".env file created with MIMO_API_KEY."
  fi

  echo "Starting Plano with config.yaml..."
  planoai up config.yaml
}

stop_demo() {
  echo "Stopping Plano..."
  planoai down
}

if [ "$1" == "down" ]; then
  stop_demo
else
  start_demo
fi
