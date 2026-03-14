# PII Anonymization Filter Chain Demo

Automatically redact PII from LLM requests and restore it in responses — inspired by [Uber's GenAI Gateway PII Redactor](https://www.uber.com/blog/genai-gateway/).

This demo uses both `input_filters` and `output_filters` on a **model-type listener** to anonymize PII before it reaches the LLM provider, then de-anonymize the response before returning it to the client.

## Architecture

```
Client ──► Plano (model listener :12000)
               │
               ├─ input_filters: pii_anonymizer
               │     └─ Replace PII with [EMAIL_0], [SSN_0], etc.
               │
               ├─ model_provider: openai/gpt-4o-mini
               │     └─ LLM only sees anonymized data
               │
               └─ output_filters: pii_deanonymizer
                     └─ Restore [EMAIL_0] → original email (per-chunk for streaming)
```

## Quick Start

```bash
# 1. Export your API key
export OPENAI_API_KEY=sk-...

# 2. Start the demo
bash run_demo.sh

# 3. (Optional) Start with Jaeger tracing
bash run_demo.sh --with-ui

# 4. Run tests (in another terminal)
bash test.sh

# 5. Stop the demo
bash run_demo.sh down
```

## Try It

**Request with PII:**

```bash
curl http://localhost:12000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "messages": [{"role": "user", "content": "Contact john@example.com or call 555-123-4567"}],
    "stream": false
  }'
```

**Streaming with PII:**

```bash
curl --no-buffer http://localhost:12000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "messages": [{"role": "user", "content": "My SSN is 123-45-6789"}],
    "stream": true
  }'
```

## Verify Anonymization

Check the PII filter service logs in the terminal running `start_agents.sh`. You should see log lines like:
```
[PII_ANONYMIZER] - INFO - request_id=abc123 anonymized PII: EMAIL=1, PHONE=1
```

## Supported PII Types

| Type | Pattern | Example | Placeholder |
|------|---------|---------|-------------|
| SSN | `XXX-XX-XXXX` | `123-45-6789` | `[SSN_0]` |
| Credit Card | `XXXX XXXX XXXX XXXX` | `4111 1111 1111 1111` | `[CREDIT_CARD_0]` |
| Email | standard email format | `user@example.com` | `[EMAIL_0]` |
| Phone | US phone formats | `555-123-4567` | `[PHONE_0]` |

## How Streaming De-anonymization Works

For streaming responses, each SSE chunk is sent through the output filters as it arrives from the LLM:

1. Plano receives a chunk with content like `"The email [EMAIL_0] belongs to..."`
2. The chunk content is sent to the `/deanonymize` endpoint
3. The filter looks up the PII mapping (stored during anonymization) and replaces placeholders
4. The restored chunk `"The email john@example.com belongs to..."` is streamed to the client

Partial placeholders split across chunks (e.g., `[EMA` in one chunk, `IL_0]` in the next) are handled via internal buffering in the filter service.

## Limitations

- **No name detection** — regex cannot reliably detect names. For production, consider [Microsoft Presidio](https://github.com/microsoft/presidio) or spaCy NER.
- **US-centric patterns** — phone and SSN patterns are US-focused. International formats may not be detected.
- **Per-chunk latency** — streaming de-anonymization adds a small network round-trip per chunk (~1ms on local network).

## Tracing

Open [Jaeger UI](http://localhost:16686) to see distributed traces for requests flowing through the PII filter chain.
