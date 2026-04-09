#!/usr/bin/env python3
"""
verify_affinity.py — Verify that model affinity (session pinning) works correctly.

Sends multiple requests with the same X-Model-Affinity session ID and asserts
that every response is served by the same model, demonstrating that Plano's
session cache is working as expected.

Usage:
    python verify_affinity.py [--url URL] [--rounds N] [--sessions N]
"""

import argparse
import json
import sys
import urllib.error
import urllib.request
from collections import defaultdict

PLANO_URL = "http://localhost:12000/v1/chat/completions"

PROMPTS = [
    "What is 2 + 2?",
    "Name the capital of France.",
    "How many days in a week?",
    "What color is the sky?",
    "Who wrote Romeo and Juliet?",
]

MESSAGES_PER_SESSION = [{"role": "user", "content": prompt} for prompt in PROMPTS]


def chat(url: str, session_id: str | None, message: str) -> dict:
    payload = json.dumps(
        {
            "model": "openai/gpt-4o-mini",
            "messages": [{"role": "user", "content": message}],
        }
    ).encode()

    headers = {"Content-Type": "application/json"}
    if session_id:
        headers["x-model-affinity"] = session_id

    req = urllib.request.Request(url, data=payload, headers=headers, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            return json.loads(resp.read())
    except urllib.error.URLError as e:
        print(f"  ERROR: could not reach Plano at {url}: {e}", file=sys.stderr)
        print("  Is the demo running? Start it with: ./run_demo.sh up", file=sys.stderr)
        sys.exit(1)


def extract_model(response: dict) -> str:
    return response.get("model", "<unknown>")


def run_verification(url: str, rounds: int, num_sessions: int) -> bool:
    print(f"Plano endpoint : {url}")
    print(f"Sessions       : {num_sessions}")
    print(f"Rounds/session : {rounds}")
    print()

    all_passed = True

    # --- Phase 1: Requests without session ID ---
    print("=" * 60)
    print("Phase 1: Requests WITHOUT X-Model-Affinity header")
    print("  (model may vary between requests — that is expected)")
    print("=" * 60)
    models_seen: set[str] = set()
    for i in range(min(rounds, 3)):
        resp = chat(url, None, PROMPTS[i % len(PROMPTS)])
        model = extract_model(resp)
        models_seen.add(model)
        print(f"  Request {i + 1}: model = {model}")
    print(f"  Models seen across {min(rounds, 3)} requests: {models_seen}")
    print()

    # --- Phase 2: Each session should always get the same model ---
    print("=" * 60)
    print("Phase 2: Requests WITH X-Model-Affinity (session pinning)")
    print("  Each session should be pinned to exactly one model.")
    print("=" * 60)

    session_results: dict[str, list[str]] = defaultdict(list)

    for s in range(num_sessions):
        session_id = f"demo-session-{s + 1:03d}"
        print(f"\n  Session '{session_id}':")

        for r in range(rounds):
            resp = chat(url, session_id, PROMPTS[r % len(PROMPTS)])
            model = extract_model(resp)
            session_results[session_id].append(model)
            pinned = " [PINNED]" if r > 0 else " [FIRST — sets affinity]"
            print(f"    Round {r + 1}: model = {model}{pinned}")

    print()
    print("=" * 60)
    print("Results")
    print("=" * 60)

    for session_id, models in session_results.items():
        unique_models = set(models)
        if len(unique_models) == 1:
            print(f"  PASS  {session_id} -> always routed to '{models[0]}'")
        else:
            print(
                f"  FAIL  {session_id} -> inconsistent models across rounds: {unique_models}"
            )
            all_passed = False

    print()
    if all_passed:
        print("All sessions were pinned consistently.")
        print("Redis session cache is working correctly.")
    else:
        print("One or more sessions were NOT pinned consistently.")
        print("Check that Redis is running and Plano is configured with:")
        print("  routing:")
        print("    session_cache:")
        print("      type: redis")
        print("      url: redis://localhost:6379")

    return all_passed


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default=PLANO_URL, help="Plano chat completions URL")
    parser.add_argument(
        "--rounds", type=int, default=4, help="Requests per session (default 4)"
    )
    parser.add_argument(
        "--sessions", type=int, default=3, help="Number of sessions to test (default 3)"
    )
    args = parser.parse_args()

    passed = run_verification(args.url, args.rounds, args.sessions)
    sys.exit(0 if passed else 1)


if __name__ == "__main__":
    main()
