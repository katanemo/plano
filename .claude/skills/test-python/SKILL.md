---
name: test-python
description: Run Python CLI tests. Use after making changes to cli/ code.
---

1. `cd cli && uv sync` — ensure dependencies are installed
2. `cd cli && uv run pytest -v` — run all tests

If tests fail, diagnose and fix the issues.
