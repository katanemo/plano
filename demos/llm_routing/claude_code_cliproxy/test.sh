#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

# These tests use mocks and never start CLIProxyAPI or Plano.
for test_file in "$SCRIPT_DIR"/tests/test_*.sh; do
  bash "$test_file"
done
