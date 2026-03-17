"""
PII Anonymization filter — redact and restore PII in LLM requests/responses.

Inspired by Uber's GenAI Gateway PII Redactor. Two endpoints:
  POST /anonymize    — replace PII with placeholders (input filter)
  POST /deanonymize  — restore original PII from placeholders (output filter)

Input filter (/anonymize):
  Receives the full raw request body (any API format). Anonymizes user message
  content and returns the modified body.

Output filter (/deanonymize):
  Receives raw LLM response bytes — SSE (streaming) or full JSON (non-streaming).
  De-anonymizes content and returns modified bytes.

The path suffix encodes the upstream API format so each endpoint knows how to
parse the body (e.g. /anonymize/v1/chat/completions, /deanonymize/v1/messages).
"""

import logging
from typing import Any, Dict

from fastapi import FastAPI, Request
from fastapi.responses import Response

from pii import anonymize_text, anonymize_message_content
from store import get_mapping, store_mapping, deanonymize_sse, deanonymize_json

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - [PII_ANONYMIZER] - %(levelname)s - %(message)s",
)
logger = logging.getLogger(__name__)

app = FastAPI(title="PII Anonymizer", version="1.0.0")


@app.post("/anonymize/{path:path}")
async def anonymize(path: str, request: Request) -> dict[str, Any]:
    """Anonymize PII in user messages. Receives and returns the full raw request body.

    The endpoint path encodes the API format:
      /anonymize/v1/chat/completions  — anonymize body["messages"]
      /anonymize/v1/responses         — anonymize body["input"] (string or items list)
      /anonymize/v1/messages          — anonymize body["messages"] (Anthropic format)
    """
    request_id = request.headers.get("x-request-id", "unknown")
    endpoint = f"/{path}"
    body = await request.json()
    all_mappings: Dict[str, str] = {}

    if endpoint == "/v1/responses":
        input_val = body.get("input", "")
        if isinstance(input_val, str):
            anonymized, mapping = anonymize_text(input_val)
            all_mappings.update(mapping)
            body = {**body, "input": anonymized}
        elif isinstance(input_val, list):
            items = [
                {**item, "content": anonymize_message_content(item.get("content", ""), all_mappings)}
                if isinstance(item, dict) and item.get("role") == "user"
                else item
                for item in input_val
            ]
            body = {**body, "input": items}
    else:
        # /v1/chat/completions and /v1/messages both use "messages"
        messages = [
            {**msg, "content": anonymize_message_content(msg.get("content", ""), all_mappings)}
            if msg.get("role") == "user"
            else msg
            for msg in body.get("messages", [])
        ]
        if messages:
            body = {**body, "messages": messages}

    if all_mappings:
        store_mapping(request_id, all_mappings)
        logger.info("request_id=%s /anonymize mapping: %s", request_id, all_mappings)
    else:
        logger.info("request_id=%s no PII detected", request_id)

    return body


@app.post("/deanonymize/{path:path}")
async def deanonymize(path: str, request: Request) -> Response:
    """De-anonymize PII placeholders in LLM response. Handles SSE (streaming) and JSON.

    The path encodes the upstream API format:
      /deanonymize/v1/chat/completions  — OpenAI chat completions
      /deanonymize/v1/messages          — Anthropic messages
      /deanonymize/v1/responses         — OpenAI responses API
    """
    endpoint = f"/{path}"
    is_anthropic = endpoint == "/v1/messages"
    request_id = request.headers.get("x-request-id", "unknown")
    mapping = get_mapping(request_id)
    raw_body = await request.body()

    if not mapping:
        logger.info("request_id=%s no mapping found, passing through", request_id)
        return Response(content=raw_body, media_type="application/json")

    body_str = raw_body.decode("utf-8", errors="replace")

    if "data: " in body_str:
        return deanonymize_sse(request_id, body_str, mapping, is_anthropic)
    return deanonymize_json(request_id, raw_body, body_str, mapping, is_anthropic)


@app.get("/health")
async def health():
    return {"status": "healthy"}
