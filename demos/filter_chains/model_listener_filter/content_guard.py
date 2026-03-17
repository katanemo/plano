"""
Content guard filter — keyword-based content safety for model listeners.

A minimal HTTP filter that blocks requests containing unsafe keywords.
No LLM calls required — keeps the demo self-contained and fast.

Receives the full raw request body (any API format: /v1/chat/completions,
/v1/responses, /v1/messages) and returns it unchanged or raises 400.
"""

import logging
from typing import Any

from fastapi import FastAPI, Request, HTTPException

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - [CONTENT_GUARD] - %(levelname)s - %(message)s",
)
logger = logging.getLogger(__name__)

app = FastAPI(title="Content Guard", version="1.0.0")

BLOCKED_KEYWORDS = [
    "hack",
    "exploit",
    "attack",
    "malware",
    "phishing",
    "ransomware",
    "ddos",
    "injection",
    "brute force",
    "keylogger",
    "bypass security",
    "steal credentials",
    "social engineering",
]


def check_content(text: str) -> str | None:
    """Return the matched keyword if blocked, else None."""
    lower = text.lower()
    for kw in BLOCKED_KEYWORDS:
        if kw in lower:
            return kw
    return None


def extract_last_user_text(body: dict[str, Any]) -> str | None:
    """Extract the most recent user message text from any supported request format."""
    messages = body.get("messages", [])
    # Anthropic /v1/messages and OpenAI /v1/chat/completions both use "messages"
    for msg in reversed(messages):
        if msg.get("role") == "user":
            content = msg.get("content", "")
            if isinstance(content, str):
                return content
            if isinstance(content, list):
                # Multimodal content parts
                return " ".join(
                    part.get("text", "")
                    for part in content
                    if isinstance(part, dict) and part.get("type") == "text"
                )

    # OpenAI /v1/responses uses "input" instead of "messages"
    input_val = body.get("input")
    if isinstance(input_val, str):
        return input_val
    if isinstance(input_val, list):
        for item in reversed(input_val):
            if isinstance(item, dict) and item.get("role") == "user":
                content = item.get("content", "")
                if isinstance(content, str):
                    return content

    return None


@app.post("/{path:path}")
async def content_guard(path: str, request: Request) -> dict[str, Any]:
    """Block requests containing unsafe keywords. Returns the full request body unchanged.

    The endpoint path encodes the API format:
      /v1/chat/completions  — check body["messages"]
      /v1/responses         — check body["input"]
      /v1/messages          — check body["messages"] (Anthropic format)
    """
    endpoint = f"/{path}"
    body = await request.json()

    # /v1/responses uses "input" instead of "messages"
    if endpoint == "/v1/responses":
        input_val = body.get("input", "")
        last_user_msg = input_val if isinstance(input_val, str) else None
    else:
        last_user_msg = extract_last_user_text(body)

    if last_user_msg is None:
        return body

    matched = check_content(last_user_msg)
    if matched:
        logger.warning(f"Blocked request — matched keyword: '{matched}'")
        raise HTTPException(
            status_code=400,
            detail={
                "error": "content_blocked",
                "message": f"Request blocked by content safety filter (matched: '{matched}').",
            },
        )

    logger.info("Content check passed — forwarding request")
    return body


@app.get("/health")
async def health():
    return {"status": "healthy"}
