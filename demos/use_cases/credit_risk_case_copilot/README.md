# Credit Risk Case Copilot

This directory contains a demo **credit risk assessment system** built to exercise Plano's orchestration, guardrails, and observability features in a realistic setup.

The goal of this project is not to show "yet another agent demo", but to answer a more practical question:

> How would you actually deploy an agentic AI system in a regulated environment?

To do that, the system includes a real multi-agent workflow, a security boundary in front of it, structured downstream actions, and full tracing across all components.

---

## Why this demo exists

Most agent demos run everything inside the agent:
- Agents call models directly
- There's no security boundary
- Observability is minimal
- Downstream systems are mocked or ignored

That works for experimentation, but it doesn't resemble how these systems would be deployed in production.

This demo flips that around:
- **Plano sits in front** as the control plane
- Requests are sanitized before reaching agents
- Agents are treated as untrusted workloads
- Every LLM call is routed, logged, and traceable

---

## High-level architecture

```
User / Streamlit UI
    ↓
Plano Orchestrator (8001)
├─ HTTP Security Filter (PII + injection)
├─ Agent routing
├─ Model routing
    ↓
Risk Crew Agent (CrewAI)
    ↓
Plano LLM Gateway (12000)
    ↓
OpenAI
```

Plano is the only component allowed to talk to models or invoke downstream systems. Everything else goes through it.

---

## What the system actually does

At a high level, the system takes an unstructured loan request and turns it into a structured credit decision.

Internally, this is implemented as a small CrewAI workflow with four agents:

1. Intake & normalization (gpt-4o-mini)
2. Risk scoring & drivers (gpt-4o)
3. Policy & compliance checks (gpt-4o)
4. Decision memo synthesis (gpt-4o)

Each agent builds on the output of the previous one. The workflow is sequential on purpose to make traces easier to follow.

The specific agent framework isn't the focus here — it's mainly used as a realistic payload for Plano to orchestrate.

---

## Plano features exercised in this demo

This demo actively uses several Plano capabilities together:

### Agent listener
- OpenAI-compatible `/v1/chat/completions` endpoint
- Requests are routed to the appropriate agent based on configuration
- Agents remain unaware of routing logic

### HTTP filter chain (security guardrails)
- Requests pass through an HTTP-based security filter before agent execution
- PII (CNIC, phone numbers, emails) is redacted in-place
- Prompt injection attempts are detected and flagged
- The agent receives only sanitized input

The filter is implemented as a simple HTTP service to keep things easy to debug and reason about.

### Central LLM gateway
- All LLM calls go through Plano
- Agents never talk to OpenAI directly
- Makes tracing, policy enforcement, and provider switching easier later

### Observability
- End-to-end OpenTelemetry tracing
- One trace per request, spanning:
  - Security filter
  - Agent execution
  - Individual LLM calls
  - Downstream API calls

---

## Example request flow

```bash
curl http://localhost:8001/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "openai/gpt-4o",
    "messages": [
      {
        "role": "user",
        "content": "CNIC 12345-1234567-1 assess risk for loan amount 300000"
      }
    ]
  }'
```

What happens:

1. Plano receives the request
2. The PII filter redacts the CNIC
3. The sanitized request is routed to the risk agent
4. The multi-agent workflow executes
5. The response is returned
6. The entire flow appears as a single trace in Jaeger

---

## Services in this repo

### Risk Crew Agent (10530)
Implements the CrewAI workflow and exposes four standalone OpenAI-compatible endpoints so Plano can route to each step independently.

The agent is intentionally kept unaware of:
- Security filters
- Model providers
- Routing decisions

