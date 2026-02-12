#!/usr/bin/env bash
set -euo pipefail

# Demo: Run Plano natively (without Docker)
#
# Prerequisites:
#   - Rust toolchain with wasm32-wasip1 target:
#       rustup target add wasm32-wasip1
#   - planoai CLI installed:
#       cd cli && uv sync
#   - OPENAI_API_KEY set in environment or .env file

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "=== Plano Native Mode Demo ==="
echo ""

# Check prerequisites
if ! command -v cargo &>/dev/null; then
    echo "Error: cargo not found. Install Rust: https://rustup.rs"
    exit 1
fi

if ! rustup target list --installed | grep -q wasm32-wasip1; then
    echo "Error: wasm32-wasip1 target not installed."
    echo "  Run: rustup target add wasm32-wasip1"
    exit 1
fi

if ! command -v planoai &>/dev/null; then
    echo "Error: planoai CLI not found."
    echo "  Run: cd cli && uv sync && uv run planoai --help"
    exit 1
fi

# Step 1: Build native artifacts
echo "Step 1: Building WASM plugins and brightstaff..."
planoai build --native
echo ""

# Step 2: Start Plano natively
echo "Step 2: Starting Plano in native mode..."
echo "  Config: ${SCRIPT_DIR}/config.yaml"
echo ""
planoai up "${SCRIPT_DIR}/config.yaml" --native --foreground
