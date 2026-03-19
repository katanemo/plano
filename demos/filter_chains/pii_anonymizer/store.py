"""In-memory mapping store and LLM response processors for PII de-anonymization."""

import json
import logging
import threading
import time
from typing import AsyncIterator, Dict, Optional, Tuple

from fastapi.responses import Response

from pii import deanonymize_text

logger = logging.getLogger(__name__)

MAPPING_TTL_SECONDS = 300  # 5 minutes

_lock = threading.Lock()
_mappings: Dict[str, Tuple[Dict[str, str], float]] = {}
_buffers: Dict[str, str] = {}  # partial placeholder buffers for streaming


def _cleanup_expired():
    now = time.time()
    expired = [k for k, (_, ts) in _mappings.items() if now - ts > MAPPING_TTL_SECONDS]
    for k in expired:
        del _mappings[k]
        _buffers.pop(k, None)


def store_mapping(request_id: str, mapping: Dict[str, str]):
    with _lock:
        _cleanup_expired()
        _mappings[request_id] = (mapping, time.time())


def get_mapping(request_id: str) -> Optional[Dict[str, str]]:
    with _lock:
        entry = _mappings.get(request_id)
        return entry[0] if entry else None


def restore_streaming(request_id: str, content: str, mapping: Dict[str, str]) -> str:
    """Restore PII in one streaming chunk, maintaining the per-request partial buffer."""
    with _lock:
        buffer = _buffers.get(request_id, "")
    restored, remaining = deanonymize_text(content, mapping, buffer)
    with _lock:
        if remaining:
            _buffers[request_id] = remaining
        else:
            _buffers.pop(request_id, None)
    if restored != content:
        logger.info(
            "request_id=%s restored '%s' -> '%s'", request_id, content, restored
        )
    return restored


def deanonymize_sse(
    request_id: str, body_str: str, mapping: Dict[str, str], is_anthropic: bool
) -> Response:
    result_lines = [
        _process_sse_line(request_id, line, mapping, is_anthropic)
        for line in body_str.split("\n")
    ]
    return Response(content="\n".join(result_lines), media_type="text/plain")


def _process_sse_line(
    request_id: str, line: str, mapping: Dict[str, str], is_anthropic: bool
) -> str:
    """Process a single SSE line, restoring PII in data payloads."""
    stripped = line.strip()
    if not (stripped.startswith("data: ") and stripped[6:] != "[DONE]"):
        return line
    try:
        chunk = json.loads(stripped[6:])
        if is_anthropic:
            if chunk.get("type") == "content_block_delta":
                delta = chunk.get("delta", {})
                if delta.get("type") == "text_delta" and delta.get("text"):
                    delta["text"] = restore_streaming(
                        request_id, delta["text"], mapping
                    )
        else:
            for choice in chunk.get("choices", []):
                delta = choice.get("delta", {})
                if delta.get("content"):
                    delta["content"] = restore_streaming(
                        request_id, delta["content"], mapping
                    )
        return "data: " + json.dumps(chunk)
    except json.JSONDecodeError:
        return line


async def deanonymize_sse_stream(
    request_id: str,
    byte_stream: AsyncIterator[bytes],
    mapping: Dict[str, str],
    is_anthropic: bool,
):
    """Async generator that reads SSE events from a streaming request body,
    de-anonymizes them, and yields processed events as they become complete.
    Buffers partial data and splits on SSE event boundaries (blank lines).
    """
    buffer = ""
    async for raw_chunk in byte_stream:
        buffer += raw_chunk.decode("utf-8", errors="replace")
        # Yield each complete SSE event (delimited by double newline)
        while "\n\n" in buffer:
            event, buffer = buffer.split("\n\n", 1)
            processed_lines = [
                _process_sse_line(request_id, line, mapping, is_anthropic)
                for line in event.split("\n")
            ]
            yield "\n".join(processed_lines) + "\n\n"
    # Flush any trailing data
    if buffer.strip():
        processed_lines = [
            _process_sse_line(request_id, line, mapping, is_anthropic)
            for line in buffer.split("\n")
        ]
        yield "\n".join(processed_lines)


def deanonymize_json(
    request_id: str,
    raw_body: bytes,
    body_str: str,
    mapping: Dict[str, str],
    is_anthropic: bool,
) -> Response:
    try:
        body = json.loads(body_str)
        if is_anthropic:
            # {"content": [{"type": "text", "text": "..."}]}
            for part in body.get("content", []):
                if (
                    isinstance(part, dict)
                    and part.get("type") == "text"
                    and part.get("text")
                ):
                    restored, _ = deanonymize_text(part["text"], mapping)
                    if restored != part["text"]:
                        logger.info(
                            "request_id=%s restored '%s' -> '%s'",
                            request_id,
                            part["text"],
                            restored,
                        )
                    part["text"] = restored
        else:
            # {"choices": [{"message": {"content": "..."}}]}
            for choice in body.get("choices", []):
                message = choice.get("message", {})
                content = message.get("content")
                if content and isinstance(content, str):
                    restored, _ = deanonymize_text(content, mapping)
                    if restored != content:
                        logger.info(
                            "request_id=%s restored '%s' -> '%s'",
                            request_id,
                            content,
                            restored,
                        )
                    message["content"] = restored
        return Response(content=json.dumps(body), media_type="application/json")
    except json.JSONDecodeError:
        return Response(content=raw_body, media_type="application/json")
