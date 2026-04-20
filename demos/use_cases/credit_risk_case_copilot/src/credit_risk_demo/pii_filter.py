import json
import logging
import re
from typing import Any, Dict, List, Optional, Union

import uvicorn
from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse
from pydantic import BaseModel

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - [PII_FILTER] - %(levelname)s - %(message)s",
)
logger = logging.getLogger(__name__)

app = FastAPI(title="PII Security Filter", version="1.0.0")

# PII patterns
CNIC_PATTERN = re.compile(r"\b\d{5}-\d{7}-\d{1}\b")
PHONE_PATTERN = re.compile(r"\b(\+92|0)?3\d{9}\b")
EMAIL_PATTERN = re.compile(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b")

# Prompt injection patterns
INJECTION_PATTERNS = [
    r"ignore\s+(all\s+)?previous\s+(instructions?|prompts?)",
    r"ignore\s+policy",
    r"bypass\s+checks?",
    r"reveal\s+system\s+prompt",
    r"you\s+are\s+now",
    r"forget\s+(everything|all)",
]


class PiiFilterRequest(BaseModel):
    messages: List[Dict[str, Any]]
    model: Optional[str] = None


def redact_pii(text: str) -> tuple[str, list]:
    """Redact PII from text and return redacted text + list of findings."""
    findings = []
    redacted = text

    # Redact CNIC
    cnic_matches = CNIC_PATTERN.findall(text)
    if cnic_matches:
        findings.append(f"CNIC patterns found: {len(cnic_matches)}")
        redacted = CNIC_PATTERN.sub("[REDACTED_CNIC]", redacted)

    # Redact phone
    phone_matches = PHONE_PATTERN.findall(text)
    if phone_matches:
        findings.append(f"Phone numbers found: {len(phone_matches)}")
        redacted = PHONE_PATTERN.sub("[REDACTED_PHONE]", redacted)

    # Redact email
    email_matches = EMAIL_PATTERN.findall(text)
    if email_matches:
        findings.append(f"Email addresses found: {len(email_matches)}")
        redacted = EMAIL_PATTERN.sub("[REDACTED_EMAIL]", redacted)

    return redacted, findings


def detect_injection(text: str) -> tuple[bool, list]:
    """Detect potential prompt injection attempts."""
    detected = False
    patterns_matched = []

    text_lower = text.lower()
    for pattern in INJECTION_PATTERNS:
        if re.search(pattern, text_lower):
            detected = True
            patterns_matched.append(pattern)

    return detected, patterns_matched


@app.post("/v1/tools/pii_security_filter")
async def pii_security_filter(request: Union[PiiFilterRequest, List[Dict[str, Any]]]):
    try:
        if isinstance(request, list):
            messages = request
        else:
            messages = request.messages

        security_events = []

        for msg in messages:
            if msg.get("role") == "user":
                content = msg.get("content", "")

                redacted_content, pii_findings = redact_pii(content)
                if pii_findings:
                    security_events.extend(pii_findings)
                    msg["content"] = redacted_content
                    logger.warning(f"PII redacted: {pii_findings}")

                is_injection, patterns = detect_injection(content)
                if is_injection:
                    security_event = f"Prompt injection detected: {patterns}"
                    security_events.append(security_event)
                    logger.warning(security_event)
                    msg["content"] = (
                        f"[SECURITY WARNING: Potential prompt injection detected]\n\n{msg['content']}"
                    )

        # Optional: log metadata server-side (but don't return it to Plano)
        logger.info(
            f"Filter events: {security_events} | pii_redacted={any('found' in e for e in security_events)} "
            f"| injection_detected={any('injection' in e.lower() for e in security_events)}"
        )

        # IMPORTANT: return only the messages list (JSON array)
        return JSONResponse(content=messages)

    except Exception as e:
        logger.error(f"Filter error: {e}", exc_info=True)
        return JSONResponse(status_code=500, content={"error": str(e)})


@app.get("/health")
async def health_check():
    return {"status": "healthy", "service": "pii-security-filter"}


if __name__ == "__main__":
    logger.info("Starting PII Security Filter on port 10550")
    uvicorn.run(app, host="0.0.0.0", port=10550)
