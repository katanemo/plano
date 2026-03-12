#!/bin/bash
set -e

# Function to load environment variables from the .env file
load_env() {
  if [ -f ".env" ]; then
    export $(grep -v '^#' .env | xargs)
  fi
}

# Function to determine the docker-compose file based on the argument
get_compose_file() {
  case "$1" in
  jaeger)
    echo "docker-compose-jaeger.yaml"
    ;;
  logfire)
    echo "docker-compose-logfire.yaml"
    ;;
  signoz)
    echo "docker-compose-signoz.yaml"
    ;;
  honeycomb)
    echo "docker-compose-honeycomb.yaml"
    ;;
  *)
    echo "docker-compose.yaml"
    ;;
  esac
}

# Function to start the demo
start_demo() {
  # Step 1: Determine the docker-compose file
  COMPOSE_FILE=$(get_compose_file "$1" 2>/dev/null)

  # Step 2: Check if .env file exists
  if [ -f ".env" ]; then
    echo ".env file already exists. Skipping creation."
  else
    # Step 3: Check for required environment variables
    if [ -z "$OPENAI_API_KEY" ]; then
      echo "Error: OPENAI_API_KEY environment variable is not set for the demo."
      exit 1
    fi
    if [ "$1" == "logfire" ] && [ -z "$LOGFIRE_API_KEY" ]; then
      echo "Error: LOGFIRE_API_KEY environment variable is required for Logfire."
      exit 1
    fi
    if [ "$1" == "honeycomb" ] && [ -z "$HONEYCOMB_API_KEY" ]; then
      echo "Error: HONEYCOMB_API_KEY environment variable is required for Honeycomb."
      exit 1
    fi

    # Create .env file
    echo "Creating .env file..."
    echo "OPENAI_API_KEY=$OPENAI_API_KEY" >.env
    if [ "$1" == "logfire" ]; then
      echo "LOGFIRE_API_KEY=$LOGFIRE_API_KEY" >>.env
    fi
    echo ".env file created with required API keys."
  fi

  load_env

  if [ "$1" == "logfire" ] && [ -z "$LOGFIRE_API_KEY" ]; then
    echo "Error: LOGFIRE_API_KEY environment variable is required for Logfire."
    exit 1
  fi
  if [ "$1" == "honeycomb" ] && [ -z "$HONEYCOMB_API_KEY" ]; then
    echo "Error: HONEYCOMB_API_KEY environment variable is required for Honeycomb."
    exit 1
  fi

  # Step 4: Optionally start UI services (AnythingLLM, Jaeger, etc.)
  # Jaeger must start before Plano so it can bind the OTEL port (4317)
  if [ "$1" == "--with-ui" ] || [ "$2" == "--with-ui" ]; then
    echo "Starting UI services with $COMPOSE_FILE..."
    docker compose -f "$COMPOSE_FILE" up -d
  fi

  # Step 5: Start Plano
  echo "Starting Plano with config.yaml..."
  planoai up config.yaml

  # Step 6: Start agents natively
  echo "Starting agents..."
  bash start_agents.sh &
}

# Function to stop the demo
stop_demo() {
  # Stop agents
  echo "Stopping agents..."
  pkill -f start_agents.sh 2>/dev/null || true

  # Stop all Docker Compose services if running
  echo "Stopping Docker Compose services..."
  for compose_file in ./docker-compose*.yaml; do
    docker compose -f "$compose_file" down 2>/dev/null || true
  done

  # Stop Plano
  echo "Stopping Plano..."
  planoai down
}

# Main script logic
if [ "$1" == "down" ]; then
  # Call stop_demo with the second argument as the demo to stop
  stop_demo
else
  # Use the argument (jaeger, logfire, signoz, --with-ui) to determine the compose file
  start_demo "$1" "$2"
fi
