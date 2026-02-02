#!/bin/bash
set -e

# Get the directory where this script is located
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# Navigate to crates directory (bin -> src -> hermesllm -> crates)
cd "$SCRIPT_DIR/../../.."

# Load environment variables silently and run fetch_models
set -a
source hermesllm/src/bin/.env
set +a

cargo run --bin fetch_models --features model-fetch
