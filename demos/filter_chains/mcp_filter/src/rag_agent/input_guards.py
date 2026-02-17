import asyncio
import json
import time
from typing import List, Optional, Dict, Any
import uuid
from fastapi import FastAPI, Depends, Request
from fastmcp.exceptions import ToolError
from openai import AsyncOpenAI
import os
import logging

from .api import ChatCompletionRequest, ChatCompletionResponse, ChatMessage
from . import mcp
from fastmcp.server.dependencies import get_http_headers

# Set up logging
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - [INPUT_GUARDS]       - %(levelname)s - %(message)s",
)
logger = logging.getLogger(__name__)

# Configuration for Plano LLM gateway
LLM_GATEWAY_ENDPOINT = os.getenv("LLM_GATEWAY_ENDPOINT", "http://localhost:12000/v1")
GUARD_MODEL = "gpt-4o-mini"

# Initialize OpenAI client for Plano
plano_client = AsyncOpenAI(
    base_url=LLM_GATEWAY_ENDPOINT,
    api_key="EMPTY",  # Plano doesn't require a real API key
)

app = FastAPI()


async def validate_query_scope(
    messages: List[ChatMessage],
    traceparent_header: str,
    request_id: Optional[str] = None,
) -> Dict[str, Any]:
    """Validate that the user query is within TechCorp's domain.

    Returns a dict with:
        - is_valid: bool indicating if query is within scope
        - reason: str explaining why query is out of scope (if applicable)
    """
    system_prompt = """You are an input validation guard for TechCorp's customer support system.

Your job is to determine if a user's query is related to TechCorp and its services/products.

TechCorp is a technology company that provides:
- Cloud services and infrastructure
- SaaS products
- Technical support
- Service level agreements (SLAs)
- Uptime guarantees
- Enterprise solutions

ALLOW queries about:
- TechCorp's services, products, or offerings
- TechCorp's pricing, SLAs, uptime, or policies
- Technical support for TechCorp products
- General questions about TechCorp as a company

REJECT queries about:
- Other companies or their products
- General knowledge questions unrelated to TechCorp
- Personal advice or topics outside TechCorp's domain
- Anything that doesn't relate to TechCorp's business

Respond in JSON format:
{
    "is_valid": true/false,
    "reason": "brief explanation if invalid"
}"""

    # Get the last user message for validation
    last_user_message = None
    for msg in reversed(messages):
        if msg.role == "user":
            last_user_message = msg.content
            break

    if not last_user_message:
        return {"is_valid": True, "reason": ""}

    # Prepare messages for the guard
    guard_messages = [
        {"role": "system", "content": system_prompt},
        {"role": "user", "content": f"Query to validate: {last_user_message}"},
    ]

    try:
        # Call Plano using OpenAI client
        extra_headers = {"x-envoy-max-retries": "3"}
        if traceparent_header:
            extra_headers["traceparent"] = traceparent_header

        if request_id:
            extra_headers["x-request-id"] = request_id

        logger.info(f"Validating query scope: '{last_user_message}'")
        response = await plano_client.chat.completions.create(
            model=GUARD_MODEL,
            messages=guard_messages,
            temperature=0.1,
            max_tokens=150,
            extra_headers=extra_headers,
        )

        result_text = response.choices[0].message.content.strip()

        # Parse JSON response
        try:
            result = json.loads(result_text)
            logger.info(f"Validation result: {result}")
            return result
        except json.JSONDecodeError:
            logger.error(f"Failed to parse validation response: {result_text}")
            # Default to allowing if parsing fails
            return {"is_valid": True, "reason": ""}

    except Exception as e:
        logger.error(f"Error validating query: {e}")
        # Default to allowing if validation fails
        return {"is_valid": True, "reason": ""}


@mcp.tool
async def input_guards(messages: List[ChatMessage]) -> List[ChatMessage]:
    """Input guard that validates queries are within TechCorp's domain.

    If the query is out of scope, replaces the user message with a rejection notice.
    """
    logger.info(f"Received request with {len(messages)} messages")

    # Get traceparent header from HTTP request using FastMCP's dependency function
    headers = get_http_headers()
    traceparent_header = headers.get("traceparent")
    request_id = headers.get("x-request-id")

    if traceparent_header:
        logger.info(f"Received traceparent header: {traceparent_header}")
    else:
        logger.info("No traceparent header found")

    # Validate the query scope
    validation_result = await validate_query_scope(
        messages, traceparent_header, request_id
    )

    if not validation_result.get("is_valid", True):
        reason = validation_result.get("reason", "Query is outside TechCorp's domain")
        logger.warning(f"Query rejected: {reason}")

        # Throw ToolError
        error_message = f"I apologize, but I can only assist with questions related to TechCorp and its services. Your query appears to be outside this scope. {reason}\n\nPlease ask me about TechCorp's products, services, pricing, SLAs, or technical support."
        raise ToolError(error_message)

    logger.info("Query validation passed - forwarding to next filter")
    return messages
