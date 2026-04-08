#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["openai>=1.0.0"]
# ///
"""
Model Affinity Demo — Agentic Tool-Calling Loop

Runs the same agentic loop twice through Plano:
  1. Without model affinity — the router may pick different models per turn
  2. With model affinity  — all turns use the model selected on turn 1

Each loop is a real tool-calling agent: the LLM decides which tools to call,
we provide simulated results, and the LLM continues until it has enough
information to produce a final answer. Each turn is a separate request to
Plano, so the router classifies intent independently every time.

Usage:
    planoai up config.yaml          # start Plano
    uv run demo.py                  # run this demo
"""

import asyncio
import json
import os
import uuid

from openai import AsyncOpenAI
from openai.types.chat import ChatCompletionMessageParam

PLANO_URL = os.environ.get("PLANO_URL", "http://localhost:12000")

SYSTEM_PROMPT = (
    "You are a database selection analyst. Use the provided tools to gather "
    "benchmark data and case studies, then recommend PostgreSQL or MongoDB "
    "for a high-traffic e-commerce backend. Be concise."
)

USER_QUERY = (
    "Should we use PostgreSQL or MongoDB for our e-commerce platform? "
    "We need strong consistency for orders but flexible schemas for products. "
    "Use the tools to research both options, then give a recommendation."
)

TOOLS = [
    {
        "type": "function",
        "function": {
            "name": "get_db_benchmarks",
            "description": "Fetch performance benchmarks for a database under a given workload.",
            "parameters": {
                "type": "object",
                "properties": {
                    "database": {
                        "type": "string",
                        "enum": ["postgresql", "mongodb"],
                    },
                    "workload": {
                        "type": "string",
                        "enum": ["read_heavy", "write_heavy", "mixed"],
                    },
                },
                "required": ["database", "workload"],
            },
        },
    },
    {
        "type": "function",
        "function": {
            "name": "get_case_studies",
            "description": "Retrieve real-world e-commerce case studies for a database.",
            "parameters": {
                "type": "object",
                "properties": {
                    "database": {
                        "type": "string",
                        "enum": ["postgresql", "mongodb"],
                    },
                },
                "required": ["database"],
            },
        },
    },
    {
        "type": "function",
        "function": {
            "name": "check_feature_support",
            "description": "Check if a database supports a specific feature.",
            "parameters": {
                "type": "object",
                "properties": {
                    "database": {
                        "type": "string",
                        "enum": ["postgresql", "mongodb"],
                    },
                    "feature": {"type": "string"},
                },
                "required": ["database", "feature"],
            },
        },
    },
]

# Simulated tool responses
_BENCHMARKS = {
    ("postgresql", "mixed"): {
        "read_qps": 42000,
        "write_qps": 21000,
        "p99_ms": 6,
        "notes": "Solid all-round; MVCC keeps reads non-blocking",
    },
    ("mongodb", "mixed"): {
        "read_qps": 60000,
        "write_qps": 50000,
        "p99_ms": 3,
        "notes": "Flexible schema accelerates feature iteration",
    },
}

_CASE_STUDIES = {
    "postgresql": [
        {"company": "Shopify", "notes": "Moved orders back to Postgres for ACID"},
        {
            "company": "Zalando",
            "notes": "Postgres + Citus for sharded order processing",
        },
    ],
    "mongodb": [
        {"company": "eBay", "notes": "Product catalogue — flexible attribute schemas"},
        {"company": "Alibaba", "notes": "Session/cart data — high write throughput"},
    ],
}

_FEATURES = {
    ("postgresql", "acid transactions"): {"supported": True, "notes": "Full ACID"},
    ("mongodb", "acid transactions"): {
        "supported": True,
        "notes": "Multi-doc ACID since v4.0",
    },
    ("postgresql", "horizontal sharding"): {
        "supported": True,
        "notes": "Via Citus extension",
    },
    ("mongodb", "horizontal sharding"): {
        "supported": True,
        "notes": "Native auto-balancing",
    },
}


def dispatch_tool(name: str, args: dict) -> str:
    if name == "get_db_benchmarks":
        key = (args["database"], args["workload"])
        return json.dumps(_BENCHMARKS.get(key, {"error": f"no data for {key}"}))
    if name == "get_case_studies":
        return json.dumps(_CASE_STUDIES.get(args["database"], {"error": "unknown db"}))
    if name == "check_feature_support":
        key = (args["database"], args["feature"].lower())
        for k, v in _FEATURES.items():
            if k[0] == key[0] and k[1] in key[1]:
                return json.dumps(v)
        return json.dumps({"error": f"no data for {key}"})
    return json.dumps({"error": f"unknown tool {name}"})


