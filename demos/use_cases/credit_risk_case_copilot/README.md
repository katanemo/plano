# Credit Risk Case Copilot

A production-ready multi-agent credit risk assessment system demonstrating Plano's intelligent orchestration, guardrails, and prompt targets. This demo showcases a sophisticated workflow that analyzes loan applications, performs policy compliance checks, generates decision memos, and creates cases with full observability.

## Overview

This demo implements a **Credit Risk Case Copilot** with:

- **Risk Crew Agent** - Multi-agent workflow for comprehensive risk assessment
- **Case Service** - Case management API for storing decisions
- **PII Security Filter** - MCP filter for redacting sensitive data and detecting prompt injections
- **Streamlit UI** - Interactive web interface for risk analysts
- **Jaeger Tracing** - End-to-end distributed tracing across all services

All services communicate through **Plano's orchestrator** which handles intelligent routing, model selection, guardrails, and function calling.

## Features

- **Multi-Agent Risk Assessment**: Intake normalization, risk scoring, policy checks, and decision memo generation
- **Risk Band Classification**: LOW/MEDIUM/HIGH with confidence scores
- **Driver Analysis**: Identifies top risk factors with supporting evidence
- **Policy Compliance**: Automated checks for KYC, income verification, and lending standards
- **Document Requirements**: Auto-generated based on risk profile
- **Security Guardrails**: PII redaction (CNIC, phone, email) and prompt injection detection
- **Case Management**: Create and track risk cases with audit trails
- **OpenTelemetry Tracing**: Full observability across UI → Plano → Agents → LLMs → APIs

## Architecture

```
Streamlit UI (8501)
      ↓
Plano Orchestrator (8001)
      ↓
PII Filter (10550) → Risk Crew Agent (10530) → Plano LLM Gateway (12000)
                           ↓
                    Case Service (10540)
                           ↓
                    Jaeger (16686)
```

## Prerequisites

