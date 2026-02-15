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
  echo "Starting Plano with arch_config_with_aliases.yaml..."
  planoai up arch_config_with_aliases.yaml

  echo "\n\nPlano started successfully."
  echo "Please run the following CURL command to test model alias routing. Additional instructions are in the README.md file. \n"
  echo "curl -sS -X POST \"http://localhost:12000/v1/chat/completions\" \
    -H \"Authorization: Bearer test-key\" \
    -H \"Content-Type: application/json\" \
    -d '{
      \"model\": \"arch.summarize.v1\",
      \"max_tokens\": 50,
      \"messages\": [
        { \"role\": \"user\",
          \"content\": \"Hello, please respond with exactly: Hello from alias arch.summarize.v1!\"
        }
      ]
    }' | jq ."
}

# Function to stop the demo
stop_demo() {
  # Step 2: Stop Plano
  echo "Stopping Plano..."
  planoai down
}

# Main script logic
if [ "$1" == "down" ]; then
  stop_demo
else
  # Default action is to bring the demo up
  start_demo
fi
