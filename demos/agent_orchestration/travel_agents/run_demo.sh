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
    if [ -z "$AEROAPI_KEY" ]; then
      echo "Error: AEROAPI_KEY environment variable is not set for the demo."
      exit 1
    fi

    echo "Creating .env file..."
    echo "OPENAI_API_KEY=$OPENAI_API_KEY" > .env
    echo "AEROAPI_KEY=$AEROAPI_KEY" >> .env
    echo ".env file created with API keys."
  fi

  # Step 3: Start Plano
  echo "Starting Plano with config.yaml..."
  planoai up config.yaml

  # Step 4: Start agents and services
  echo "Starting agents using Docker Compose..."
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
