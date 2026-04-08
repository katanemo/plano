#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# Run the demo directly against Plano (no agent server needed)
uv run "$SCRIPT_DIR/demo.py"
