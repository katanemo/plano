import json
from typing import List, Optional, Dict, Any
from openai import AsyncOpenAI
import os
import logging
import csv
from pathlib import Path

from .api import ChatMessage
from . import mcp
from fastmcp.server.dependencies import get_http_headers

# Set up logging
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - [CONTEXT_BUILDER]    - %(levelname)s - %(message)s",
)
logger = logging.getLogger(__name__)


# Configuration for archgw LLM gateway
LLM_GATEWAY_ENDPOINT = os.getenv("LLM_GATEWAY_ENDPOINT", "http://localhost:12000/v1")
RAG_MODEL = "gpt-4o-mini"

# Initialize OpenAI client for archgw
archgw_client = AsyncOpenAI(
    base_url=LLM_GATEWAY_ENDPOINT,
    api_key="EMPTY",  # archgw doesn't require a real API key
)

# Global variable to store the knowledge base
knowledge_base = []


def load_knowledge_base():
    """Load the sample_knowledge_base.csv file into memory on startup."""
    global knowledge_base

    # Get the path to the CSV file relative to this script
    current_dir = Path(__file__).parent
    csv_path = current_dir / "sample_knowledge_base.csv"

    print(f"Loading knowledge base from {csv_path}")

    try:
        knowledge_base = []
        with open(csv_path, "r", encoding="utf-8-sig") as file:
            csv_reader = csv.DictReader(file)
            for row in csv_reader:
                knowledge_base.append({"path": row["path"], "content": row["content"]})

        logger.info(f"Loaded {len(knowledge_base)} documents from knowledge base")

    except Exception as e:
        logger.error(f"Error loading knowledge base: {e}")
        knowledge_base = []


async def find_relevant_passages(
    query: str,
    traceparent: Optional[str] = None,
    request_id: Optional[str] = None,
    top_k: int = 3,
) -> List[Dict[str, str]]:
    """Use the LLM to find the most relevant passages from the knowledge base."""

    if not knowledge_base:
        logger.warning("Knowledge base is empty")
        return []

    # Create a system prompt for passage selection
    system_prompt = f"""You are a retrieval assistant that selects the most relevant document passages for a given query.

                    Given a user query and a list of document passages, identify the {top_k} most relevant passages that would help answer the query.

                    Query: {query}

                    Available passages:
                    """

    # Add all passages with indices
    for i, doc in enumerate(knowledge_base):
        system_prompt += (
            f"\n[{i}] Path: {doc['path']}\nContent: {doc['content'][:500]}...\n"
        )

    system_prompt += f"""

        Please respond with ONLY the indices of the {top_k} most relevant passages, separated by commas (e.g., "0,3,7").
        If fewer than {top_k} passages are relevant, return only the relevant ones.
        If no passages are relevant, return "NONE"."""

    try:
        # Call archgw to select relevant passages
        logger.info(f"Calling archgw to find relevant passages for query: '{query}'")

        # Prepare extra headers if traceparent is provided
        extra_headers = {
            "x-envoy-max-retries": "3",
        }
        if request_id:
            extra_headers["x-request-id"] = request_id
        if traceparent:
            extra_headers["traceparent"] = traceparent

        response = await archgw_client.chat.completions.create(
            model=RAG_MODEL,
            messages=[{"role": "system", "content": system_prompt}],
            temperature=0.1,
            max_tokens=50,
            extra_headers=extra_headers,
        )

        result = response.choices[0].message.content.strip()
        logger.info(f"LLM selected passages: {result}")

        # Parse the indices
        if result.upper() == "NONE":
            return []

        selected_passages = []
        indices = [
            int(idx.strip()) for idx in result.split(",") if idx.strip().isdigit()
        ]

        for idx in indices:
            if 0 <= idx < len(knowledge_base):
                selected_passages.append(knowledge_base[idx])

        logger.info(f"Selected {len(selected_passages)} relevant passages")
        return selected_passages

    except Exception as e:
        logger.error(f"Error finding relevant passages: {e}")
        return []


async def augment_query_with_context(
    messages: List[ChatMessage],
    traceparent: Optional[str] = None,
    request_id: Optional[str] = None,
) -> List[ChatMessage]:
    """Extract user query, find relevant context, and augment the messages."""

    # Find the last user message
    last_user_message = None
    last_user_index = -1

    for i in range(len(messages) - 1, -1, -1):
        if messages[i].role == "user":
            last_user_message = messages[i].content
            last_user_index = i
            break

    if not last_user_message:
        logger.warning("No user message found in conversation")
        return messages

    logger.info(f"Processing user query: '{last_user_message}'")

    # Find relevant passages
    relevant_passages = await find_relevant_passages(
        last_user_message, traceparent, request_id
    )

    if not relevant_passages:
        logger.info("No relevant passages found, returning original messages")
        return messages

    # Build context from relevant passages
    context_parts = []
    for i, passage in enumerate(relevant_passages):
        context_parts.append(
            f"Document {i+1} ({passage['path']}):\n{passage['content']}"
        )

    context = "\n\n".join(context_parts)

    # Create augmented content with original query and context
    augmented_content = f"""{last_user_message} RELEVANT CONTEXT:
    {context}"""

    # Create updated messages with the augmented query
    updated_messages = messages.copy()
    updated_messages[last_user_index] = ChatMessage(
        role="user", content=augmented_content
    )

    logger.info(f"Augmented user query with {len(relevant_passages)} relevant passages")

    return updated_messages


# Load knowledge base on module import
load_knowledge_base()


async def context_builder(messages: List[ChatMessage]) -> List[ChatMessage]:
    """MCP tool that augments user queries with relevant context from the knowledge base."""
    logger.info(f"Received chat completion request with {len(messages)} messages")

    # Get traceparent header from MCP request
    headers = get_http_headers()
    traceparent_header = headers.get("traceparent")
    request_id = headers.get("x-request-id")
    logger.info(f"Received request ID: {request_id}")

    if traceparent_header:
        logger.info(f"Received traceparent header: {traceparent_header}")
    else:
        logger.info("No traceparent header found")

    # Augment the user query with relevant context
    updated_messages = await augment_query_with_context(
        messages, traceparent_header, request_id
    )

    # Return as dict to minimize text serialization
    return [{"role": msg.role, "content": msg.content} for msg in updated_messages]


# Register MCP tool only if mcp is available
if mcp is not None:
    mcp.tool()(context_builder)
