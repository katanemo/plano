#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
export PLANO_URL="${PLANO_URL:-http://localhost:12000}"

echo "Running session pinning demo..."
echo "PLANO_URL=$PLANO_URL"
echo ""

python3 "$SCRIPT_DIR/demo.py"
