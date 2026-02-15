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
from . import mcp
from fastmcp.server.dependencies import get_http_headers

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

app = FastAPI()


async def rewrite_query_with_plano(
    messages: List[ChatMessage],
    traceparent_header: str,
    request_id: Optional[str] = None,
) -> str:
    """Rewrite the user query using LLM for better retrieval."""
    system_prompt = """You are a query rewriter that improves user queries for better retrieval.

    Given a conversation history, rewrite the last user message to be more specific and context-aware.
    The rewritten query should:
    1. Include relevant context from previous messages
    2. Be clear and specific for information retrieval
    3. Maintain the user's intent
    4. Be concise but comprehensive

    Return only the rewritten query, nothing else."""

    # Prepare messages for the query rewriter - just add system prompt to existing messages
    rewrite_messages = [{"role": "system", "content": system_prompt}]

    # Add conversation history
    for msg in messages:
        rewrite_messages.append({"role": msg.role, "content": msg.content})

    try:
        # Call Plano using OpenAI client
        extra_headers = {"x-envoy-max-retries": "3"}
        if traceparent_header:
            extra_headers["traceparent"] = traceparent_header
        if request_id:
            extra_headers["x-request-id"] = request_id
        logger.info(f"Calling Plano at {LLM_GATEWAY_ENDPOINT} to rewrite query")
        response = await plano_client.chat.completions.create(
            model=QUERY_REWRITE_MODEL,
            messages=rewrite_messages,
            temperature=0.3,
            max_tokens=200,
            extra_headers=extra_headers,
        )

        rewritten_query = response.choices[0].message.content.strip()
        logger.info(f"Query rewritten successfully: '{rewritten_query}'")
        return rewritten_query

    except Exception as e:
        logger.error(f"Error rewriting query: {e}")

    # If rewriting fails, return the original last user message
    logger.info("Falling back to original user message")
    for message in reversed(messages):
        if message.role == "user":
            return message.content
    return ""


async def query_rewriter(messages: List[ChatMessage]) -> List[ChatMessage]:
    """Chat completions endpoint that rewrites the last user query using Plano.

    Returns a dict with a 'messages' key containing the updated message list.
    """
    logger.info(f"Received chat completion request with {len(messages)} messages")

    # Get traceparent header from HTTP request using FastMCP's dependency function
    headers = get_http_headers()
    traceparent_header = headers.get("traceparent")
    request_id = headers.get("x-request-id")

    if traceparent_header:
        logger.info(f"Received traceparent header: {traceparent_header}")
    else:
        logger.info("No traceparent header found")

    # Call Plano to rewrite the last user query
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

    # Return as dict to minimize text serialization
    return [{"role": msg.role, "content": msg.content} for msg in updated_messages]


# Register MCP tool only if mcp is available
if mcp is not None:
    mcp.tool()(query_rewriter)


@app.post("/")
async def chat_completions_endpoint(
    request_messages: List[ChatMessage], request: Request
) -> List[ChatMessage]:
    """FastAPI endpoint for chat completions with query rewriting."""
    logger.info(
        f"Received /v1/chat/completions request with {len(request_messages)} messages"
    )

    # Extract traceparent header
    traceparent_header = request.headers.get("traceparent")
    if traceparent_header:
        logger.info(f"Received traceparent header: {traceparent_header}")
    else:
        logger.info("No traceparent header found")

    # Call the query rewriter tool
    updated_messages_data = await query_rewriter(request_messages)

    # Convert back to ChatMessage objects
    updated_messages = [ChatMessage(**msg) for msg in updated_messages_data]

    logger.info("Returning rewritten chat completion response")
    return updated_messages


def start_server(host: str = "0.0.0.0", port: int = 10501):
    """Start the FastAPI server for query rewriter."""
    import uvicorn

    logger.info(f"Starting Query Rewriter REST server on {host}:{port}")
    uvicorn.run(app, host=host, port=port)
