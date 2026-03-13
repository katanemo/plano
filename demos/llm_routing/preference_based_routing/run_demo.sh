#!/bin/bash
set -e

# Function to start the demo
start_demo() {
  # Step 1: Check if .env file exists
  if [ -f ".env" ]; then
    echo ".env file already exists. Skipping creation."
  else
    # Step 2: Create `.env` file and set API keys
    if [ -z "$OPENAI_API_KEY" ]; then
      echo "Error: OPENAI_API_KEY environment variable is not set for the demo."
      exit 1
    fi
    if [ -z "$ANTHROPIC_API_KEY" ]; then
      echo "Warning: ANTHROPIC_API_KEY environment variable is not set. Anthropic features may not work."
    fi

    echo "Creating .env file..."
    echo "OPENAI_API_KEY=$OPENAI_API_KEY" > .env
    if [ -n "$ANTHROPIC_API_KEY" ]; then
      echo "ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY" >> .env
    fi
    echo ".env file created with API keys."
  fi

  # Step 3: Optionally start UI services (AnythingLLM, Jaeger)
  # Jaeger must start before Plano so it can bind the OTEL port (4317)
  if [ "$1" == "--with-ui" ]; then
    echo "Starting UI services (AnythingLLM, Jaeger)..."
    docker compose up -d
  fi

  # Step 4: Start Plano
  echo "Starting Plano with config.yaml..."
  planoai up config.yaml
}

# Function to stop the demo
stop_demo() {
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
