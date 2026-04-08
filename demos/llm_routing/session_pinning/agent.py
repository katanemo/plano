#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["fastapi>=0.115", "uvicorn>=0.30", "openai>=1.0.0"]
# ///
"""
Research Agent — FastAPI service exposing /v1/chat/completions.

For each incoming request the agent runs 3 independent research tasks,
each with its own tool-calling loop. The tasks deliberately alternate between
code_generation and complex_reasoning intents so Plano's preference-based
router selects different models for each task.

If the client sends X-Routing-Session-Id, the agent forwards it on every
outbound call to Plano. The first task pins the model; all subsequent tasks
skip the router and reuse it — keeping the whole session on one consistent
model.

Run standalone:
    uv run agent.py
    PLANO_URL=http://myhost:12000 AGENT_PORT=8000 uv run agent.py
"""

import json
import logging
import os
import uuid

import uvicorn
from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse
from openai import AsyncOpenAI
from openai.types.chat import ChatCompletionMessageParam

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [AGENT] %(levelname)s %(message)s",
)
log = logging.getLogger(__name__)

PLANO_URL = os.environ.get("PLANO_URL", "http://localhost:12000")
PORT = int(os.environ.get("AGENT_PORT", "8000"))

# ---------------------------------------------------------------------------
# Tasks — each has its own conversation so Plano routes each independently.
# Intent alternates: code_generation → complex_reasoning → code_generation.
# ---------------------------------------------------------------------------

TASKS = [
    {
        "name": "generate_comparison",
        # Triggers code_generation routing preference (write/generate output)
        "prompt": (
            "Use the tools to fetch benchmark data for PostgreSQL and MongoDB "
            "under a mixed workload. Then generate a compact Markdown comparison "
            "table with columns: metric, PostgreSQL, MongoDB. Cover read QPS, "
            "write QPS, p99 latency ms, ACID support, and horizontal scaling."
        ),
    },
    {
        "name": "analyse_tradeoffs",
        # Triggers complex_reasoning routing preference (analyse/reason/evaluate)
        "prompt": (
            "Context from prior research:\n{context}\n\n"
            "Perform a deep analysis: for a high-traffic e-commerce platform that "
            "requires ACID guarantees for order processing but flexible schemas for "
            "product attributes, carefully reason through and evaluate the long-term "
            "architectural trade-offs of each database. Consider consistency "
            "guarantees, operational complexity, and scalability risks."
        ),
    },
    {
        "name": "write_schema",
        # Triggers code_generation routing preference (write SQL / generate code)
        "prompt": (
            "Context from prior research:\n{context}\n\n"
            "Write the CREATE TABLE SQL schema for the database you would recommend "
            "from the analysis above. Include: orders, order_items, products, and "
            "users tables with appropriate primary keys, foreign keys, and indexes."
        ),
    },
]

SYSTEM_PROMPT = (
    "You are a database selection analyst for an e-commerce platform. "
    "Use the available tools when you need data. "
    "Be concise — each response should be a compact table, code block, "
    "or 3–5 clear sentences."
)

# ---------------------------------------------------------------------------
# Tool definitions
# ---------------------------------------------------------------------------

TOOLS = [
    {
        "type": "function",
        "function": {
            "name": "get_db_benchmarks",
            "description": (
                "Fetch performance benchmark data for a database. "
                "Returns read/write throughput, latency, and scaling characteristics."
            ),
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
            "description": "Retrieve e-commerce case studies for a database.",
            "parameters": {
                "type": "object",
                "properties": {
                    "database": {"type": "string", "enum": ["postgresql", "mongodb"]},
                },
                "required": ["database"],
            },
        },
    },
    {
        "type": "function",
        "function": {
            "name": "check_feature_support",
            "description": (
                "Check whether a database supports a specific feature "
                "(e.g. ACID transactions, horizontal sharding, JSON documents)."
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "database": {"type": "string", "enum": ["postgresql", "mongodb"]},
                    "feature": {"type": "string"},
                },
                "required": ["database", "feature"],
            },
        },
    },
]

# ---------------------------------------------------------------------------
# Tool implementations (simulated — no external calls)
# ---------------------------------------------------------------------------