- Docker and Docker Compose
- [Plano CLI](https://docs.planoai.dev) installed (`pip install planoai` or `uvx planoai`)
- OpenAI API key

## Quick Start

### 1. Set Environment Variables

Copy the example environment file and add your API key:

```bash
cp .env.example .env
# Edit .env and add your OPENAI_API_KEY
```

Or export directly:

```bash
export OPENAI_API_KEY="your-openai-api-key"
```

### 2. Start Docker Services

Start all containerized services (agents, UI, Jaeger):

```bash
docker compose up --build
```

This starts:
- **Risk Crew Agent** on port 10530
- **Case Service** on port 10540
- **PII Filter** on port 10550
- **Streamlit UI** on port 8501
- **Jaeger** on port 16686

### 3. Start Plano Orchestrator

In a new terminal, start Plano (runs on host, not in Docker):

```bash
cd /path/to/credit_risk_case_copilot
planoai up config.yaml

# Or if installed with uv:
# uvx planoai up config.yaml
```

The orchestrator will start on:
- Port **8001** - Agent listener (main entry point)
- Port **12000** - LLM gateway (for agents to call)
- Port **10000** - Prompt listener (for function calling)

### 4. Access the UI

Open your browser to:

- **Streamlit UI**: http://localhost:8501
- **Jaeger Tracing**: http://localhost:16686

## Using the Demo

### Streamlit UI Workflow

1. **Select a Scenario** (or paste your own JSON):
   - 🟢 **Scenario A** - Low risk (stable job, good credit, low DTI)
   - 🟡 **Scenario B** - Medium risk (thin file, missing verifications)
   - 🔴 **Scenario C** - High risk + prompt injection attempt

2. **Click "Assess Risk"** - Plano routes to Risk Crew Agent

3. **View Results** in tabs:
   - **Risk Summary**: Normalized data and overview
   - **Risk Drivers**: Top factors with evidence
   - **Policy & Compliance**: Checks, exceptions, required documents
   - **Decision Memo**: Bank-ready memo with recommendation
   - **Audit Trail**: Models used, timestamps, request ID

4. **Create Case** - Stores assessment in Case Service

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

### Risk Crew Agent (Port 10530)

Multi-step workflow:
1. **Intake & Normalization** - Extract and validate data
2. **Risk Scoring** - Calculate DTI, assess credit, classify band
3. **Policy Checks** - Verify KYC, income, address, lending limits
4. **Decision Memo** - Generate bank-ready recommendation

### Case Service (Port 10540)

RESTful API for case management:
- `POST /cases` - Create new case
- `GET /cases/{case_id}` - Retrieve case
- `GET /cases` - List all cases
- `GET /health` - Health check

### PII Security Filter (Port 10550)

MCP filter that:
- Redacts CNIC patterns (12345-6789012-3)
- Redacts phone numbers (+923001234567)
- Redacts email addresses
- Detects prompt injections ("ignore policy", "bypass checks", etc.)
- Adds security warnings to flagged content

## Configuration Files

### config.yaml (Plano Configuration)

- **Agents**: `risk_crew_agent` with rich description for routing
- **Filters**: `pii_security_filter` in filter chain
- **Model Providers**: OpenAI GPT-4o and GPT-4o-mini
- **Model Aliases**: `risk_fast` (mini), `risk_reasoning` (4o)
- **Prompt Targets**: `create_risk_case` → Case Service API
- **Listeners**: agent (8001), model (12000), prompt (10000)
- **Tracing**: 100% sampling to Jaeger

### docker-compose.yaml

Orchestrates 5 services:
- `risk-crew-agent` - Risk assessment engine
- `case-service` - Case management
- `pii-filter` - Security filter
- `streamlit-ui` - Web interface
- `jaeger` - Tracing backend

## Observability

### Jaeger Tracing

View distributed traces at http://localhost:16686

Trace flow:
1. UI sends request to Plano
2. Plano applies PII filter
3. Plano routes to Risk Crew Agent
4. Agent calls Plano LLM Gateway
5. Agent returns assessment
6. (Optional) Prompt target calls Case Service

Search for:
- Service: `risk-crew-agent`
- Operation: `chat_completions`
- Tags: `request_id`, `risk_band`, `recommended_action`

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
        ├── case_service.py          # Case management API (FastAPI)
        ├── pii_filter.py            # MCP security filter (FastAPI)
        └── ui_streamlit.py          # Web UI (Streamlit)
```

## Development

### Running Services Individually

```bash
# Risk Crew Agent
uv run python src/credit_risk_demo/risk_crew_agent.py

# Case Service
uv run python src/credit_risk_demo/case_service.py

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
- Verify ports are available: `lsof -i :8001,10530,10540,10550,8501,16686`
- Check logs: `docker compose logs -f`

**Plano won't start**
- Verify installation: `planoai --version`
- Check config: `planoai validate config.yaml`
- Ensure OPENAI_API_KEY is set

**No response from agents**
- Verify all services are healthy:
  - `curl http://localhost:10530/health`
  - `curl http://localhost:10540/health`
  - `curl http://localhost:10550/health`
- Check Plano is running: `curl http://localhost:8001/health` (if health endpoint exists)

**Streamlit can't connect**
- Verify PLANO_ENDPOINT in docker-compose matches Plano port
- Check `host.docker.internal` resolves (should point to host machine)

**Jaeger shows no traces**
- Verify OTLP_ENDPOINT in services points to Jaeger
- Check Jaeger is running: `docker compose ps jaeger`
- Allow a few seconds for traces to appear

## API Endpoints

### Plano Orchestrator (8001)
- `POST /v1/chat/completions` - Main entry point (OpenAI-compatible)

### Risk Crew Agent (10530)
- `POST /v1/chat/completions` - Risk assessment endpoint
- `GET /health` - Health check

### Case Service (10540)
- `POST /cases` - Create case
- `GET /cases/{case_id}` - Get case
- `GET /cases` - List cases
- `GET /health` - Health check

### PII Filter (10550)
- `POST /v1/tools/pii_security_filter` - MCP filter endpoint
- `GET /health` - Health check

## Next Steps

- Add database persistence for case storage (PostgreSQL)
- Implement full CrewAI integration with actual agent execution
- Add more sophisticated risk models and policy rules
- Connect to real credit bureau APIs
- Implement user authentication and RBAC
- Add email notifications for case creation
- Build analytics dashboard for risk metrics

## License

This is a demo project for educational purposes.

## Support

For issues with Plano, see: https://docs.planoai.dev
