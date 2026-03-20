import json
import logging
import os
import uuid
from datetime import datetime
from typing import Any, Dict, Optional

import uvicorn
from crewai import Agent, Crew, Task, Process
from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse
from langchain_openai import ChatOpenAI
from opentelemetry import trace
from opentelemetry.exporter.otlp.proto.http.trace_exporter import OTLPSpanExporter
from opentelemetry.instrumentation.fastapi import FastAPIInstrumentor
from opentelemetry.sdk.resources import Resource
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import BatchSpanProcessor

# Logging configuration
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - [RISK_CREW_AGENT] - %(levelname)s - %(message)s",
)
logger = logging.getLogger(__name__)

# Configuration
LLM_GATEWAY_ENDPOINT = os.getenv(
    "LLM_GATEWAY_ENDPOINT", "http://host.docker.internal:12000/v1"
)
OTLP_ENDPOINT = os.getenv("OTLP_ENDPOINT", "http://jaeger:4318/v1/traces")

# OpenTelemetry setup
resource = Resource.create({"service.name": "risk-crew-agent"})
tracer_provider = TracerProvider(resource=resource)
otlp_exporter = OTLPSpanExporter(endpoint=OTLP_ENDPOINT)
tracer_provider.add_span_processor(BatchSpanProcessor(otlp_exporter))
trace.set_tracer_provider(tracer_provider)
tracer = trace.get_tracer(__name__)

# FastAPI app
app = FastAPI(title="Credit Risk Crew Agent", version="1.0.0")
FastAPIInstrumentor.instrument_app(app)

# Configure LLMs to use Plano's gateway with model aliases
llm_fast = ChatOpenAI(
    base_url=LLM_GATEWAY_ENDPOINT,
    model="openai/gpt-4o-mini",  # alias not working
    api_key="EMPTY",
    temperature=0.1,
    max_tokens=1500,
)

llm_reasoning = ChatOpenAI(
    base_url=LLM_GATEWAY_ENDPOINT,
    model="openai/gpt-4o",  # alias not working
    api_key="EMPTY",
    temperature=0.7,
    max_tokens=2000,
)


def build_intake_agent() -> Agent:
    """Build the intake & normalization agent."""
    return Agent(
        role="Loan Intake & Normalization Specialist",
        goal="Extract, validate, and normalize loan application data for downstream risk assessment",
        backstory="""You are an expert at processing loan applications from various sources. 
        You extract all relevant information, identify missing data points, normalize values 
        (e.g., calculate DTI if possible), and flag data quality issues. You prepare a clean, 
        structured dataset for the risk analysts.""",
        llm=llm_fast,  # Use faster model for data extraction
        verbose=True,
        allow_delegation=False,
    )


def build_risk_agent() -> Agent:
    """Build the risk scoring & driver analysis agent."""
    return Agent(
        role="Risk Scoring & Driver Analysis Expert",
        goal="Calculate comprehensive risk scores and identify key risk drivers with evidence",
        backstory="""You are a senior credit risk analyst with 15+ years experience. You analyze:
        - Debt-to-income ratios and payment capacity
        - Credit utilization and credit history
        - Delinquency patterns and payment history
        - Employment stability and income verification
        - Credit score ranges and trends
        
        You classify applications into risk bands (LOW/MEDIUM/HIGH) and identify the top 3 risk 
        drivers with specific evidence from the application data.""",
        llm=llm_reasoning,  # Use reasoning model for analysis
        verbose=True,
        allow_delegation=False,
    )


def build_policy_agent() -> Agent:
    """Build the policy & compliance agent."""
    return Agent(
        role="Policy & Compliance Officer",
        goal="Verify compliance with lending policies and identify exceptions",
        backstory="""You are a compliance expert ensuring all loan applications meet regulatory 
        and internal policy requirements. You check:
        - KYC completion (CNIC, phone, address)
        - Income and address verification status
        - Debt-to-income limits (reject if >60%)
        - Minimum credit score thresholds (reject if <500)
        - Recent delinquency patterns
        
        You identify required documents based on risk profile and flag any policy exceptions.""",
        llm=llm_reasoning,
        verbose=True,
        allow_delegation=False,
    )


