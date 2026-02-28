# Credit Risk Case Copilot

A small demo that follows the two-loop model: Plano is the **outer loop** (routing, guardrails, tracing), and each credit-risk step is a focused **inner-loop agent**.

---

## What runs

- **Risk Crew Agent (10530)**: four OpenAI-compatible endpoints (intake, risk, policy, memo).
- **PII Filter (10550)**: redacts PII and flags prompt injection.
- **Streamlit UI (8501)**: single-call client.
- **Jaeger (16686)**: tracing backend.

---

## Quick start

```bash
cp .env.example .env
# add OPENAI_API_KEY
docker compose up --build
uvx planoai up config.yaml
```

Open:
- Streamlit UI: http://localhost:8501
- Jaeger: http://localhost:16686

---

## How it works

1. The UI sends **one** request to Plano with the application JSON.
2. Plano routes the request across the four agents in order:
   intake → risk → policy → memo.
3. Each agent returns JSON with a `step` key.
4. The memo agent returns the final response.

All model calls go through Plano’s LLM gateway, and guardrails run before any agent sees input.

---

## Endpoints

Risk Crew Agent (10530):
- `POST /v1/agents/intake/chat/completions`
- `POST /v1/agents/risk/chat/completions`
- `POST /v1/agents/policy/chat/completions`
- `POST /v1/agents/memo/chat/completions`
- `GET /health`

PII Filter (10550):
- `POST /v1/tools/pii_security_filter`
- `GET /health`

Plano (8001):
- `POST /v1/chat/completions`

---

## UI flow

1. Paste or select an application JSON.
2. Click **Assess Risk**.
3. Review the decision memo.

---

## Troubleshooting

- **No response**: confirm Plano is running and ports are free (`8001`, `10530`, `10550`, `8501`).
- **LLM gateway errors**: check `LLM_GATEWAY_ENDPOINT=http://host.docker.internal:12000/v1`.
- **No traces**: check Jaeger and `OTLP_ENDPOINT`.
