"""AG2 Research Agent — Multi-agent team exposed as a single Plano endpoint.

This demonstrates AG2's unique capability: a GroupChat of multiple specialized
agents (researcher + analyst) running behind a single HTTP endpoint. Plano
routes requests to this endpoint, while AG2 handles internal multi-agent
orchestration.

AG2 (formerly AutoGen) is a community-maintained framework with 400K+ monthly
PyPI downloads. Learn more: https://ag2.ai
"""

import logging
import os
import sys
import time
import uuid
from contextlib import asynccontextmanager

import uvicorn
from autogen import AssistantAgent, GroupChat, GroupChatManager, LLMConfig
from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse, StreamingResponse

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from openai_protocol import create_chat_completion_chunk

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - [AG2_RESEARCH_AGENT] - %(levelname)s - %(message)s",
)
logger = logging.getLogger(__name__)

AGENT_PORT = 10530
LLM_GATEWAY = os.environ.get("LLM_GATEWAY_ENDPOINT", "http://localhost:12000/v1")
MODEL = "gpt-4o"

# AG2 LLM configuration — routes through Plano's LLM gateway.
# LLMConfig takes *configs (dicts or entry objects); api_key is required by
# the OpenAI client even when Plano proxies the real key.
llm_config = LLMConfig(
    {
        "api_type": "openai",
        "model": MODEL,
        "base_url": LLM_GATEWAY,
        "api_key": os.environ.get("OPENAI_API_KEY", "plano-proxied"),
    }
)


async def run_ag2_research_team(query: str) -> str:
    """Run an AG2 multi-agent research team on the given query.

    Creates a GroupChat with two specialized agents:
    - Researcher: gathers information and provides detailed findings
    - Analyst: synthesizes research into actionable insights

    Args:
        query: The user's research question

    Returns:
        The final analysis as a string
    """

    # Termination condition: stop as soon as analyst replies with TERMINATE.
    # Applied to both agents so the GroupChat halts immediately after the
    # analyst's first response, preventing a second researcher round-trip
    # that would cause an extra LLM call through Plano's gateway.
    def is_done(msg: dict) -> bool:
        return "TERMINATE" in msg.get("content", "")

    # Agent 1: Researcher — gathers and presents information
    researcher = AssistantAgent(
        name="researcher",
        llm_config=llm_config,
        system_message=(
            "You are a research specialist. When given a topic, provide "
            "detailed, factual information with specific data points. "
            "Be thorough but concise. Present your findings clearly."
        ),
        is_termination_msg=is_done,
    )

    # Agent 2: Analyst — synthesizes research into insights.
    analyst = AssistantAgent(
        name="analyst",
        llm_config=llm_config,
        system_message=(
            "You are an analyst. Review the researcher's findings and create "
            "a concise, actionable summary with key insights and recommendations. "
            "End your response with TERMINATE."
        ),
        is_termination_msg=is_done,
    )

    # AG2 GroupChat — internal multi-agent orchestration.
    # max_round=2: researcher answers once, analyst synthesizes once, done.
    # round_robin ensures deterministic researcher → analyst order.
    group_chat = GroupChat(
        agents=[researcher, analyst],
        messages=[],
        max_round=2,
        speaker_selection_method="round_robin",
    )

    manager = GroupChatManager(
        groupchat=group_chat,
        llm_config=llm_config,
    )

    # Run the multi-agent conversation
    result = await researcher.a_initiate_chat(
        manager,
        message=query,
    )

    # Extract the analyst's final message (contains TERMINATE when done).
    # Prefer the last message from "analyst"; fall back to any non-empty message.
    if result and hasattr(result, "chat_history") and result.chat_history:
        for msg in reversed(result.chat_history):
            if msg.get("name") == "analyst":
                content = msg.get("content", "").replace("TERMINATE", "").strip()
                if content:
                    return content
        # fallback: last non-empty, non-TERMINATE message from any agent
        for msg in reversed(result.chat_history):
            content = msg.get("content", "").replace("TERMINATE", "").strip()
            if content:
                return content

    return "No analysis generated."