def build_memo_agent() -> Agent:
    """Build the decision memo & action agent."""
    return Agent(
        role="Decision Memo & Action Specialist",
        goal="Generate bank-ready decision memos and recommend clear actions",
        backstory="""You are a senior credit officer who writes clear, concise decision memos 
        for loan committees. You synthesize:
        - Risk assessment findings
        - Policy compliance status
        - Required documentation
        - Evidence-based recommendations
        
        You recommend actions: APPROVE (low risk + compliant), CONDITIONAL_APPROVE (minor issues), 
        REFER (manual review needed), or REJECT (high risk/major violations).""",
        llm=llm_reasoning,
        verbose=True,
        allow_delegation=False,
    )


def make_intake_task(application_data: Dict[str, Any], agent: Agent) -> Task:
    """Build the intake task prompt."""
    return Task(
        description=f"""Analyze this loan application and extract all relevant information:

        {json.dumps(application_data, indent=2)}

        Extract and normalize:
        1. Applicant name and loan amount
        2. Monthly income and employment status
        3. Credit score and existing loans
        4. Total debt and delinquencies
        5. Credit utilization rate
        6. KYC, income verification, and address verification status
        7. Calculate DTI if income is available: (total_debt / monthly_income) * 100
        8. Flag any missing critical fields

        Output JSON only with:
        - step: "intake"
        - normalized_data: object of normalized fields
        - missing_fields: list of missing critical fields""",
        agent=agent,
        expected_output="JSON only with normalized data and missing fields",
    )


def make_risk_task(payload: Dict[str, Any], agent: Agent) -> Task:
    """Build the risk scoring task prompt."""
    return Task(
        description=f"""You are given an input payload that includes the application and intake output:

        {json.dumps(payload, indent=2)}

        Use intake.normalized_data for your analysis.

        **Risk Scoring Criteria:**
        1. **Credit Score Assessment:**
           - Excellent (≥750): Low risk
           - Good (650-749): Medium risk
           - Fair (550-649): High risk
           - Poor (<550): Critical risk
           - Missing: Medium risk (thin file)

        2. **Debt-to-Income Ratio:**
           - <35%: Low risk
           - 35-50%: Medium risk
           - >50%: Critical risk
           - Missing: High risk

        3. **Delinquency History:**
           - 0: Low risk
           - 1-2: Medium risk
           - >2: Critical risk

        4. **Credit Utilization:**
           - <30%: Low risk
           - 30-70%: Medium risk
           - >70%: High risk

        Output JSON only with:
        - step: "risk"
        - risk_band: LOW|MEDIUM|HIGH
        - confidence_score: 0.0-1.0
        - top_3_risk_drivers: [{{
            "factor": string,
            "impact": CRITICAL|HIGH|MEDIUM|LOW,
            "evidence": string
          }}]""",
        agent=agent,
        expected_output="JSON only with risk band, confidence, and top drivers",
    )


def make_policy_task(payload: Dict[str, Any], agent: Agent) -> Task:
    """Build the policy compliance task prompt."""
    return Task(
        description=f"""You are given an input payload that includes the application, intake, and risk output:

        {json.dumps(payload, indent=2)}

        Use intake.normalized_data and risk outputs.

        **Policy Checks:**
        1. KYC Completion: Check if CNIC, phone, and address are provided
        2. Income Verification: Check if income is verified
        3. Address Verification: Check if address is verified
        4. DTI Limit: Flag if DTI >60% (automatic reject threshold)
        5. Credit Score: Flag if <500 (minimum acceptable)
        6. Delinquencies: Flag if >2 in recent history

        **Required Documents by Risk Band:**
        - LOW: Valid CNIC, Credit Report, Employment Letter, Bank Statements (3 months)
        - MEDIUM: + Income proof (6 months), Address proof, Tax Returns (2 years)
        - HIGH: + Guarantor Documents, Collateral Valuation, Detailed Financials

        Output JSON only with:
        - step: "policy"
        - policy_checks: [{{"check": string, "status": PASS|FAIL|WARNING, "details": string}}]
        - exceptions: [string]
        - required_documents: [string]""",
        agent=agent,
        expected_output="JSON only with policy checks, exceptions, and required documents",
    )


