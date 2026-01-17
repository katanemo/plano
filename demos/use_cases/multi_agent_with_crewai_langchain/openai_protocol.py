"""OpenAI API protocol utilities for standardized response formatting."""

import time
import uuid
from typing import Optional


def create_chat_completion_chunk(
    model: str,
    content: str,
    finish_reason: Optional[str] = None,
) -> dict:
    """Create an OpenAI-compatible streaming chat completion chunk.

    Args:
        model: Model identifier to include in the response
        content: Content text for this chunk
        finish_reason: Optional finish reason ('stop', 'length', etc.)

    Returns:
        Dictionary formatted as OpenAI chat.completion.chunk
    """
    return {
        "id": f"chatcmpl-{uuid.uuid4().hex[:8]}",
        "object": "chat.completion.chunk",
        "created": int(time.time()),
        "model": model,
        "choices": [
            {
                "index": 0,
                "delta": {"content": content} if content else {},
                "finish_reason": finish_reason,
            }
        ],
    }
