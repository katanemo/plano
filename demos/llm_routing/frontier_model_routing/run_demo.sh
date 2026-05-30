#!/bin/bash
set -e

# ---------------------------------------------------------------------------
# Frontier model routing demo: DigitalOcean Sonnet 4.6 + GPT 5.5 + Opus 4.7
# ---------------------------------------------------------------------------

start_demo() {
  if [ -f ".env" ]; then
    echo ".env file already exists. Skipping creation."
  else
    missing=()
    [ -z "$DO_API_KEY" ]         && missing+=("DO_API_KEY")
    [ -z "$OPENAI_API_KEY" ]     && missing+=("OPENAI_API_KEY")
    [ -z "$ANTHROPIC_API_KEY" ]  && missing+=("ANTHROPIC_API_KEY")

    if [ ${#missing[@]} -ne 0 ]; then
      echo "Error: the following environment variables are not set:"
      for key in "${missing[@]}"; do echo "  - $key"; done
      echo
      echo "Set them in your shell, then re-run this script. Example:"
      echo "  export DO_API_KEY=...        # from https://cloud.digitalocean.com/account/api/tokens"
      echo "  export OPENAI_API_KEY=...    # from https://platform.openai.com/api-keys"
      echo "  export ANTHROPIC_API_KEY=... # from https://console.anthropic.com/"
      exit 1
    fi

    echo "Creating .env file..."
    {
      echo "DO_API_KEY=$DO_API_KEY"
      echo "OPENAI_API_KEY=$OPENAI_API_KEY"
      echo "ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY"
    } > .env
    echo ".env file created."
  fi

  echo "Starting Plano with config.yaml..."
  planoai up config.yaml

  cat <<'EOF'

Plano is up. Try the demo with:
  ./test.sh           # runs three sample prompts and shows which model handled each
  planoai trace        # live router decisions in a separate terminal

Or call any model directly using its alias:
  curl -sS -X POST http://localhost:12000/v1/chat/completions \
    -H "Content-Type: application/json" \
    -d '{"model":"frontier.max","messages":[{"role":"user","content":"hello"}]}' | jq .

EOF
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
