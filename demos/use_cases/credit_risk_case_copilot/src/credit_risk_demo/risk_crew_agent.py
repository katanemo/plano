import json
import logging
import os
import uuid
from datetime import datetime
from typing import Any, Dict, List, Optional

import uvicorn
from crewai import Agent, Crew, Task, Process
from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse
from langchain_openai import ChatOpenAI
from opentelemetry import trace
from opentelemetry.exporter.otlp.proto.http.trace_exporter import OTLPSpanExporter
from opentelemetry.instrumentation.fastapi import FastAPIInstrumentor
from opentelemetry.propagate import extract
from opentelemetry.sdk.resources import Resource
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import BatchSpanProcessor
from pydantic import BaseModel

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


class RiskAssessmentResult(BaseModel):
    request_id: str
    normalized_application: Dict[str, Any]
    risk_band: str
    confidence: float
    drivers: List[Dict[str, Any]]
    policy_checks: List[Dict[str, str]]
    exceptions: List[str]
    required_documents: List[str]
    recommended_action: str
    decision_memo: str
    audit_trail: Dict[str, Any]
    human_response: str


def create_risk_crew(application_data: Dict[str, Any]) -> Crew:
    """Create a CrewAI crew for risk assessment with 4 specialized agents."""

    # Agent 1: Intake & Normalization Specialist
    intake_agent = Agent(
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

    # Agent 2: Risk Scoring & Driver Analysis Expert
    risk_scoring_agent = Agent(
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

    # Agent 3: Policy & Compliance Officer
    policy_agent = Agent(
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

    # Agent 4: Decision Memo & Action Specialist
    memo_agent = Agent(
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

    # Task 1: Intake and normalization
    intake_task = Task(
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

        Output a JSON structure with normalized values and a missing_fields list.""",
        agent=intake_agent,
        expected_output="Normalized application data with missing field analysis in JSON format",
    )

    # Task 2: Risk scoring
    risk_task = Task(
        description="""Based on the normalized data from the Intake Specialist, perform risk assessment:

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

        **Output:**
        - Risk band: LOW (excellent profile), MEDIUM (some concerns), HIGH (significant issues)
        - Confidence score (0.0-1.0)
        - Top 3 risk drivers with: factor name, impact level (CRITICAL/HIGH/MEDIUM/LOW), evidence

        Provide your analysis in JSON format.""",
        agent=risk_scoring_agent,
        expected_output="Risk band classification with confidence score and top 3 drivers in JSON format",
        context=[intake_task],
    )

    # Task 3: Policy checks
    policy_task = Task(
        description="""Verify policy compliance using the normalized data and risk assessment:

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

        **Output JSON:**
        - policy_checks: [{check, status (PASS/FAIL/WARNING), details}]
        - exceptions: [list of exception codes like "KYC_INCOMPLETE", "INCOME_NOT_VERIFIED"]
        - required_documents: [list of document names]""",
        agent=policy_agent,
        expected_output="Policy compliance status with exceptions and required documents in JSON format",
        context=[intake_task, risk_task],
    )

    # Task 4: Decision memo
    memo_task = Task(
        description="""Generate a bank-ready decision memo synthesizing all findings:

        **Memo Structure:**
        1. **Executive Summary** (2-3 sentences)
           - Loan amount, applicant, risk band, recommendation

        2. **Applicant Profile**
           - Name, loan amount, credit score, monthly income

        3. **Risk Assessment**
           - Risk band and confidence
           - Top risk drivers with evidence

        4. **Policy Compliance**
           - Number of checks passed
           - Outstanding issues or exceptions

        5. **Required Documents**
           - List key documents needed

        6. **Recommendation**
           - APPROVE: LOW risk + all checks passed
           - CONDITIONAL_APPROVE: LOW/MEDIUM risk + minor issues (collect docs)
           - REFER: MEDIUM/HIGH risk + exceptions (manual review)
           - REJECT: HIGH risk OR critical policy violations (>60% DTI, <500 credit score)

        7. **Next Steps**
           - Action items based on recommendation

        Keep professional, concise, and actionable. Max 300 words.

        **Also provide:**
        - recommended_action: One of APPROVE/CONDITIONAL_APPROVE/REFER/REJECT
        - decision_memo: Full memo text""",
        agent=memo_agent,
        expected_output="Professional decision memo with clear recommendation",
        context=[intake_task, risk_task, policy_task],
    )

    # Create crew with sequential process
    crew = Crew(
        agents=[intake_agent, risk_scoring_agent, policy_agent, memo_agent],
        tasks=[intake_task, risk_task, policy_task, memo_task],
        process=Process.sequential,
        verbose=True,
    )

    return crew


def parse_crew_output(crew_output: str, application_data: Dict) -> Dict[str, Any]:
    """Parse CrewAI output and extract structured data."""

    # Initialize result structure
    result = {
        "normalized_application": {},
        "risk_band": "MEDIUM",
        "confidence": 0.75,
        "drivers": [],
        "policy_checks": [],
        "exceptions": [],
        "required_documents": [],
        "recommended_action": "REFER",
        "decision_memo": "",
    }

    try:
        # CrewAI returns the final task output as a string
        output_text = str(crew_output)

        # Try to extract JSON blocks from the output
        json_blocks = []
        lines = output_text.split("\n")
        in_json = False
        current_json = []

        for line in lines:
            if "```json" in line or line.strip().startswith("{"):
                in_json = True
                if not line.strip().startswith("```"):
                    current_json.append(line)
            elif "```" in line and in_json:
                in_json = False
                if current_json:
                    try:
                        json_obj = json.loads("\n".join(current_json))
                        json_blocks.append(json_obj)
                    except:
                        pass
                    current_json = []
            elif in_json:
                current_json.append(line)

        # Extract from JSON blocks if available
        for block in json_blocks:
            if "risk_band" in block:
                result["risk_band"] = block.get("risk_band", result["risk_band"])
            if "confidence" in block:
                result["confidence"] = float(
                    block.get("confidence", result["confidence"])
                )
            if "drivers" in block:
                result["drivers"] = block.get("drivers", result["drivers"])
            if "policy_checks" in block:
                result["policy_checks"] = block.get(
                    "policy_checks", result["policy_checks"]
                )
            if "exceptions" in block:
                result["exceptions"] = block.get("exceptions", result["exceptions"])
            if "required_documents" in block:
                result["required_documents"] = block.get(
                    "required_documents", result["required_documents"]
                )
            if "recommended_action" in block:
                result["recommended_action"] = block.get(
                    "recommended_action", result["recommended_action"]
                )

        # Extract decision memo from text
        if (
            "**CREDIT RISK DECISION MEMO**" in output_text
            or "Executive Summary" in output_text
        ):
            memo_start = output_text.find("**CREDIT RISK DECISION MEMO**")
            if memo_start == -1:
                memo_start = output_text.find("Executive Summary")
            if memo_start != -1:
                result["decision_memo"] = output_text[memo_start:].strip()
        else:
            result["decision_memo"] = output_text

        # Normalize application data
        result["normalized_application"] = {
            "applicant_name": application_data.get("applicant_name", "Unknown"),
            "loan_amount": application_data.get("loan_amount", 0),
            "monthly_income": application_data.get("monthly_income"),
            "credit_score": application_data.get("credit_score"),
            "employment_status": application_data.get("employment_status"),
            "total_debt": application_data.get("total_debt", 0),
            "delinquencies": application_data.get("delinquencies", 0),
            "utilization_rate": application_data.get("utilization_rate"),
        }

    except Exception as e:
        logger.error(f"Error parsing crew output: {e}")
        # Fall back to basic extraction
        result["decision_memo"] = str(crew_output)

    return result


async def run_risk_assessment_with_crew(
    application_data: Dict[str, Any], request_id: str, trace_context: dict
) -> RiskAssessmentResult:
    """Run CrewAI workflow for risk assessment."""

    with tracer.start_as_current_span("crewai_risk_assessment_workflow") as span:
        span.set_attribute("request_id", request_id)
        span.set_attribute(
            "applicant_name", application_data.get("applicant_name", "Unknown")
        )

        logger.info(f"Starting CrewAI risk assessment for request {request_id}")

        try:
            # Create and execute crew
            crew = create_risk_crew(application_data)

            # Run the crew - this will execute all tasks sequentially
            crew_result = crew.kickoff()

            logger.info(f"CrewAI workflow completed for request {request_id}")

            # Parse the crew output
            parsed_result = parse_crew_output(crew_result, application_data)

            # Build human-friendly response
            human_response = f"""**Credit Risk Assessment Complete** (Powered by CrewAI)

**Applicant:** {parsed_result['normalized_application']['applicant_name']}
**Loan Amount:** ${parsed_result['normalized_application']['loan_amount']:,.2f}
**Risk Band:** {parsed_result['risk_band']} (Confidence: {parsed_result['confidence']:.1%})

**Top Risk Drivers:**
{format_drivers(parsed_result['drivers'])}

**Policy Status:** {len(parsed_result['exceptions'])} exception(s) identified
**Required Documents:** {len(parsed_result['required_documents'])} document(s)

**Recommendation:** {parsed_result['recommended_action']}

*Assessment performed by 4-agent CrewAI workflow: Intake → Risk Scoring → Policy → Decision Memo*"""

            return RiskAssessmentResult(
                request_id=request_id,
                normalized_application=parsed_result["normalized_application"],
                risk_band=parsed_result["risk_band"],
                confidence=parsed_result["confidence"],
                drivers=(
                    parsed_result["drivers"]
                    if parsed_result["drivers"]
                    else [
                        {
                            "factor": "Analysis in Progress",
                            "impact": "MEDIUM",
                            "evidence": "CrewAI assessment completed",
                        }
                    ]
                ),
                policy_checks=(
                    parsed_result["policy_checks"]
                    if parsed_result["policy_checks"]
                    else [
                        {
                            "check": "Comprehensive Review",
                            "status": "COMPLETED",
                            "details": "Multi-agent analysis performed",
                        }
                    ]
                ),
                exceptions=parsed_result["exceptions"],
                required_documents=(
                    parsed_result["required_documents"]
                    if parsed_result["required_documents"]
                    else ["Standard loan documentation required"]
                ),
                recommended_action=parsed_result["recommended_action"],
                decision_memo=parsed_result["decision_memo"],
                audit_trail={
                    "models_used": [
                        "risk_fast (gpt-4o-mini)",
                        "risk_reasoning (gpt-4o)",
                    ],
                    "agents_executed": [
                        "intake_agent",
                        "risk_scoring_agent",
                        "policy_agent",
                        "memo_agent",
                    ],
                    "framework": "CrewAI",
                    "timestamp": datetime.utcnow().isoformat(),
                    "request_id": request_id,
                },
                human_response=human_response,
            )

        except Exception as e:
            logger.error(f"CrewAI workflow error: {e}", exc_info=True)
            span.record_exception(e)

            # Fallback to basic response
            return RiskAssessmentResult(
                request_id=request_id,
                normalized_application={
                    "applicant_name": application_data.get("applicant_name", "Unknown"),
                    "loan_amount": application_data.get("loan_amount", 0),
                },
                risk_band="MEDIUM",
                confidence=0.50,
                drivers=[
                    {"factor": "Assessment Error", "impact": "HIGH", "evidence": str(e)}
                ],
                policy_checks=[
                    {
                        "check": "System Check",
                        "status": "ERROR",
                        "details": "CrewAI workflow encountered an error",
                    }
                ],
                exceptions=["SYSTEM_ERROR"],
                required_documents=["Manual review required"],
                recommended_action="REFER",
                decision_memo=f"System encountered an error during assessment. Manual review required. Error: {str(e)}",
                audit_trail={
                    "error": str(e),
                    "timestamp": datetime.utcnow().isoformat(),
                    "request_id": request_id,
                },
                human_response=f"Assessment error occurred. Manual review required. Request ID: {request_id}",
            )


def format_drivers(drivers: List[Dict]) -> str:
    """Format drivers for display."""
    if not drivers:
        return "- Analysis in progress"

    lines = []
    for driver in drivers:
        lines.append(
            f"- **{driver.get('factor', 'Unknown')}** ({driver.get('impact', 'UNKNOWN')}): {driver.get('evidence', 'N/A')}"
        )
    return "\n".join(lines)


@app.post("/v1/chat/completions")
async def chat_completions(request: Request):
    """OpenAI-compatible chat completions endpoint powered by CrewAI."""
    with tracer.start_as_current_span("chat_completions") as span:
        try:
            body = await request.json()
            messages = body.get("messages", [])
            request_id = str(uuid.uuid4())

            span.set_attribute("request_id", request_id)

            # Extract loan application from last user message
            last_user_msg = next(
                (m for m in reversed(messages) if m.get("role") == "user"), None
            )
            if not last_user_msg:
                return JSONResponse(
                    status_code=400, content={"error": "No user message found"}
                )

            content = last_user_msg.get("content", "")
            logger.info(f"Processing CrewAI request {request_id}: {content[:100]}")

            # Try to parse JSON from content
            application_data = {}
            try:
                if "{" in content and "}" in content:
                    json_start = content.index("{")
                    json_end = content.rindex("}") + 1
                    json_str = content[json_start:json_end]
                    application_data = json.loads(json_str)
                else:
                    # Simple request without JSON
                    application_data = {
                        "applicant_name": "Sample Applicant",
                        "loan_amount": 100000,
                    }
            except Exception as e:
                logger.warning(f"Could not parse JSON from message: {e}")
                application_data = {
                    "applicant_name": "Sample Applicant",
                    "loan_amount": 100000,
                }

            # Extract trace context
            trace_context = extract(request.headers)

            # Run CrewAI risk assessment
            result = await run_risk_assessment_with_crew(
                application_data, request_id, trace_context
            )

            # Format response
            response_content = result.human_response

            # Add machine-readable data as JSON
            response_content += (
                f"\n\n```json\n{json.dumps(result.dict(), indent=2)}\n```"
            )

            # Return OpenAI-compatible response
            return JSONResponse(
                content={
                    "id": f"chatcmpl-{request_id}",
                    "object": "chat.completion",
                    "created": int(datetime.utcnow().timestamp()),
                    "model": "risk_crew_agent",
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
                        "agents_used": 4,
                        "request_id": request_id,
                    },
                }
            )

        except Exception as e:
            logger.error(f"Error processing CrewAI request: {e}", exc_info=True)
            span.record_exception(e)
            return JSONResponse(
                status_code=500, content={"error": str(e), "framework": "CrewAI"}
            )


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
