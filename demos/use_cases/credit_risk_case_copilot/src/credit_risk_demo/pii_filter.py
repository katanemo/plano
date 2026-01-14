import json
import logging
import re
from typing import Any, Dict

import uvicorn
from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse
from pydantic import BaseModel

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - [PII_FILTER] - %(levelname)s - %(message)s",
)
logger = logging.getLogger(__name__)

app = FastAPI(title="PII Security Filter (MCP)", version="1.0.0")

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


class MCPRequest(BaseModel):
    messages: list
    model: str = None


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
async def pii_security_filter(request: MCPRequest):
    """MCP filter endpoint for PII redaction and injection detection."""
    try:
        messages = request.messages
        security_events = []

        # Process each message
        for msg in messages:
            if msg.get("role") == "user":
                content = msg.get("content", "")

                # PII redaction
                redacted_content, pii_findings = redact_pii(content)
                if pii_findings:
                    security_events.extend(pii_findings)
                    msg["content"] = redacted_content
                    logger.warning(f"PII redacted: {pii_findings}")

                # Injection detection
                is_injection, patterns = detect_injection(content)
                if is_injection:
                    security_event = f"Prompt injection detected: {patterns}"
                    security_events.append(security_event)
                    logger.warning(security_event)

                    # Add warning to content
                    msg["content"] = (
                        f"[SECURITY WARNING: Potential prompt injection detected]\n\n{msg['content']}"
                    )

        # Return filtered messages
        response = {
            "messages": messages,
            "metadata": {
                "security_events": security_events,
                "pii_redacted": len([e for e in security_events if "found" in e]) > 0,
                "injection_detected": len(
                    [e for e in security_events if "injection" in e.lower()]
                )
                > 0,
            },
        }

        return JSONResponse(content=response)

    except Exception as e:
        logger.error(f"Filter error: {e}", exc_info=True)
        return JSONResponse(status_code=500, content={"error": str(e)})


@app.get("/health")
async def health_check():
    return {"status": "healthy", "service": "pii-security-filter"}


if __name__ == "__main__":
    logger.info("Starting PII Security Filter on port 10550")
    uvicorn.run(app, host="0.0.0.0", port=10550)
