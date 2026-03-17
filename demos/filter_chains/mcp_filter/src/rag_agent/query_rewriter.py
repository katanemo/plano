import asyncio
import json
import time
from typing import List, Optional, Dict, Any
import uuid
from openai import AsyncOpenAI
import os
import logging

from .api import ChatMessage
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


async def query_rewriter(body: dict, path: str) -> dict:
    """Rewrites the last user query in the request body using Plano.

    Receives the full request body dict and the API path hint (e.g. /v1/chat/completions).
    Returns the body with the last user message rewritten for better retrieval.
    """
    messages = [ChatMessage(**m) for m in body.get("messages", [])]
    logger.info(f"Received request with {len(messages)} messages at path {path}")

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

    # Find and update the last user message with the rewritten query
    updated_messages = [m.model_dump() for m in messages]
    for i in range(len(updated_messages) - 1, -1, -1):
        if updated_messages[i]["role"] == "user":
            logger.info(
                f"Updated user query from '{updated_messages[i]['content']}' to '{rewritten_query}'"
            )
            updated_messages[i]["content"] = rewritten_query
            break

    logger.info("Returning rewritten chat completion response")
    return {**body, "messages": updated_messages}


# Register MCP tool only if mcp is available
if mcp is not None:
    mcp.tool()(query_rewriter)
