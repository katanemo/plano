#!/bin/bash
set -e

# Function to start the demo
start_demo() {
  # Step 1: Check if .env file exists
  if [ -f ".env" ]; then
    echo ".env file already exists. Skipping creation."
  else
    # Step 2: Create `.env` file and set OpenAI key
    if [ -z "$OPENAI_API_KEY" ]; then
      echo "Error: OPENAI_API_KEY environment variable is not set for the demo."
      exit 1
    fi

    echo "Creating .env file..."
    echo "OPENAI_API_KEY=$OPENAI_API_KEY" > .env
    echo ".env file created with OPENAI_API_KEY."
  fi

  # Step 3: Optionally start UI services (AnythingLLM)
  # UI services must start before Plano to avoid OTEL port conflicts
  if [ "$1" == "--with-ui" ]; then
    echo "Starting UI services (AnythingLLM)..."
    docker compose up -d
  fi

  # Step 4: Start Plano
  echo "Starting Plano with config.yaml..."
  planoai up config.yaml

  # Step 5: Start agents natively
  echo "Starting agents..."
  bash start_agents.sh &
}

# Function to stop the demo
stop_demo() {
  # Stop agents
  echo "Stopping agents..."
  pkill -f start_agents.sh 2>/dev/null || true

  # Stop Docker Compose services if running
  docker compose down 2>/dev/null || true

  # Stop Plano
  echo "Stopping Plano..."
  planoai down
}

# Main script logic
if [ "$1" == "down" ]; then
  stop_demo
else
  start_demo "$1"
fi
