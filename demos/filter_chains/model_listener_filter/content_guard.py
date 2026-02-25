"""
Content guard filter — keyword-based content safety for model listeners.

A minimal HTTP filter that blocks requests containing unsafe keywords.
No LLM calls required — keeps the demo self-contained and fast.
"""

import logging
from typing import List

from fastapi import FastAPI, Request, HTTPException
from pydantic import BaseModel

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


class ChatMessage(BaseModel):
    role: str
    content: str


def check_content(text: str) -> str | None:
    """Return the matched keyword if blocked, else None."""
    lower = text.lower()
    for kw in BLOCKED_KEYWORDS:
        if kw in lower:
            return kw
    return None


@app.post("/")
async def content_guard(
    messages: List[ChatMessage], request: Request
) -> List[ChatMessage]:
    """Block messages that contain unsafe keywords."""
    last_user_msg = None
    for msg in reversed(messages):
        if msg.role == "user":
            last_user_msg = msg.content
            break

    if last_user_msg is None:
        return messages

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
    return messages


@app.get("/health")
async def health():
    return {"status": "healthy"}
