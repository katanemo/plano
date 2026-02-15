import json
from fastapi import FastAPI, Request
from fastapi.responses import StreamingResponse
from openai import AsyncOpenAI
import os
import logging
import time
import uuid
import uvicorn
import asyncio

from .api import (
    ChatCompletionRequest,
    ChatCompletionResponse,
    ChatCompletionStreamResponse,
)

# Set up logging
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - [RESPONSE_GENERATOR] - %(levelname)s - %(message)s",
)
logger = logging.getLogger(__name__)

# Configuration for Plano LLM gateway
LLM_GATEWAY_ENDPOINT = os.getenv("LLM_GATEWAY_ENDPOINT", "http://localhost:12000/v1")
RESPONSE_MODEL = "gpt-4o"

# System prompt for response generation
SYSTEM_PROMPT = """You are a helpful assistant that generates coherent, contextual responses.

Given a conversation history, generate a helpful and relevant response based on all the context available in the messages.
Your response should:
1. Be contextually aware of the entire conversation
2. Address the user's needs appropriately
3. Be helpful and informative
4. Maintain a natural conversational tone

Generate a complete response to assist the user."""

# Initialize OpenAI client for Plano
plano_client = AsyncOpenAI(
    base_url=LLM_GATEWAY_ENDPOINT,
    api_key="EMPTY",  # Plano doesn't require a real API key
)

# FastAPI app for REST server
app = FastAPI(title="RAG Agent Response Generator", version="1.0.0")


def prepare_response_messages(request_body: ChatCompletionRequest):
    """Prepare messages for response generation by adding system prompt."""
    response_messages = [{"role": "system", "content": SYSTEM_PROMPT}]

    # Add conversation history
    for msg in request_body.messages:
        response_messages.append({"role": msg.role, "content": msg.content})

    return response_messages


@app.post("/v1/chat/completions")
async def chat_completion_http(request: Request, request_body: ChatCompletionRequest):
    """HTTP endpoint for chat completions with streaming support."""
    logger.info(
        f"Received chat completion request with {len(request_body.messages)} messages"
    )

    # Get traceparent header from HTTP request
    traceparent_header = request.headers.get("traceparent")
    request_id = request.headers.get("x-request-id")

    if traceparent_header:
        logger.info(f"Received traceparent header: {traceparent_header}")
    else:
        logger.info("No traceparent header found")

    return StreamingResponse(
        stream_chat_completions(request_body, traceparent_header, request_id),
        media_type="text/plain",
        headers={
            "content-type": "text/event-stream",
        },
    )


async def stream_chat_completions(
    request_body: ChatCompletionRequest,
    traceparent_header: str = None,
    request_id: str = None,
):
    """Generate streaming chat completions."""
    # Prepare messages for response generation
    response_messages = prepare_response_messages(request_body)

    try:
        # Call Plano using OpenAI client for streaming
        logger.info(
            f"Calling Plano at {LLM_GATEWAY_ENDPOINT} to generate streaming response"
        )

        logger.info(f"rag_agent - request_id: {request_id}")
        # Prepare extra headers if traceparent is provided
        extra_headers = {"x-envoy-max-retries": "3"}
        if request_id:
            extra_headers["x-request-id"] = request_id
        if traceparent_header:
            extra_headers["traceparent"] = traceparent_header

        response_stream = await plano_client.chat.completions.create(
            model=RESPONSE_MODEL,
            messages=response_messages,
            temperature=request_body.temperature or 0.7,
            max_tokens=request_body.max_tokens or 1000,
            stream=True,
            extra_headers=extra_headers,
        )

        completion_id = f"chatcmpl-{uuid.uuid4().hex[:8]}"
        created_time = int(time.time())
        collected_content = []

        async for chunk in response_stream:
            if chunk.choices and chunk.choices[0].delta.content:
                content = chunk.choices[0].delta.content
                collected_content.append(content)

                # Create streaming response chunk
                stream_chunk = ChatCompletionStreamResponse(
                    id=completion_id,
                    created=created_time,
                    model=request_body.model,
                    choices=[
                        {
                            "index": 0,
                            "delta": {"content": content},
                            "finish_reason": None,
                        }
                    ],
                )

                yield f"data: {stream_chunk.model_dump_json()}\n\n"

        # Send final chunk with complete response in expected format
        full_response = "".join(collected_content)
        updated_history = [{"role": "assistant", "content": full_response}]

        final_chunk = ChatCompletionStreamResponse(
            id=completion_id,
            created=created_time,
            model=request_body.model,
            choices=[
                {
                    "index": 0,
                    "delta": {},
                    "finish_reason": "stop",
                    "message": {
                        "role": "assistant",
                        "content": json.dumps(updated_history),
                    },
                }
            ],
        )

        yield f"data: {final_chunk.model_dump_json()}\n\n"
        yield "data: [DONE]\n\n"

    except Exception as e:
        logger.error(f"Error generating streaming response: {e}")

        # Send error as streaming response
        error_chunk = ChatCompletionStreamResponse(
            id=f"chatcmpl-{uuid.uuid4().hex[:8]}",
            created=int(time.time()),
            model=request_body.model,
            choices=[
                {
                    "index": 0,
                    "delta": {
                        "content": "I apologize, but I'm having trouble generating a response right now. Please try again."
                    },
                    "finish_reason": "stop",
                }
            ],
        )

        yield f"data: {error_chunk.model_dump_json()}\n\n"
        yield "data: [DONE]\n\n"


@app.get("/health")
async def health_check():
    """Health check endpoint."""
    return {"status": "healthy"}


def start_server(host: str = "localhost", port: int = 8000):
    """Start the REST server."""
    uvicorn.run(
        app,
        host=host,
        port=port,
        log_config={
            "version": 1,
            "disable_existing_loggers": False,
            "formatters": {
                "default": {
                    "format": "%(asctime)s - [RESPONSE_GENERATOR] - %(levelname)s - %(message)s",
                },
            },
            "handlers": {
                "default": {
                    "formatter": "default",
                    "class": "logging.StreamHandler",
                    "stream": "ext://sys.stdout",
                },
            },
            "root": {
                "level": "INFO",
                "handlers": ["default"],
            },
        },
    )
