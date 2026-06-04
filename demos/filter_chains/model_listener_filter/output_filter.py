"""
Output filter for model-listener filter demos.

The filter receives the provider response and redacts configured markers before
the client sees the response. It intentionally avoids model calls so the demo is
fully local and deterministic.
"""

import gzip
from typing import Any

from fastapi import FastAPI, Request
from fastapi.responses import Response

app = FastAPI(title="Output Redaction Filter", version="1.0.0")

SENSITIVE_MARKERS = ("SECRET_TOKEN",)


def redact_text(text: str) -> str:
    redacted = text
    for marker in SENSITIVE_MARKERS:
        redacted = redacted.replace(marker, "[REDACTED]")
    return redacted


def redact_chat_completion(body: dict[str, Any]) -> dict[str, Any]:
    choices = []
    for choice in body.get("choices", []):
        message = choice.get("message", {})
        content = message.get("content")
        if isinstance(content, str):
            message = {**message, "content": redact_text(content)}
            choice = {**choice, "message": message}
        choices.append(choice)
    return {**body, "choices": choices}


def redact_bytes(raw_body: bytes) -> bytes:
    if raw_body.startswith(b"\x1f\x8b"):
        decompressed_body = gzip.decompress(raw_body)
        return gzip.compress(redact_bytes(decompressed_body))

    body_text = raw_body.decode("utf-8", errors="replace")
    return redact_text(body_text).encode("utf-8")


@app.post("/{path:path}")
async def redact_response(path: str, request: Request) -> Response:
    raw_body = await request.body()
    content_type = request.headers.get("content-type", "application/json")
    return Response(content=redact_bytes(raw_body), media_type=content_type)


@app.get("/health")
async def health() -> dict[str, str]:
    return {"status": "healthy"}
