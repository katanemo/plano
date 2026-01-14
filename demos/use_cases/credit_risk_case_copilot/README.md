# Credit Risk Case Copilot

A demo multi-agent credit risk assessment system demonstrating Plano's intelligent orchestration, guardrails, and prompt targets. This demo showcases a sophisticated workflow that analyzes loan applications, performs policy compliance checks, generates decision memos, and creates cases with full observability.

## 🤖 CrewAI Multi-Agent System

This demo uses **actual CrewAI execution** with 4 specialized AI agents working sequentially through Plano's LLM gateway:

### Agent Workflow

```
Loan Application JSON
    ↓
Agent 1: Intake & Normalization (risk_fast/gpt-4o-mini) → 1-2s
    ↓
Agent 2: Risk Scoring & Drivers (risk_reasoning/gpt-4o) → 2-3s
    ↓
Agent 3: Policy & Compliance (risk_reasoning/gpt-4o) → 2-3s
    ↓
Agent 4: Decision Memo & Action (risk_reasoning/gpt-4o) → 2-4s
    ↓
Complete Risk Assessment (Total: 8-15 seconds)
```

### Key Implementation Details

**LLM Configuration:**
```python
# All agents use Plano's gateway with model aliases
llm_fast = ChatOpenAI(
    base_url="http://host.docker.internal:12000/v1",
    model="risk_fast",      # → gpt-4o-mini
)
llm_reasoning = ChatOpenAI(
    base_url="http://host.docker.internal:12000/v1", 
    model="risk_reasoning", # → gpt-4o
)
```

**Performance:**
- Response time: 8-15 seconds (4 sequential LLM calls)
- Cost per request: ~$0.02-0.05
- Quality: Enhanced analysis vs deterministic logic
- Observability: Full traces in Jaeger showing each agent execution

**Why No Plano Config Changes:**
The existing `config.yaml` already had everything needed:
- ✅ Model aliases (`risk_fast`, `risk_reasoning`) 
- ✅ LLM gateway on port 12000
- ✅ OpenTelemetry tracing enabled
- ✅ Agent routing configured

**Dependencies Added:**
- `crewai>=0.80.0` - Multi-agent framework
- `crewai-tools>=0.12.0` - Agent tools
- `langchain-openai>=0.1.0` - LLM integration with Plano

## Overview

This demo implements a **Credit Risk Case Copilot** with:

- **Risk Crew Agent** - Multi-agent workflow for comprehensive risk assessment
- **Case Service** - Case management API for storing decisions
- **PII Security Filter** - MCP filter for redacting sensitive data and detecting prompt injections
- **Streamlit UI** - Interactive web interface for risk analysts
- **Jaeger Tracing** - End-to-end distributed tracing across all services

All services communicate through **Plano's orchestrator** which handles intelligent routing, model selection, guardrails, and function calling.

## Features

- **CrewAI Multi-Agent Workflow**: 4 specialized agents executing sequentially with context passing
- **Risk Band Classification**: LOW/MEDIUM/HIGH with confidence scores and evidence-based drivers
- **Policy Compliance**: Automated KYC, income verification, and lending standard checks
- **Decision Memos**: Bank-ready recommendations (APPROVE/CONDITIONAL/REFER/REJECT)
- **Security Guardrails**: PII redaction (CNIC, phone, email) and prompt injection detection
- **Case Management**: Create and track risk cases with full audit trails
- **Full Observability**: OpenTelemetry traces showing all 4 agent executions in Jaeger
- **Model Optimization**: Uses `risk_fast` (gpt-4o-mini) for extraction, `risk_reasoning` (gpt-4o) for analysis
- **Plano Integration**: All LLM calls through centralized gateway for unified management

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

### Risk Crew Agent (Port 10530) - CrewAI Multi-Agent System

Implements a 4-agent CrewAI workflow where each agent is specialized:

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

**Context Passing:** Each agent builds on the previous agent's output for comprehensive analysis.

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
6. (Optional) Prompt target calls Case Service (10540)
7. All spans visible in Jaeger (16686)

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
  - `curl http://localhost:10540/health`
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

## Next Steps & Extensions

### Immediate Enhancements
- Add database persistence for case storage (PostgreSQL/MongoDB)
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
- Add email/SMS notifications for case creation
- Implement batch processing API for multiple applications
- Create PDF export for decision memos
- Add A/B testing framework for different risk models

## What This Demo Demonstrates

This project showcases:

✅ **True Multi-Agent AI System** - 4 specialized CrewAI agents with distinct roles and expertise  
✅ **Plano Orchestration** - Central LLM gateway managing all agent calls without config changes  
✅ **Model Aliases** - Semantic routing (`risk_fast`, `risk_reasoning`) for cost/quality optimization  
✅ **Security Guardrails** - PII redaction and prompt injection detection via MCP filters  
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
