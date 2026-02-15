import asyncio
import json
import time
from typing import List, Optional, Dict, Any
import uuid
from fastapi import FastAPI, Depends, Request
from openai import AsyncOpenAI
import os
import logging

from .api import ChatCompletionRequest, ChatCompletionResponse, ChatMessage

# from . import mcp
# from fastmcp.server.dependencies import get_http_headers

# Set up logging
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - [QUERY_REWRITER]     - %(levelname)s - %(message)s",
)
logger = logging.getLogger(__name__)

# Configuration for Plano LLM gateway
LLM_GATEWAY_ENDPOINT = os.getenv("LLM_GATEWAY_ENDPOINT", "http://localhost:12000/v1")
QUERY_REWRITE_MODEL = "gpt-4o-mini"

# Initialize OpenAI client for Plano
plano_client = AsyncOpenAI(
    base_url=LLM_GATEWAY_ENDPOINT,
    api_key="EMPTY",  # Plano doesn't require a real API key
)

app = FastAPI(title="RAG Agent Query Rewriter", version="1.0.0")


async def rewrite_query_with_plano(
    messages: List[ChatMessage],
    traceparent_header: Optional[str] = None,
    request_id: Optional[str] = None,
) -> str:
    """Rewrite the last user message for better retrieval. Returns the rewritten query."""
    system_prompt = """You are a query rewriter that improves user queries for better retrieval.

Given a conversation history, rewrite the last user message to be more specific and context-aware.
The rewritten query should:
1. Include relevant context from previous messages
2. Be clear and specific for information retrieval
3. Maintain the user's intent
4. Be concise but comprehensive

Return only the rewritten query, nothing else."""

    rewrite_messages = [{"role": "system", "content": system_prompt}]
    for msg in messages:
        rewrite_messages.append({"role": msg.role, "content": msg.content})

    extra_headers = {"x-envoy-max-retries": "3", "x-request-id": request_id}
    if traceparent_header:
        extra_headers["traceparent"] = traceparent_header

    try:
        logger.info(f"Calling Plano at {LLM_GATEWAY_ENDPOINT} to rewrite query")
        resp = await plano_client.chat.completions.create(
            model=QUERY_REWRITE_MODEL,
            messages=rewrite_messages,
            temperature=0.3,
            max_tokens=200,
            extra_headers=extra_headers,
        )
        rewritten = resp.choices[0].message.content.strip()
        logger.info(f"Query rewritten successfully: '{rewritten}'")
        return rewritten
    except Exception as e:
        logger.error(f"Error rewriting query: {e}")

    # Fallback: return the original last user message
    for m in reversed(messages):
        if m.role == "user":
            logger.info("Falling back to original user message")
            return m.content
    return ""


@app.post("/")
async def query_rewriter_http(
    messages: List[ChatMessage], request: Request
) -> List[ChatMessage]:
    """HTTP filter endpoint used by Plano (type: http)."""
    logger.info(f"Received request with {len(messages)} messages")

    traceparent_header = request.headers.get("traceparent")
    request_id = request.headers.get("x-request-id")

    if traceparent_header:
        logger.info(f"Received traceparent header: {traceparent_header}")
    else:
        logger.info("No traceparent header found")

    rewritten_query = await rewrite_query_with_plano(
        messages, traceparent_header, request_id
    )
    # Create updated messages with the rewritten query
    updated_messages = messages.copy()

    # Find and update the last user message with the rewritten query
    for i in range(len(updated_messages) - 1, -1, -1):
        if updated_messages[i].role == "user":
            original_query = updated_messages[i].content
            updated_messages[i] = ChatMessage(role="user", content=rewritten_query)
            logger.info(
                f"Updated user query from '{original_query}' to '{rewritten_query}'"
            )
            break
    updated_messages_data = [
        {"role": msg.role, "content": msg.content} for msg in updated_messages
    ]
    updated_messages = [ChatMessage(**msg) for msg in updated_messages_data]

    logger.info("Returning rewritten chat completion response")
    return updated_messages


@app.get("/health")
async def health():
    return {"status": "healthy"}


def start_server(host: str = "0.0.0.0", port: int = 10501):
    """Start the FastAPI server for query rewriter."""
    import uvicorn

    logger.info(f"Starting Query Rewriter REST server on {host}:{port}")
    uvicorn.run(app, host=host, port=port)