def make_memo_task(payload: Dict[str, Any], agent: Agent) -> Task:
    """Build the decision memo task prompt."""
    return Task(
        description=f"""You are given an input payload that includes the application, intake, risk, and policy output:

        {json.dumps(payload, indent=2)}

        Generate a concise memo and recommendation.

        **Recommendation Rules:**
        - APPROVE: LOW risk + all checks passed
        - CONDITIONAL_APPROVE: LOW/MEDIUM risk + minor issues (collect docs)
        - REFER: MEDIUM/HIGH risk + exceptions (manual review)
        - REJECT: HIGH risk OR critical policy violations (>60% DTI, <500 credit score)

        Output JSON only with:
        - step: "memo"
        - recommended_action: APPROVE|CONDITIONAL_APPROVE|REFER|REJECT
        - decision_memo: string (max 300 words)""",
        agent=agent,
        expected_output="JSON only with recommended action and decision memo",
    )


def run_single_step(agent: Agent, task: Task) -> str:
    """Run a single-step CrewAI workflow and return the output."""
    crew = Crew(
        agents=[agent],
        tasks=[task],
        process=Process.sequential,
        verbose=True,
    )
    return crew.kickoff()


def extract_json_from_content(content: str) -> Optional[Dict[str, Any]]:
    """Extract a JSON object from a message content string."""
    try:
        if "{" in content and "}" in content:
            json_start = content.index("{")
            json_end = content.rindex("}") + 1
            json_str = content[json_start:json_end]
            return json.loads(json_str)
    except Exception as e:
        logger.warning(f"Could not parse JSON from message: {e}")
    return None


def extract_json_block(output_text: str) -> Optional[Dict[str, Any]]:
    """Extract the first JSON object or fenced JSON block from output text."""
    try:
        if "```json" in output_text:
            json_start = output_text.index("```json") + 7
            json_end = output_text.index("```", json_start)
            return json.loads(output_text[json_start:json_end].strip())
        if "{" in output_text and "}" in output_text:
            json_start = output_text.index("{")
            json_end = output_text.rindex("}") + 1
            return json.loads(output_text[json_start:json_end])
    except Exception as e:
        logger.warning(f"Could not parse JSON from output: {e}")
    return None


def extract_step_outputs(messages: list) -> Dict[str, Dict[str, Any]]:
    """Extract step outputs from assistant messages."""
    step_outputs: Dict[str, Dict[str, Any]] = {}
    for message in messages:
        content = message.get("content", "")
        if not content:
            continue
        json_block = extract_json_block(content)
        if isinstance(json_block, dict) and json_block.get("step"):
            step_outputs[json_block["step"]] = json_block
    return step_outputs


def extract_application_from_messages(messages: list) -> Optional[Dict[str, Any]]:
    """Extract the raw application JSON from the latest user message."""
    for message in reversed(messages):
        if message.get("role") != "user":
            continue
        content = message.get("content", "")
        json_block = extract_json_from_content(content)
        if isinstance(json_block, dict):
            if "application" in json_block and isinstance(json_block["application"], dict):
                return json_block["application"]
            if "step" not in json_block:
                return json_block
    return None


