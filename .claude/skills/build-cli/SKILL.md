---
name: build-cli
description: Build and install the Python CLI (planoai). Use after making changes to cli/ code to install locally.
---

1. `cd cli && uv sync` — ensure dependencies are installed
2. `cd cli && uv tool install --editable .` — install the CLI locally
3. Verify the installation: `cd cli && uv run planoai --help`

If the build or install fails, diagnose and fix the issues.
