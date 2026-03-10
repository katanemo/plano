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

  # Step 3: Start Plano
  echo "Starting Plano with config.yaml..."
  planoai up config.yaml

  # Step 4: Start services
  echo "Starting services using Docker Compose..."
  docker compose up -d
}

# Function to stop the demo
stop_demo() {
  # Step 1: Stop Docker Compose services
  echo "Stopping Docker Compose services..."
  docker compose down

  # Step 2: Stop Plano
  echo "Stopping Plano..."
  planoai down
}

# Main script logic
if [ "$1" == "down" ]; then
  stop_demo
else
  start_demo
fi
