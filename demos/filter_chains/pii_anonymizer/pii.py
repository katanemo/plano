"""PII detection and anonymization utilities."""

import re
from typing import Any, Dict, List, Tuple

# Order matters: SSN before phone to avoid overlap
PII_PATTERNS = [
    ("SSN", re.compile(r"\b\d{3}-\d{2}-\d{4}\b")),
    ("CREDIT_CARD", re.compile(r"\b(?:\d{4}[-\s]?){3}\d{4}\b")),
    ("EMAIL", re.compile(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}")),
    ("PHONE", re.compile(r"(\+?1[-.\s]?)?\(?\d{3}\)?[-.\s]?\d{3}[-.\s]?\d{4}")),
]


def anonymize_text(text: str) -> Tuple[str, Dict[str, str]]:
    """Replace PII with [TYPE_N] placeholders. Returns (anonymized_text, mapping)."""
    mapping: Dict[str, str] = {}
    counters: Dict[str, int] = {}
    matched_spans: List[Tuple[int, int]] = []

    for pii_type, pattern in PII_PATTERNS:
        for match in pattern.finditer(text):
            start, end = match.start(), match.end()
            if any(s <= start < e or s < end <= e for s, e in matched_spans):
                continue
            matched_spans.append((start, end))
            idx = counters.get(pii_type, 0)
            counters[pii_type] = idx + 1
            mapping[f"[{pii_type}_{idx}]"] = match.group()

    # Replace right-to-left to preserve span indices
    matched_spans.sort(reverse=True)
    result = text
    for start, end in matched_spans:
        placeholder = next(k for k, v in mapping.items() if v == text[start:end])
        result = result[:start] + placeholder + result[end:]

    return result, mapping


def deanonymize_text(
    text: str, mapping: Dict[str, str], buffer: str = ""
) -> Tuple[str, str]:
    """Replace placeholders back with original PII values.

    Handles partial placeholders at chunk boundaries via a buffer.
    Returns (processed_text, remaining_buffer).
    """
    combined = buffer + text

    # Build prefix set for all known placeholders (e.g. "[EMAIL_0" is a prefix of "[EMAIL_0]")
    prefixes: set[str] = set()
    for placeholder in mapping:
        for i in range(1, len(placeholder)):
            prefixes.add(placeholder[:i])

    # If the tail looks like the start of a placeholder, hold it in the buffer
    remaining_buffer = ""
    last_bracket = combined.rfind("[")
    if last_bracket != -1 and "]" not in combined[last_bracket:]:
        tail = combined[last_bracket:]
        if tail in prefixes:
            remaining_buffer = tail
            combined = combined[:last_bracket]

    for placeholder, original in mapping.items():
        combined = combined.replace(placeholder, original)

    return combined, remaining_buffer


def anonymize_message_content(content: Any, all_mappings: Dict[str, str]) -> Any:
    """Anonymize string content or list of content parts."""
    if isinstance(content, str):
        anonymized, mapping = anonymize_text(content)
        all_mappings.update(mapping)
        return anonymized
    if isinstance(content, list):
        result = []
        for part in content:
            if isinstance(part, dict) and part.get("type") == "text":
                anonymized, mapping = anonymize_text(part.get("text", ""))
                all_mappings.update(mapping)
                result.append({**part, "text": anonymized})
            else:
                result.append(part)
        return result
    return content
