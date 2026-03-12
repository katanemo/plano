"""
PII Anonymization filter — redact and restore PII in LLM requests/responses.

Inspired by Uber's GenAI Gateway PII Redactor. Two endpoints:
  POST /anonymize    — replace PII with placeholders (input filter)
  POST /deanonymize  — restore original PII from placeholders (output filter)

Uses regex-based detection for: email, phone, SSN, credit card.
Correlates request/response via x-request-id header.
"""

import logging
import re
import time
import threading
from typing import Dict, List, Optional, Tuple

from fastapi import FastAPI, Request
from pydantic import BaseModel

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - [PII_ANONYMIZER] - %(levelname)s - %(message)s",
)
logger = logging.getLogger(__name__)

app = FastAPI(title="PII Anonymizer", version="1.0.0")

# --- PII patterns (order matters: SSN before phone to avoid overlap) ---

PII_PATTERNS = [
    ("SSN", re.compile(r"\b\d{3}-\d{2}-\d{4}\b")),
    ("CREDIT_CARD", re.compile(r"\b(?:\d{4}[-\s]?){3}\d{4}\b")),
    ("EMAIL", re.compile(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}")),
    (
        "PHONE",
        re.compile(r"(\+?1[-.\s]?)?\(?\d{3}\)?[-.\s]?\d{3}[-.\s]?\d{4}"),
    ),
]

# --- In-memory mapping store (request_id -> mapping + timestamp) ---

_store_lock = threading.Lock()
_mapping_store: Dict[str, Tuple[Dict[str, str], float]] = {}
# Buffer for partial placeholder matches during streaming de-anonymization
_buffer_store: Dict[str, str] = {}
MAPPING_TTL_SECONDS = 300  # 5 minutes


def _cleanup_expired():
    """Remove expired mappings."""
    now = time.time()
    expired = [
        k for k, (_, ts) in _mapping_store.items() if now - ts > MAPPING_TTL_SECONDS
    ]
    for k in expired:
        del _mapping_store[k]
        _buffer_store.pop(k, None)


def _store_mapping(request_id: str, mapping: Dict[str, str]):
    with _store_lock:
        _cleanup_expired()
        _mapping_store[request_id] = (mapping, time.time())


def _get_mapping(request_id: str) -> Optional[Dict[str, str]]:
    with _store_lock:
        entry = _mapping_store.get(request_id)
        if entry:
            return entry[0]
        return None


# --- Core logic ---


class ChatMessage(BaseModel):
    role: str
    content: str


def anonymize_text(text: str) -> Tuple[str, Dict[str, str]]:
    """Replace PII with [TYPE_N] placeholders. Returns (anonymized_text, mapping)."""
    mapping: Dict[str, str] = {}
    counters: Dict[str, int] = {}
    # Track spans already matched to avoid overlapping replacements
    matched_spans: List[Tuple[int, int]] = []

    for pii_type, pattern in PII_PATTERNS:
        for match in pattern.finditer(text):
            start, end = match.start(), match.end()
            # Skip if this span overlaps with an already-matched span
            if any(s <= start < e or s < end <= e for s, e in matched_spans):
                continue
            matched_spans.append((start, end))
            idx = counters.get(pii_type, 0)
            counters[pii_type] = idx + 1
            placeholder = f"[{pii_type}_{idx}]"
            mapping[placeholder] = match.group()

    # Replace from right to left to preserve indices
    matched_spans.sort(reverse=True)
    result = text
    for start, end in matched_spans:
        original = text[start:end]
        # Find the placeholder for this original value
        placeholder = next(k for k, v in mapping.items() if v == original)
        result = result[:start] + placeholder + result[end:]

    return result, mapping


def deanonymize_text(
    text: str, mapping: Dict[str, str], buffer: str = ""
) -> Tuple[str, str]:
    """Replace placeholders back with original PII values.

    Handles partial placeholders across streaming chunks via a buffer.
    Only buffers text that could be the prefix of an actual placeholder
    from this request's mapping, not arbitrary ``[`` from normal text.
    Returns (processed_text, remaining_buffer).
    """
    combined = buffer + text

    # Build the set of all prefixes for placeholders in this request's mapping.
    # e.g. for "[EMAIL_0]" -> {"[", "[E", "[EM", "[EMA", "[EMAI", "[EMAIL", "[EMAIL_", "[EMAIL_0"}
    prefixes: set[str] = set()
    for placeholder in mapping:
        # Exclude the full placeholder (with closing ']') — that's a complete match, not partial
        for i in range(1, len(placeholder)):
            prefixes.add(placeholder[:i])

    # Check if the end of the text could be a partial placeholder.
    remaining_buffer = ""
    last_bracket = combined.rfind("[")
    if last_bracket != -1 and "]" not in combined[last_bracket:]:
        tail = combined[last_bracket:]
        if tail in prefixes:
            remaining_buffer = tail
            combined = combined[:last_bracket]

    # Replace all complete placeholders
    for placeholder, original in mapping.items():
        combined = combined.replace(placeholder, original)

    return combined, remaining_buffer


# --- Endpoints ---


@app.post("/anonymize")
async def anonymize(messages: List[ChatMessage], request: Request) -> List[ChatMessage]:
    """Anonymize PII in user messages. Stores mapping for later de-anonymization."""
    request_id = request.headers.get("x-request-id", "unknown")
    all_mappings: Dict[str, str] = {}
    result_messages = []

    for msg in messages:
        if msg.role == "user":
            anonymized, mapping = anonymize_text(msg.content)
            all_mappings.update(mapping)
            result_messages.append(ChatMessage(role=msg.role, content=anonymized))
        else:
            result_messages.append(msg)

    if all_mappings:
        _store_mapping(request_id, all_mappings)
        logger.info(
            "request_id=%s /anonymize mapping: %s",
            request_id,
            all_mappings,
        )
    else:
        logger.info("request_id=%s no PII detected", request_id)

    logger.info(
        "request_id=%s /anonymize input: %s -> output: %s",
        request_id,
        [m.content for m in messages],
        [m.content for m in result_messages],
    )

    return result_messages


@app.post("/deanonymize")
async def deanonymize(
    messages: List[ChatMessage], request: Request
) -> List[ChatMessage]:
    """De-anonymize PII placeholders in response messages using stored mapping."""
    request_id = request.headers.get("x-request-id", "unknown")
    mapping = _get_mapping(request_id)

    if not mapping:
        logger.info("request_id=%s no mapping found, passing through", request_id)
        return messages

    result_messages = []
    for msg in messages:
        if msg.role == "assistant" and msg.content:
            with _store_lock:
                buffer = _buffer_store.get(request_id, "")

            restored, remaining = deanonymize_text(msg.content, mapping, buffer)

            with _store_lock:
                if remaining:
                    _buffer_store[request_id] = remaining
                else:
                    _buffer_store.pop(request_id, None)

            # Only log when a replacement actually happened
            if restored != msg.content:
                logger.info(
                    "request_id=%s /deanonymize '%s' -> '%s'",
                    request_id,
                    msg.content,
                    restored,
                )

            result_messages.append(ChatMessage(role=msg.role, content=restored))
        else:
            result_messages.append(msg)

    return result_messages


@app.get("/health")
async def health():
    return {"status": "healthy"}
