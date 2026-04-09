#!/bin/bash
set -e

start_demo() {
  if [ -f ".env" ]; then
    echo ".env file already exists. Skipping creation."
  else
    if [ -z "$OPENAI_API_KEY" ]; then
      echo "Error: OPENAI_API_KEY environment variable is not set."
      exit 1
    fi

    echo "Creating .env file..."
    echo "OPENAI_API_KEY=$OPENAI_API_KEY" > .env
    echo ".env file created."
  fi

  if [ "$1" == "--with-ui" ]; then
    echo "Starting UI services (Jaeger)..."
    docker compose up -d
  fi

  echo "Starting Plano with config.yaml..."
  planoai up config.yaml

  echo "Installing dependencies..."
  uv sync

  echo "Starting AG2 research agent..."
  ./start_agents.sh &
}

stop_demo() {
  echo "Stopping agents..."
  pkill -f start_agents.sh 2>/dev/null || true
  pkill -f research_agent.py 2>/dev/null || true
  docker compose down 2>/dev/null || true
  echo "Stopping Plano..."
  planoai down
}

if [ "$1" == "down" ]; then
  stop_demo
else
  start_demo "$1"
fi