_BENCHMARKS = {
    ("postgresql", "read_heavy"): {
        "read_qps": 55_000,
        "write_qps": 18_000,
        "p99_ms": 4,
        "notes": "Excellent for complex joins; connection pooling via pgBouncer recommended",
    },
    ("postgresql", "write_heavy"): {
        "read_qps": 30_000,
        "write_qps": 24_000,
        "p99_ms": 8,
        "notes": "WAL overhead increases at very high write volume; partitioning helps",
    },
    ("postgresql", "mixed"): {
        "read_qps": 42_000,
        "write_qps": 21_000,
        "p99_ms": 6,
        "notes": "Solid all-round; MVCC keeps reads non-blocking",
    },
    ("mongodb", "read_heavy"): {
        "read_qps": 85_000,
        "write_qps": 30_000,
        "p99_ms": 2,
        "notes": "Atlas Search built-in; sharding distributes read load well",
    },
    ("mongodb", "write_heavy"): {
        "read_qps": 40_000,
        "write_qps": 65_000,
        "p99_ms": 3,
        "notes": "WiredTiger compression reduces I/O; journal writes are async-safe",
    },
    ("mongodb", "mixed"): {
        "read_qps": 60_000,
        "write_qps": 50_000,
        "p99_ms": 3,
        "notes": "Flexible schema accelerates feature iteration",
    },
}

_CASE_STUDIES = {
    "postgresql": [
        {
            "company": "Shopify",
            "scale": "100 B+ req/day",
            "notes": "Moved critical order tables back to Postgres for ACID guarantees",
        },
        {
            "company": "Zalando",
            "scale": "50 M customers",
            "notes": "Uses Postgres + Citus for sharded order processing",
        },
        {
            "company": "Instacart",
            "scale": "10 M orders/mo",
            "notes": "Postgres for inventory; strict consistency required for stock levels",
        },
    ],
    "mongodb": [
        {
            "company": "eBay",
            "scale": "1.5 B listings",
            "notes": "Product catalogue in MongoDB for flexible attribute schemas",
        },
        {
            "company": "Alibaba",
            "scale": "billions of docs",
            "notes": "Session and cart data in MongoDB; high write throughput",
        },
        {
            "company": "Foursquare",
            "scale": "10 B+ check-ins",
            "notes": "Geospatial queries and flexible location schemas",
        },
    ],
}

_FEATURES = {
    ("postgresql", "acid transactions"): {
        "supported": True,
        "notes": "Full ACID with serialisable isolation",
    },
    ("postgresql", "horizontal sharding"): {
        "supported": True,
        "notes": "Via Citus extension or manual partitioning; not native",
    },
    ("postgresql", "json documents"): {
        "supported": True,
        "notes": "JSONB with indexing; flexible but slower than native doc store",
    },
    ("postgresql", "full-text search"): {
        "supported": True,
        "notes": "Built-in tsvector/tsquery; Elasticsearch for advanced use cases",
    },
    ("postgresql", "multi-document transactions"): {
        "supported": True,
        "notes": "Native cross-table ACID",
    },
    ("mongodb", "acid transactions"): {
        "supported": True,
        "notes": "Multi-document ACID since v4.0; single-doc always atomic",
    },
    ("mongodb", "horizontal sharding"): {
        "supported": True,
        "notes": "Native sharding; auto-balancing across shards",
    },
    ("mongodb", "json documents"): {
        "supported": True,
        "notes": "Native BSON document model; schema-free by default",
    },
    ("mongodb", "full-text search"): {
        "supported": True,
        "notes": "Atlas Search (Lucene-based) for advanced full-text",
    },
    ("mongodb", "multi-document transactions"): {
        "supported": True,
        "notes": "Available but adds latency; best avoided on hot paths",
    },
}


def _dispatch(name: str, args: dict) -> str:
    if name == "get_db_benchmarks":
        key = (args["database"].lower(), args["workload"].lower())
        return json.dumps(_BENCHMARKS.get(key, {"error": f"no data for {key}"}))

    if name == "get_case_studies":
        db = args["database"].lower()
        return json.dumps(_CASE_STUDIES.get(db, {"error": f"unknown db '{db}'"}))

    if name == "check_feature_support":
        key = (args["database"].lower(), args["feature"].lower())
        for k, v in _FEATURES.items():
            if k[0] == key[0] and k[1] in key[1]:
                return json.dumps(v)
        return json.dumps({"error": f"feature '{args['feature']}' not in dataset"})

    return json.dumps({"error": f"unknown tool '{name}'"})


