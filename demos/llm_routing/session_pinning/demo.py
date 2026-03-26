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


STEP_LABELS = [
    "Design REST API schema",
    "Analyze SQL vs NoSQL trade-offs",
    "Write SQLAlchemy database models",
    "Review API security vulnerabilities",
    "Implement JWT auth middleware",
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

        results.append({"step": i, "model": model, "route": route, "pinned": pinned})

    return results


def print_results_table(results):
    """Print results as a compact aligned table."""
    label_width = max(len(l) for l in STEP_LABELS)
    for r in results:
        step = r["step"]
        label = STEP_LABELS[step - 1]
        model = r["model"]
        pinned = r["pinned"]

        # Shorten model names for readability
        short_model = model.replace("anthropic/", "").replace("openai/", "")

        pin_indicator = ""
        if pinned is True:
            pin_indicator = "  ◀ pinned"
        elif pinned is False:
            pin_indicator = "  ◀ routed"

        print(f"  {step}. {label:<{label_width}}  →  {short_model}{pin_indicator}")


def print_summary(label, results):
    """Print a one-line summary of model consistency."""
    models = [r["model"] for r in results]
    unique = sorted(set(models))
    if len(unique) == 1:
        short = models[0].replace("anthropic/", "").replace("openai/", "")
        print(f"  ✓ {label}: All 5 steps → {short}")
    else:
        short = [m.replace("anthropic/", "").replace("openai/", "") for m in unique]
        print(f"  ✗ {label}: Models varied → {', '.join(short)}")


def main():
    print()
    print("  ╔══════════════════════════════════════════════════════════════╗")
    print("  ║       Session Pinning Demo — Iterative Research Agent        ║")
    print("  ╚══════════════════════════════════════════════════════════════╝")
    print()
    print("  An agent builds a task management app in 5 steps.")
    print("  Each step asks Plano's router which model to use.")
    print()

    # --- Run 1: Without session pinning ---
    print("  ┌──────────────────────────────────────────────────────────────┐")
    print("  │  Run 1: WITHOUT Session Pinning                              │")
    print("  └──────────────────────────────────────────────────────────────┘")
    print()
    results_no_pin = run_research_loop(session_id=None)
    print_results_table(results_no_pin)
    print()

    # --- Run 2: With session pinning ---
    session_id = str(uuid.uuid4())
    short_sid = session_id[:8]
    print(f"  ┌──────────────────────────────────────────────────────────────┐")
    print(f"  │  Run 2: WITH Session Pinning (session: {short_sid}…)         │")
    print(f"  └──────────────────────────────────────────────────────────────┘")
    print()
    results_pinned = run_research_loop(session_id=session_id)
    print_results_table(results_pinned)
    print()

    # --- Summary ---
    print("  ┌──────────────────────────────────────────────────────────────┐")
    print("  │  Summary                                                    │")
    print("  └──────────────────────────────────────────────────────────────┘")
    print()
    print_summary("Without pinning", results_no_pin)
    print_summary("With pinning   ", results_pinned)
    print()


if __name__ == "__main__":
    main()