async def handle_single_agent_step(request: Request, step: str) -> JSONResponse:
    """Handle a single-step agent request with OpenAI-compatible response."""
    with tracer.start_as_current_span(f"{step}_chat_completions") as span:
        try:
            body = await request.json()
            messages = body.get("messages", [])
            request_id = str(uuid.uuid4())

            span.set_attribute("request_id", request_id)
            span.set_attribute("step", step)

            application_data = extract_application_from_messages(messages)
            step_outputs = extract_step_outputs(messages)
            logger.info(f"Processing {step} request {request_id}")

            if step == "intake" and not application_data:
                return JSONResponse(
                    status_code=400,
                    content={"error": "No application JSON found in user messages"},
                )

            if step == "intake":
                agent = build_intake_agent()
                task = make_intake_task(application_data, agent)
                model_name = "loan_intake_agent"
                human_response = "Intake normalization complete. Passing to the next agent."
            elif step == "risk":
                intake_output = step_outputs.get("intake")
                if not intake_output:
                    return JSONResponse(
                        status_code=400,
                        content={"error": "Missing intake output for risk step"},
                    )
                payload = {
                    "application": application_data or {},
                    "intake": intake_output,
                }
                agent = build_risk_agent()
                task = make_risk_task(payload, agent)
                model_name = "risk_scoring_agent"
                human_response = "Risk scoring complete. Passing to the next agent."
            elif step == "policy":
                intake_output = step_outputs.get("intake")
                risk_output = step_outputs.get("risk")
                if not intake_output or not risk_output:
                    return JSONResponse(
                        status_code=400,
                        content={"error": "Missing intake or risk output for policy step"},
                    )
                payload = {
                    "application": application_data or {},
                    "intake": intake_output,
                    "risk": risk_output,
                }
                agent = build_policy_agent()
                task = make_policy_task(payload, agent)
                model_name = "policy_compliance_agent"
                human_response = "Policy compliance review complete. Passing to the next agent."
            elif step == "memo":
                intake_output = step_outputs.get("intake")
                risk_output = step_outputs.get("risk")
                policy_output = step_outputs.get("policy")
                if not intake_output or not risk_output or not policy_output:
                    return JSONResponse(
                        status_code=400,
                        content={"error": "Missing prior outputs for memo step"},
                    )
                payload = {
                    "application": application_data or {},
                    "intake": intake_output,
                    "risk": risk_output,
                    "policy": policy_output,
                }
                agent = build_memo_agent()
                task = make_memo_task(payload, agent)
                model_name = "decision_memo_agent"
                human_response = "Decision memo complete."
            else:
                return JSONResponse(
                    status_code=400, content={"error": f"Unknown step: {step}"}
                )

            crew_output = run_single_step(agent, task)
            json_payload = extract_json_block(str(crew_output)) or {"step": step}

            if step == "memo":
                decision_memo = json_payload.get("decision_memo")
                if decision_memo:
                    human_response = decision_memo

            response_content = (
                f"{human_response}\n\n```json\n{json.dumps(json_payload, indent=2)}\n```"
            )

            return JSONResponse(
                content={
                    "id": f"chatcmpl-{request_id}",
                    "object": "chat.completion",
                    "created": int(datetime.utcnow().timestamp()),
                    "model": model_name,
                    "choices": [
                        {
                            "index": 0,
                            "message": {
                                "role": "assistant",
                                "content": response_content,
                            },
                            "finish_reason": "stop",
                        }
                    ],
                    "usage": {
                        "prompt_tokens": 0,
                        "completion_tokens": 0,
                        "total_tokens": 0,
                    },
                    "metadata": {
                        "framework": "CrewAI",
                        "step": step,
                        "request_id": request_id,
                    },
                }
            )

        except Exception as e:
            logger.error(f"Error processing {step} request: {e}", exc_info=True)
            span.record_exception(e)
            return JSONResponse(
                status_code=500, content={"error": str(e), "framework": "CrewAI"}
            )


@app.post("/v1/agents/intake/chat/completions")
async def intake_chat_completions(request: Request):
    return await handle_single_agent_step(request, "intake")


@app.post("/v1/agents/risk/chat/completions")
async def risk_chat_completions(request: Request):
    return await handle_single_agent_step(request, "risk")


@app.post("/v1/agents/policy/chat/completions")
async def policy_chat_completions(request: Request):
    return await handle_single_agent_step(request, "policy")


@app.post("/v1/agents/memo/chat/completions")
async def memo_chat_completions(request: Request):
    return await handle_single_agent_step(request, "memo")


@app.get("/health")
async def health_check():
    """Health check endpoint."""
    return {
        "status": "healthy",
        "service": "risk-crew-agent",
        "framework": "CrewAI",
        "llm_gateway": LLM_GATEWAY_ENDPOINT,
        "agents": 4,
    }


if __name__ == "__main__":
    logger.info("Starting Risk Crew Agent with CrewAI on port 10530")
    logger.info(f"LLM Gateway: {LLM_GATEWAY_ENDPOINT}")
    logger.info("Agents: Intake → Risk Scoring → Policy → Decision Memo")
    uvicorn.run(app, host="0.0.0.0", port=10530)