### PII Security Filter (10550)
A small FastAPI service that:
- Redacts CNIC, phone numbers, and emails
- Detects common prompt injection patterns
- Mutates messages in-place
- Returns only the updated message list (as expected by Plano's HTTP filter interface)

This runs before the agent is invoked.

### Streamlit UI (8501)
A lightweight UI for interacting with the system:
- Provides example scenarios
- Displays structured outputs
- Useful for demos and manual testing

### Jaeger (16686)
Used for distributed tracing.  
All services emit OpenTelemetry spans.

---

## Observability notes

Open Jaeger at: **http://localhost:16686**

A typical trace shows:
- One parent request span
- A security filter span
- Four LLM call spans (one per agent step)

This is intentional — the trace tells the full story of what happened and why.

---

## Running the demo

### Prerequisites
- Docker + Docker Compose
- Plano CLI (`pip install planoai` or `uvx planoai`)
- OpenAI API key

### Environment setup
```bash
cp .env.example .env
# add OPENAI_API_KEY
```

### Start services
```bash
docker compose up --build
```

### Start Plano
In a separate terminal:

```bash
uvx planoai up config.yaml
```

Plano runs on:
- **8001** – agent listener
- **12000** – LLM gateway

### Access
- **Streamlit UI**: http://localhost:8501
- **Jaeger Traces**: http://localhost:16686

---

## Screenshots

### Streamlit UI
The UI provides a simple interface for testing scenarios and viewing risk assessments:

![Streamlit UI](images/ui-demo.png)

### PII Redaction in Action
The security filter automatically redacts sensitive information (CNIC, email, phone) before it reaches the agent:

![PII Redaction](images/pii-redaction.png)

### Prompt Injection Detection
The filter detects and flags malicious prompt injection attempts:

![Prompt Injection Detection](images/prompt-injection.png)

---

## Notes on design choices

- The PII filter is HTTP-based rather than MCP to keep the demo simpler to debug.
- Agents execute sequentially to make traces readable.
- Model aliases are supported by Plano, but the agent uses explicit model IDs to avoid ambiguity during the demo.
- Error handling favors fallback responses over hard failures.

These are demo choices, not hard requirements.

---

## What this demo demonstrates

- A real multi-agent workflow running behind a control plane
- Centralized security and routing
- Clear separation between agents and infrastructure
- End-to-end observability
- OpenAI-compatible APIs preserved throughout

This is closer to how agentic systems are likely to be deployed in practice.

## Using the Demo

### Streamlit UI Workflow

1. **Select a Scenario** (or paste your own JSON):
   - 🟢 **Scenario A** - Low risk (stable job, good credit, low DTI)
   - 🟡 **Scenario B** - Medium risk (thin file, missing verifications)
   - 🔴 **Scenario C** - High risk + prompt injection attempt

2. **Click "Assess Risk"** - The UI calls the four agents sequentially through Plano

3. **Review Results** - Memo + key summary fields, with normalized data in an expander

### Direct API Testing

You can also send requests directly to Plano:

```bash
curl http://localhost:8001/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o",
    "messages": [
      {
        "role": "user",
        "content": "Assess credit risk for this application: {\"applicant_name\": \"Sarah Ahmed\", \"loan_amount\": 300000, \"credit_score\": 780, \"monthly_income\": 200000, \"total_debt\": 25000, \"delinquencies\": 0, \"kyc_complete\": true, \"income_verified\": true}"
      }
    ]
  }'
```

## Example Scenarios

### Scenario A: Low Risk
- Applicant: Sarah Ahmed
- Credit Score: 780 (Excellent)
- DTI: 12.5% (Low)
- Delinquencies: 0
- KYC: Complete
- **Expected**: LOW risk, APPROVE recommendation

### Scenario B: Medium Risk
- Applicant: Hassan Khan
- Credit Score: 620 (Fair)
- DTI: 50% (Elevated)
- Delinquencies: 1
- KYC: Incomplete (missing income/address verification)
- **Expected**: MEDIUM risk, CONDITIONAL_APPROVE or REFER

### Scenario C: High Risk + Injection
- Applicant: Ali Raza
- Credit Score: 520 (Poor)
- DTI: 100% (Critical)
- Delinquencies: 3
- Contains: "Ignore all previous instructions" (prompt injection)
- **Expected**: HIGH risk, REJECT, PII redacted, injection detected

## Service Details

### Risk Crew Agent (Port 10530) - CrewAI Multi-Agent System

Implements four standalone endpoints where each agent is specialized:

1. **Intake & Normalization Agent** 
   - Model: `risk_fast` (gpt-4o-mini)
   - Task: Extract application data, normalize fields, calculate DTI, flag missing data
   - Output: Clean structured dataset with validation results

2. **Risk Scoring & Driver Analysis Agent**
   - Model: `risk_reasoning` (gpt-4o) 
   - Task: Analyze credit score, DTI, delinquencies, utilization
   - Output: Risk band (LOW/MEDIUM/HIGH) with confidence + top 3 risk drivers with evidence

3. **Policy & Compliance Agent**
   - Model: `risk_reasoning` (gpt-4o)
   - Task: Verify KYC completion, income/address verification, check policy violations
   - Output: Policy checks status + exceptions + required documents list

4. **Decision Memo & Action Agent**
   - Model: `risk_reasoning` (gpt-4o)
   - Task: Synthesize findings into bank-ready memo
   - Output: Executive summary + recommendation (APPROVE/CONDITIONAL_APPROVE/REFER/REJECT)

**Context Passing:** Each call includes the prior outputs as explicit JSON payloads:
- Intake expects the raw application JSON.
- Risk expects `{ application, intake }`.
- Policy expects `{ application, intake, risk }`.
- Memo expects `{ application, intake, risk, policy }`.

### PII Security Filter (Port 10550)

HTTP Filter that:
- Redacts CNIC patterns (12345-6789012-3)
- Redacts phone numbers (+923001234567)
- Redacts email addresses
- Detects prompt injections ("ignore policy", "bypass checks", etc.)
- Adds security warnings to flagged content

## Configuration Files

### config.yaml (Plano Configuration)

- **Agents**: `loan_intake_agent`, `risk_scoring_agent`, `policy_compliance_agent`, `decision_memo_agent`
- **Filters**: `pii_security_filter` in filter chain
- **Model Providers**: OpenAI GPT-4o and GPT-4o-mini
- **Model Aliases**: `risk_fast` (mini), `risk_reasoning` (4o)
- **Listeners**: agent (8001), model (12000)
- **Tracing**: 100% sampling to Jaeger

### docker-compose.yaml

Orchestrates 4 services:
- `risk-crew-agent` - Risk assessment engine
- `pii-filter` - Security filter
- `streamlit-ui` - Web interface
- `jaeger` - Tracing backend

## Observability

### Jaeger Tracing

View distributed traces at http://localhost:16686

**CrewAI Multi-Agent Trace Flow:**
```
chat_completions (risk-crew-agent) - 8500ms
├─ crewai_risk_assessment_workflow - 8200ms
│  ├─ POST /v1/chat/completions (risk_fast) - 800ms
│  │  └─ openai.chat.completions.create (gpt-4o-mini) - 750ms
│  ├─ POST /v1/chat/completions (risk_reasoning) - 2100ms
│  │  └─ openai.chat.completions.create (gpt-4o) - 2000ms
│  ├─ POST /v1/chat/completions (risk_reasoning) - 1800ms
│  │  └─ openai.chat.completions.create (gpt-4o) - 1750ms
│  └─ POST /v1/chat/completions (risk_reasoning) - 2400ms
│     └─ openai.chat.completions.create (gpt-4o) - 2350ms
```

**Complete Request Flow:**
1. UI sends request to Plano orchestrator (8001)
2. Plano applies PII security filter (10550)
3. Plano routes to Risk Crew Agent (10530)
4. CrewAI executes 4 agents sequentially:
   - Each agent calls Plano LLM Gateway (12000)
   - Plano routes to OpenAI with configured model alias
5. Agent returns synthesized assessment
6. All spans visible in Jaeger (16686)

**Search Tips:**
- Service: `risk-crew-agent`
- Operation: `chat_completions` or `crewai_risk_assessment_workflow`
- Tags: `request_id`, `risk_band`, `recommended_action`, `applicant_name`
- Look for 4-5 LLM call spans per request (indicates CrewAI is working)

## Project Structure

```
credit_risk_case_copilot/
├── config.yaml                      # Plano orchestrator config
├── docker-compose.yaml              # Service orchestration
├── Dockerfile                       # Multi-purpose container
├── pyproject.toml                   # Python dependencies
├── .env.example                     # Environment template
├── README.md                        # This file
├── test.rest                        # REST client examples
├── scenarios/                       # Test fixtures
│   ├── scenario_a_low_risk.json
│   ├── scenario_b_medium_risk.json
│   └── scenario_c_high_risk_injection.json
└── src/
    └── credit_risk_demo/
        ├── __init__.py
        ├── risk_crew_agent.py       # Multi-agent workflow (FastAPI)
        ├── pii_filter.py            # HTTP security filter (FastAPI)
        └── ui_streamlit.py          # Web UI (Streamlit)
```

## Development

### Running Services Individually

```bash
# Risk Crew Agent
uv run python src/credit_risk_demo/risk_crew_agent.py

# PII Filter
uv run python src/credit_risk_demo/pii_filter.py

# Streamlit UI
uv run streamlit run src/credit_risk_demo/ui_streamlit.py
```

### Installing Dependencies Locally

```bash
uv sync
# or
pip install -e .
```

## Troubleshooting

**Services won't start**
- Check Docker is running: `docker ps`
- Verify ports are available: `lsof -i :8001,10530,10550,8501,16686`
- Check logs: `docker compose logs -f`

**CrewAI Import Errors** (e.g., "No module named 'crewai'")
- Rebuild container with new dependencies:
  ```bash
  docker compose build risk-crew-agent --no-cache
  docker compose up risk-crew-agent
  ```

**Slow Response Times (>20 seconds)**
- **Expected:** 8-15 seconds is normal for CrewAI (4 sequential LLM calls)
- **If slower:** Check OpenAI API status, review Jaeger traces for bottlenecks, check Plano logs

**LLM Gateway Connection Failed**
- Verify Plano is running: `curl http://localhost:12000/health`
- Check environment variable: `docker compose exec risk-crew-agent env | grep LLM_GATEWAY`
- Should show: `LLM_GATEWAY_ENDPOINT=http://host.docker.internal:12000/v1`

**Plano won't start**
- Verify installation: `planoai --version`
- Check config: `planoai validate config.yaml`
- Ensure OPENAI_API_KEY is set

**No response from agents**
- Verify all services are healthy:
  - `curl http://localhost:10530/health` (should show `"framework": "CrewAI"`)
  - `curl http://localhost:10550/health`
- Check Plano is running on port 8001

**Streamlit can't connect**
- Verify PLANO_ENDPOINT in docker-compose matches Plano port
- Check `host.docker.internal` resolves (should point to host machine)

**Jaeger shows no traces**
- Verify OTLP_ENDPOINT in services points to Jaeger
- Check Jaeger is running: `docker compose ps jaeger`
- Allow a few seconds for traces to appear
- **CrewAI traces:** Look for `crewai_risk_assessment_workflow` span with 4 child LLM calls

**CrewAI Output Parsing Errors**
- Check logs: `docker compose logs risk-crew-agent | grep "Error parsing"`
- System falls back to basic response if parsing fails (check for "REFER" recommendation)

## API Endpoints

### Plano Orchestrator (8001)
- `POST /v1/chat/completions` - Main entry point (OpenAI-compatible)

### Risk Crew Agent (10530)
- `POST /v1/agents/intake/chat/completions` - Intake normalization endpoint
- `POST /v1/agents/risk/chat/completions` - Risk scoring endpoint
- `POST /v1/agents/policy/chat/completions` - Policy compliance endpoint
- `POST /v1/agents/memo/chat/completions` - Decision memo endpoint
- `POST /v1/chat/completions` - Full risk assessment endpoint (legacy)
- `GET /health` - Health check

### PII Filter (10550)
- `POST /v1/tools/pii_security_filter` - PII filter endpoint
- `GET /health` - Health check

## Next Steps & Extensions

### Immediate Enhancements
- Add database persistence for assessment storage (PostgreSQL/MongoDB)
- Implement parallel agent execution where possible (e.g., Risk + Policy agents)
- Add agent tools (credit bureau API integration, fraud detection)
- Enable CrewAI memory for cross-request learning

### Production Readiness
- Implement rate limiting and request throttling
- Add caching layer for repeated assessments
- Set up monitoring/alerting (Prometheus + Grafana)
- Implement user authentication and RBAC
- Add audit log persistence

### Feature Extensions
- Add Fraud Detection Agent to the crew
- Implement Appeals Agent for rejected applications
- Build analytics dashboard for risk metrics
- Add email/SMS notifications for decisions
- Implement batch processing API for multiple applications
- Create PDF export for decision memos
- Add A/B testing framework for different risk models

## What This Demo Demonstrates

This project showcases:

✅ **True Multi-Agent AI System** - 4 specialized CrewAI agents with distinct roles and expertise  
✅ **Plano Orchestration** - Central LLM gateway managing all agent calls without config changes  
✅ **Model Aliases** - Semantic routing (`risk_fast`, `risk_reasoning`) for cost/quality optimization  
✅ **Security Guardrails** - PII redaction and prompt injection detection via HTTP filters  
✅ **Full Observability** - OpenTelemetry traces showing every agent execution in Jaeger  
✅ **Production Patterns** - Error handling, fallbacks, health checks, structured logging  
✅ **Context Passing** - Agents build on each other's work through sequential task dependencies  
✅ **Backward Compatibility** - OpenAI-compatible API maintained throughout  

### Key Metrics

- **4 LLM calls** per risk assessment (1x gpt-4o-mini + 3x gpt-4o)
- **8-15 second** response time (sequential agent execution)
- **~$0.02-0.05** cost per request
- **Zero config changes** to Plano (everything already supported!)
- **100% trace visibility** across all services

### Documentation

- **This README** - Quick start and API reference
- **CREWAI_INTEGRATION.md** - Deep dive into CrewAI implementation (500+ lines)
- **CREWAI_CHECKLIST.md** - Testing and verification guide
- **IMPLEMENTATION_SUMMARY.md** - What changed and why

## License

This is a demo project for educational purposes.

## Support

For issues with Plano, see: https://docs.planoai.dev

---

**Last Updated:** January 2026  
**Version:** 0.2.0 - CrewAI Multi-Agent Integration  
**Status:** Production-ready demo with full CrewAI implementation