@asynccontextmanager
async def lifespan(app: FastAPI):
    logger.info("AG2 Research Agent starting on port %d", AGENT_PORT)
    logger.info("LLM gateway: %s", LLM_GATEWAY)
    yield
    logger.info("AG2 Research Agent shutting down")


app = FastAPI(title="AG2 Research Agent", version="1.0.0", lifespan=lifespan)


@app.post("/v1/chat/completions")
async def handle_request(request: Request):
    request_body = await request.json()
    is_streaming = request_body.get("stream", True)
    model = request_body.get("model", MODEL)
    messages = request_body.get("messages", [])

    # Extract the user's query from the last user message
    user_query = "Hello"
    for msg in reversed(messages):
        if msg.get("role") == "user":
            user_query = msg.get("content", "Hello")
            break

    logger.info("Received query: %s", user_query[:100])

    try:
        if is_streaming:
            return StreamingResponse(
                invoke_research_team_stream(user_query, model),
                media_type="text/event-stream",
                headers={"content-type": "text/event-stream"},
            )

        content = await run_ag2_research_team(user_query)
        return JSONResponse(
            {
                "id": f"chatcmpl-{uuid.uuid4().hex[:8]}",
                "object": "chat.completion",
                "created": int(time.time()),
                "model": model,
                "choices": [
                    {
                        "index": 0,
                        "message": {"role": "assistant", "content": content},
                        "finish_reason": "stop",
                    }
                ],
            }
        )
    except Exception as e:
        logger.error("Error generating research response: %s", e)
        if is_streaming:
            return StreamingResponse(
                invoke_research_team_error_stream(model, str(e)),
                media_type="text/event-stream",
                headers={"content-type": "text/event-stream"},
            )
        return JSONResponse(
            {
                "error": {
                    "message": f"Research team error: {e}",
                    "type": "server_error",
                }
            },
            status_code=500,
        )


async def invoke_research_team_stream(user_query: str, model: str):
    try:
        result = await run_ag2_research_team(user_query)
        logger.info("AG2 team result length: %d chars", len(result))

        # Stream the result in chunks
        chunk_size = 50
        for i in range(0, len(result), chunk_size):
            chunk_text = result[i : i + chunk_size]
            yield f"data: {create_chat_completion_chunk(model, chunk_text).model_dump_json()}\n\n"

        yield f"data: {create_chat_completion_chunk(model, '', 'stop').model_dump_json()}\n\n"
        yield "data: [DONE]\n\n"
    except Exception as e:
        logger.error("Error streaming research response: %s", e)
        error_message = f"Research team error: {e}"
        yield f"data: {create_chat_completion_chunk(model, error_message, 'stop').model_dump_json()}\n\n"
        yield "data: [DONE]\n\n"


async def invoke_research_team_error_stream(model: str, error_message: str):
    yield f"data: {create_chat_completion_chunk(model, f'Error: {error_message}', 'stop').model_dump_json()}\n\n"
    yield "data: [DONE]\n\n"


@app.get("/health")
async def health_check():
    return {"status": "healthy", "agent": "ag2_research_team"}


if __name__ == "__main__":
    uvicorn.run(
        app,
        host="0.0.0.0",
        port=AGENT_PORT,
        log_config={
            "version": 1,
            "disable_existing_loggers": False,
            "formatters": {
                "default": {
                    "format": "%(asctime)s - [AG2_RESEARCH_AGENT] - %(levelname)s - %(message)s",
                }
            },
            "handlers": {
                "default": {
                    "formatter": "default",
                    "class": "logging.StreamHandler",
                    "stream": "ext://sys.stdout",
                }
            },
            "root": {"level": "INFO", "handlers": ["default"]},
        },
    )
