#!/usr/bin/env python3
"""
Session Pinning Demo — Iterative Research Agent

Demonstrates how session pinning ensures consistent model selection
across multiple iterations of an agentic loop. Runs the same 5-step
research workflow twice:

  1) Without session pinning — models may switch between iterations
  2) With session pinning    — first iteration pins the model for all subsequent ones

Uses the /routing/v1/chat/completions endpoint (routing decisions only, no LLM calls).
"""

import json
import os
import urllib.request
import uuid

PLANO_URL = os.environ.get("PLANO_URL", "http://localhost:12000")

# Simulates an iterative research agent building a task management app.
# Prompts deliberately alternate between code_generation and complex_reasoning
# intents so that without pinning, different models get selected per step.
RESEARCH_STEPS = [
    "Design a REST API schema for a task management app with users, projects, and tasks",
    "Analyze the trade-offs between SQL and NoSQL databases for this task management system",
    "Write the database models and ORM setup in Python using SQLAlchemy",
    "Review the API design for security vulnerabilities and suggest improvements",
    "Implement the authentication middleware with JWT tokens",
]


def run_research_loop(session_id=None):
    """Run the research agent loop, optionally with session pinning."""
    results = []

    for i, prompt in enumerate(RESEARCH_STEPS, 1):
        headers = {"Content-Type": "application/json"}
        if session_id:
            headers["X-Session-Id"] = session_id

        payload = {
            "model": "gpt-4o-mini",
            "messages": [{"role": "user", "content": prompt}],
        }

        resp = urllib.request.urlopen(
            urllib.request.Request(
                f"{PLANO_URL}/routing/v1/chat/completions",
                data=json.dumps(payload).encode(),
                headers=headers,
            ),
            timeout=10,
        )
        data = json.loads(resp.read())

        model = data.get("model", "unknown")
        route = data.get("route") or "none"
        pinned = data.get("pinned")

        pinned_str = ""
        if pinned is not None:
            pinned_str = f"  pinned={pinned}"

        print(f"  Step {i}: {prompt[:60]:<60s}")
        print(f"          → model={model}  route={route}{pinned_str}")
        print()

        results.append({"step": i, "model": model, "route": route, "pinned": pinned})

    return results


def print_summary(label, results):
    """Print a one-line summary of model consistency."""
    models = [r["model"] for r in results]
    unique = set(models)
    if len(unique) == 1:
        print(f"  ✓ {label}: All 5 steps routed to {models[0]}")
    else:
        print(f"  ✗ {label}: Models varied across steps — {', '.join(unique)}")


def main():
    print("=" * 70)
    print("  Iterative Research Agent — Session Pinning Demo")
    print("=" * 70)
    print()
    print("An agent is building a task management app in 5 iterative steps.")
    print("Each step hits Plano's routing endpoint to pick the best model.")
    print()

    # --- Run 1: Without session pinning ---
    print("-" * 70)
    print("  Run 1: WITHOUT Session Pinning (no X-Session-Id header)")
    print("-" * 70)
    print()
    results_no_pin = run_research_loop(session_id=None)

    # --- Run 2: With session pinning ---
    session_id = str(uuid.uuid4())
    print("-" * 70)
    print(f"  Run 2: WITH Session Pinning (X-Session-Id: {session_id})")
    print("-" * 70)
    print()
    results_pinned = run_research_loop(session_id=session_id)

    # --- Summary ---
    print("=" * 70)
    print("  Summary")
    print("=" * 70)
    print()
    print_summary("Without pinning", results_no_pin)
    print_summary("With pinning   ", results_pinned)
    print()


if __name__ == "__main__":
    main()
