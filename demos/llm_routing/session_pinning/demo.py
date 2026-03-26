#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["httpx>=0.27"]
# ///
"""
Session Pinning Demo — Research Agent client

Sends the same query to the Research Agent twice — once without a session ID
and once with one — and compares the routing trace to show how session pinning
keeps the model consistent across the LLM's tool-calling loop.

Requires the agent to already be running (start it with ./start_agents.sh).

Usage:
    uv run demo.py
    AGENT_URL=http://localhost:8000 uv run demo.py
"""

import asyncio
import os
import uuid

import httpx

AGENT_URL = os.environ.get("AGENT_URL", "http://localhost:8000")

QUERY = (
    "Should we use PostgreSQL or MongoDB for a high-traffic e-commerce backend "
    "that needs strong consistency for orders but flexible schemas for products?"
)


# ---------------------------------------------------------------------------
# Client helpers
# ---------------------------------------------------------------------------


async def wait_for_agent(timeout: int = 30) -> bool:
    async with httpx.AsyncClient() as client:
        for _ in range(timeout * 2):
            try:
                r = await client.get(f"{AGENT_URL}/health", timeout=1.0)
                if r.status_code == 200:
                    return True
            except Exception:
                pass
            await asyncio.sleep(0.5)
    return False


async def ask_agent(query: str, session_id: str | None = None) -> dict:
    headers: dict[str, str] = {}
    if session_id:
        headers["X-Session-Id"] = session_id

    async with httpx.AsyncClient(timeout=120.0) as client:
        r = await client.post(
            f"{AGENT_URL}/v1/chat/completions",
            headers=headers,
            json={"messages": [{"role": "user", "content": query}]},
        )
        r.raise_for_status()
        return r.json()


# ---------------------------------------------------------------------------
# Display helpers
# ---------------------------------------------------------------------------


def _short(model: str) -> str:
    return model.split("/")[-1] if "/" in model else model


def _print_trace(result: dict) -> None:
    trace = result.get("routing_trace", [])
    if not trace:
        print("    (no trace)")
        return

    prev: str | None = None
    for t in trace:
        short = _short(t["model"])
        switch = "  ← switched" if (prev and t["model"] != prev) else ""
        prev = t["model"]
        print(f"    {t['task']:<26}  [{short}]{switch}")


def _print_summary(label: str, result: dict) -> None:
    models = [t["model"] for t in result.get("routing_trace", [])]
    if not models:
        print(f"  ?  {label}: no routing data")
        return
    unique = set(models)
    if len(unique) == 1:
        print(f"  ✓  {label}: {_short(next(iter(unique)))} for all {len(models)} turns")
    else:
        switched = sum(1 for a, b in zip(models, models[1:]) if a != b)
        names = ", ".join(sorted(_short(m) for m in unique))
        print(f"  ✗  {label}: model switched {switched} time(s) — {names}")


# ---------------------------------------------------------------------------
# Demo
# ---------------------------------------------------------------------------


async def main() -> None:
    print()
    print("  ╔══════════════════════════════════════════════════════════════╗")
    print("  ║      Session Pinning Demo — Research Agent                   ║")
    print("  ╚══════════════════════════════════════════════════════════════╝")
    print()
    print(f"  Agent : {AGENT_URL}")
    print(f"  Query : \"{QUERY[:72]}…\"")
    print()
    print("  The agent uses a tool-calling loop (get_db_benchmarks,")
    print("  get_case_studies, check_feature_support) to research the")
    print("  question. Each LLM turn hits Plano's preference-based router.")
    print()

    print(f"  Waiting for agent at {AGENT_URL}…", end=" ", flush=True)
    if not await wait_for_agent():
        print("FAILED — agent did not respond within 30 s")
        return
    print("ready.")
    print()

    sid = str(uuid.uuid4())
    print("  Sending queries (running concurrently)…")
    print()
    without, with_pin = await asyncio.gather(
        ask_agent(QUERY, session_id=None),
        ask_agent(QUERY, session_id=sid),
    )

    # ── Run 1 ────────────────────────────────────────────────────────────
    print("  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    print("  Run 1: WITHOUT Session Pinning")
    print("  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    print()
    print("  LLM turns inside the agent loop:")
    print()
    _print_trace(without)
    print()
    _print_summary("Without pinning", without)
    print()

    # ── Run 2 ────────────────────────────────────────────────────────────
    print("  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    print(f"  Run 2: WITH Session Pinning  (X-Session-Id: {sid[:8]}…)")
    print("  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    print()
    print("  LLM turns inside the agent loop:")
    print()
    _print_trace(with_pin)
    print()
    _print_summary("With pinning   ", with_pin)
    print()

    # ── Final answer ─────────────────────────────────────────────────────
    answer = with_pin["choices"][0]["message"]["content"]
    print("  ══ Agent recommendation (pinned session) ═════════════════════")
    print()
    for line in answer.splitlines():
        print(f"  {line}")
    print()
    print("  ══════════════════════════════════════════════════════════════")
    print()


if __name__ == "__main__":
    asyncio.run(main())
