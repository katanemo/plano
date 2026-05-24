"""
OpenAI-compatible local provider for model-listener filter demos.

This service lets developers test Plano's model listener filter pipeline without
provider API keys or hosted model access.
"""

import json
import time
from typing import Any

from fastapi import FastAPI, Request
from fastapi.responses import Response, StreamingResponse

app = FastAPI(title="Local Fake LLM Provider", version="1.0.0")


def latest_user_content(messages: list[dict[str, Any]]) -> str:
    for message in reversed(messages):
        if message.get("role") == "user":
            content = message.get("content", "")
            if isinstance(content, str):
                return content
            if isinstance(content, list):
                return " ".join(
                    part.get("text", "")
                    for part in content
                    if isinstance(part, dict) and part.get("type") == "text"
                )
    return ""


@app.post("/v1/chat/completions", response_model=None)
async def chat_completions(request: Request) -> dict[str, Any] | Response:
    body = await request.json()
    model = body.get("model", "gpt-4o-mini")
    user_content = latest_user_content(body.get("messages", []))
    content = "Hello from the local fake provider."
    if "secret" in user_content.lower():
        content = "The local fake provider returned SECRET_TOKEN."

    if body.get("stream") is True:

        async def generate():
            chunk = {
                "id": "chatcmpl-local-filter-demo",
                "object": "chat.completion.chunk",
                "created": int(time.time()),
                "model": model,
                "choices": [
                    {
                        "index": 0,
                        "delta": {"role": "assistant", "content": content},
                        "finish_reason": None,
                    }
                ],
            }
            yield f"data: {json.dumps(chunk)}\n\n"
            yield "data: [DONE]\n\n"

        return StreamingResponse(generate(), media_type="text/event-stream")

    return {
        "id": "chatcmpl-local-filter-demo",
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
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
    }


@app.get("/health")
async def health() -> dict[str, str]:
    return {"status": "healthy"}