# ---------------------------------------------------------------------------
# Agentic loop — runs tool calls until the LLM produces a final answer
# ---------------------------------------------------------------------------


async def run_agent_loop(
    affinity_id: str | None = None,
    max_turns: int = 10,
) -> tuple[str, list[dict]]:
    """
    Run a tool-calling agent loop against Plano.

    Returns (final_answer, trace) where trace is a list of
    {"turn": int, "model": str, "tool_calls": [...]} dicts.
    """
    client = AsyncOpenAI(base_url=f"{PLANO_URL}/v1", api_key="EMPTY")
    headers = {"X-Model-Affinity": affinity_id} if affinity_id else None

    messages: list[ChatCompletionMessageParam] = [
        {"role": "system", "content": SYSTEM_PROMPT},
        {"role": "user", "content": USER_QUERY},
    ]
    trace: list[dict] = []

    for turn in range(1, max_turns + 1):
        resp = await client.chat.completions.create(
            model="gpt-4o-mini",
            messages=messages,
            tools=TOOLS,
            tool_choice="auto",
            max_completion_tokens=800,
            extra_headers=headers,
        )

        choice = resp.choices[0]
        turn_info: dict = {"turn": turn, "model": resp.model}

        if choice.finish_reason == "tool_calls" and choice.message.tool_calls:
            tool_names = [tc.function.name for tc in choice.message.tool_calls]
            turn_info["tool_calls"] = tool_names
            trace.append(turn_info)

            messages.append(choice.message)
            for tc in choice.message.tool_calls:
                args = json.loads(tc.function.arguments or "{}")
                result = dispatch_tool(tc.function.name, args)
                messages.append(
                    {"role": "tool", "content": result, "tool_call_id": tc.id}
                )
        else:
            turn_info["tool_calls"] = []
            trace.append(turn_info)
            return (choice.message.content or "").strip(), trace

    return "(max turns reached)", trace


# ---------------------------------------------------------------------------
# Display helpers
# ---------------------------------------------------------------------------


def short_model(model: str) -> str:
    return model.split("/")[-1] if "/" in model else model


def print_trace(trace: list[dict]) -> None:
    for t in trace:
        model = short_model(t["model"])
        tools = ", ".join(t["tool_calls"]) if t["tool_calls"] else "final answer"
        print(f"    turn {t['turn']}  [{model:<30}]  {tools}")


def print_summary(label: str, trace: list[dict]) -> None:
    models = [t["model"] for t in trace]
    unique = set(models)
    if len(unique) == 1:
        print(
            f"  ✓  {label}: {short_model(next(iter(unique)))} "
            f"for all {len(models)} turns"
        )
    else:
        switches = sum(1 for a, b in zip(models, models[1:]) if a != b)
        names = ", ".join(sorted(short_model(m) for m in unique))
        print(f"  ✗  {label}: model switched {switches} time(s) — {names}")


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


async def main() -> None:
    print()
    print("  ╔══════════════════════════════════════════════════════════╗")
    print("  ║          Model Affinity Demo — Agentic Loop             ║")
    print("  ╚══════════════════════════════════════════════════════════╝")
    print()
    print(f"  Plano : {PLANO_URL}")
    print(f'  Query : "{USER_QUERY[:65]}…"')
    print()
    print("  The agent calls tools (get_db_benchmarks, get_case_studies,")
    print("  check_feature_support) across multiple turns. Each turn is")
    print("  a separate request to Plano — the router classifies intent")
    print("  independently, so different turns may get different models.")
    print()

    # --- Run 1: without affinity ---
    print("  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    print("  Run 1: WITHOUT Model Affinity")
    print("  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    print()
    answer1, trace1 = await run_agent_loop(affinity_id=None)
    print_trace(trace1)
    print()
    print_summary("Without affinity", trace1)
    print()

    # --- Run 2: with affinity ---
    aid = str(uuid.uuid4())
    print("  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    print(f"  Run 2: WITH Model Affinity  (X-Model-Affinity: {aid[:8]}…)")
    print("  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    print()
    answer2, trace2 = await run_agent_loop(affinity_id=aid)
    print_trace(trace2)
    print()
    print_summary("With affinity   ", trace2)
    print()

    # --- Final answer ---
    print("  ══ Agent recommendation (affinity session) ════════════════")
    print()
    for line in answer2.splitlines():
        print(f"  {line}")
    print()
    print("  ═══════════════════════════════════════════════════════════")
    print()


if __name__ == "__main__":
    asyncio.run(main())
