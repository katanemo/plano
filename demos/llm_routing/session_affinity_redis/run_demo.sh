#!/usr/bin/env bash
set -euo pipefail

DEMO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

load_env() {
  if [ -f "$DEMO_DIR/.env" ]; then
    set -a
    # shellcheck disable=SC1091
    source "$DEMO_DIR/.env"
    set +a
  fi
}

check_prereqs() {
  local missing=()
  command -v docker   >/dev/null 2>&1 || missing+=("docker")
  command -v planoai  >/dev/null 2>&1 || missing+=("planoai (pip install planoai)")
  if [ ${#missing[@]} -gt 0 ]; then
    echo "ERROR: missing required tools: ${missing[*]}"
    exit 1
  fi

  if [ -z "${OPENAI_API_KEY:-}" ]; then
    echo "ERROR: OPENAI_API_KEY is not set."
    echo "       Create a .env file or export the variable before running."
    exit 1
  fi
}

start_demo() {
  echo "==> Starting Redis + Jaeger..."
  docker compose -f "$DEMO_DIR/docker-compose.yaml" up -d

  echo "==> Waiting for Redis to be ready..."
  local retries=0
  until docker exec plano-session-redis redis-cli ping 2>/dev/null | grep -q PONG; do
    retries=$((retries + 1))
    if [ $retries -ge 15 ]; then
      echo "ERROR: Redis did not become ready in time"
      exit 1
    fi
    sleep 1
  done
  echo "    Redis is ready."

  echo "==> Starting Plano..."
  planoai up "$DEMO_DIR/config.yaml"

  echo ""
  echo "Demo is running!"
  echo ""
  echo "  Model endpoint:  http://localhost:12000/v1/chat/completions"
  echo "  Jaeger UI:       http://localhost:16686"
  echo "  Redis:           localhost:6379"
  echo ""
  echo "Run the verification script to confirm session pinning:"
  echo "  python $DEMO_DIR/verify_affinity.py"
  echo ""
  echo "Stop the demo with: $0 down"
}

stop_demo() {
  echo "==> Stopping Plano..."
  planoai down 2>/dev/null || true

  echo "==> Stopping Docker services..."
  docker compose -f "$DEMO_DIR/docker-compose.yaml" down

  echo "Demo stopped."
}

usage() {
  echo "Usage: $0 [up|down]"
  echo ""
  echo "  up    Start Redis, Jaeger, and Plano (default)"
  echo "  down  Stop all services"
}

load_env

case "${1:-up}" in
  up)
    check_prereqs
    start_demo
    ;;
  down)
    stop_demo
    ;;
  *)
    usage
    exit 1
    ;;
esac
