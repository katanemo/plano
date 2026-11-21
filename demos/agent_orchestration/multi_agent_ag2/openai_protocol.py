"""OpenAI API protocol utilities for standardized response formatting."""

import time
from typing import Optional
from openai.types.chat import ChatCompletionChunk
from openai.types.chat.chat_completion_chunk import Choice, ChoiceDelta


def create_chat_completion_chunk(
    model: str,
    content: str,
    finish_reason: Optional[str] = None,
) -> ChatCompletionChunk:
    """Create an OpenAI-compatible streaming chat completion chunk.

    Args:
        model: Model identifier to include in the response
        content: Content text for this chunk
        finish_reason: Optional finish reason ('stop', 'length', etc.)

    Returns:
        ChatCompletionChunk object from OpenAI SDK
    """
    return ChatCompletionChunk(
        id=f"chatcmpl-{int(time.time() * 1000000)}",
        object="chat.completion.chunk",
        created=int(time.time()),
        model=model,
        choices=[
            Choice(
                index=0,
                delta=ChoiceDelta(content=content) if content else ChoiceDelta(),
                finish_reason=finish_reason,
            )
        ],
    )