# ---------------------------------------------------------------------------
# Task runner — one independent conversation per task
# ---------------------------------------------------------------------------


async def run_task(
    client: AsyncOpenAI,
    task_name: str,
    prompt: str,
    session_id: str | None,
) -> tuple[str, str]:
    """
    Run a single research task with its own tool-calling loop.

    Each task is an independent conversation so the router sees only
    this task's intent — not the accumulated context of previous tasks.
    Session pinning via X-Routing-Session-Id pins the model from the first
    task onward, so all tasks stay on the same model.

    Returns (answer, first_model_used).
    """
    headers = {"X-Routing-Session-Id": session_id} if session_id else {}
    messages: list[ChatCompletionMessageParam] = [
        {"role": "system", "content": SYSTEM_PROMPT},
        {"role": "user", "content": prompt},
    ]
    first_model: str | None = None

    while True:
        resp = await client.chat.completions.create(
            model="gpt-4o-mini",  # Plano's router overrides this via routing_preferences
            messages=messages,
            tools=TOOLS,
            tool_choice="auto",
            max_completion_tokens=600,
            extra_headers=headers or None,
        )
        if first_model is None:
            first_model = resp.model

        log.info(
            "task=%s  model=%s  finish=%s",
            task_name,
            resp.model,
            resp.choices[0].finish_reason,
        )

        choice = resp.choices[0]
        if choice.finish_reason == "tool_calls" and choice.message.tool_calls:
            messages.append(choice.message)
            for tc in choice.message.tool_calls:
                args = json.loads(tc.function.arguments or "{}")
                result = _dispatch(tc.function.name, args)
                log.info("  tool %s(%s)", tc.function.name, args)
                messages.append(
                    {"role": "tool", "content": result, "tool_call_id": tc.id}
                )
        else:
            return (choice.message.content or "").strip(), first_model or "unknown"


# ---------------------------------------------------------------------------
# Research loop — runs all tasks, threading context forward
# ---------------------------------------------------------------------------


async def run_research_loop(
    client: AsyncOpenAI,
    session_id: str | None,
) -> tuple[str, list[dict]]:
    """
    Run all 3 research tasks in sequence, passing each task's output as
    context to the next. Returns (final_answer, routing_trace).
    """
    context = ""
    trace: list[dict] = []
    final_answer = ""

    for task in TASKS:
        prompt = task["prompt"].format(context=context)
        answer, model = await run_task(client, task["name"], prompt, session_id)
        trace.append({"task": task["name"], "model": model})
        context += f"\n### {task['name']}\n{answer}\n"
        final_answer = answer

    return final_answer, trace


# ---------------------------------------------------------------------------
# FastAPI app
# ---------------------------------------------------------------------------

app = FastAPI(title="Research Agent", version="1.0.0")


@app.post("/v1/chat/completions")
async def chat(request: Request) -> JSONResponse:
    body = await request.json()
    session_id: str | None = request.headers.get("x-routing-session-id")

    log.info("request  session_id=%s", session_id or "none")

    client = AsyncOpenAI(base_url=f"{PLANO_URL}/v1", api_key="EMPTY")
    answer, trace = await run_research_loop(client, session_id)

    return JSONResponse(
        {
            "id": f"chatcmpl-{uuid.uuid4().hex[:8]}",
            "object": "chat.completion",
            "choices": [
                {
                    "index": 0,
                    "message": {"role": "assistant", "content": answer},
                    "finish_reason": "stop",
                }
            ],
            "routing_trace": trace,
            "session_id": session_id,
        }
    )


@app.get("/health")
async def health() -> dict:
    return {"status": "ok", "plano_url": PLANO_URL}


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------

if __name__ == "__main__":
    log.info("starting on port %d  plano=%s", PORT, PLANO_URL)
    uvicorn.run(app, host="0.0.0.0", port=PORT, log_level="warning")
