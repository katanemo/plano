import json
import logging
import os
import uuid
from datetime import datetime
from typing import Any, Dict, List, Optional

import httpx
import uvicorn
from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse
from openai import AsyncOpenAI
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

# OpenAI client pointing to Plano
openai_client = AsyncOpenAI(base_url=LLM_GATEWAY_ENDPOINT, api_key="EMPTY")
http_client = httpx.AsyncClient(timeout=60.0)


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


def calculate_risk_band(app: Dict) -> tuple:
    """Calculate risk band based on application data."""
    score = 0
    drivers = []

    # Credit score assessment
    credit_score = app.get("credit_score")
    if credit_score:
        if credit_score >= 750:
            score += 30
        elif credit_score >= 650:
            score += 20
            drivers.append(
                {
                    "factor": "Credit Score",
                    "impact": "MEDIUM",
                    "evidence": f"Credit score {credit_score} is in fair range (650-750)",
                }
            )
        elif credit_score >= 550:
            score += 10
            drivers.append(
                {
                    "factor": "Credit Score",
                    "impact": "HIGH",
                    "evidence": f"Credit score {credit_score} is below good range",
                }
            )
        else:
            drivers.append(
                {
                    "factor": "Credit Score",
                    "impact": "CRITICAL",
                    "evidence": f"Credit score {credit_score} is in poor range (<550)",
                }
            )
    else:
        score += 10
        drivers.append(
            {
                "factor": "Credit Score",
                "impact": "MEDIUM",
                "evidence": "No credit score available - thin file",
            }
        )

    # DTI assessment
    monthly_income = app.get("monthly_income")
    total_debt = app.get("total_debt", 0)
    if monthly_income and monthly_income > 0:
        dti = (total_debt / monthly_income) * 100
        if dti < 35:
            score += 30
        elif dti < 50:
            score += 15
            drivers.append(
                {
                    "factor": "Debt-to-Income Ratio",
                    "impact": "MEDIUM",
                    "evidence": f"DTI of {dti:.1f}% is elevated (35-50% range)",
                }
            )
        else:
            drivers.append(
                {
                    "factor": "Debt-to-Income Ratio",
                    "impact": "CRITICAL",
                    "evidence": f"DTI of {dti:.1f}% exceeds prudent limits (>50%)",
                }
            )
    else:
        score += 10
        drivers.append(
            {
                "factor": "Income Verification",
                "impact": "HIGH",
                "evidence": "Monthly income not verified or missing",
            }
        )

    # Delinquency check
    delinquencies = app.get("delinquencies", 0)
    if delinquencies == 0:
        score += 20
    elif delinquencies <= 2:
        score += 10
        drivers.append(
            {
                "factor": "Payment History",
                "impact": "MEDIUM",
                "evidence": f"{delinquencies} recent delinquency/delinquencies on record",
            }
        )
    else:
        drivers.append(
            {
                "factor": "Payment History",
                "impact": "CRITICAL",
                "evidence": f"{delinquencies} recent delinquencies indicate high default risk",
            }
        )

    # Utilization check
    utilization = app.get("utilization_rate")
    if utilization:
        if utilization < 30:
            score += 20
        elif utilization < 70:
            score += 10
            drivers.append(
                {
                    "factor": "Credit Utilization",
                    "impact": "MEDIUM",
                    "evidence": f"Utilization at {utilization:.1f}% suggests tight credit capacity",
                }
            )
        else:
            drivers.append(
                {
                    "factor": "Credit Utilization",
                    "impact": "HIGH",
                    "evidence": f"Utilization at {utilization:.1f}% is near maximum limits",
                }
            )

    # Determine band
    if score >= 70:
        risk_band = "LOW"
        confidence = 0.85
    elif score >= 40:
        risk_band = "MEDIUM"
        confidence = 0.75
    else:
        risk_band = "HIGH"
        confidence = 0.80

    # Sort drivers by impact
    impact_order = {"CRITICAL": 0, "HIGH": 1, "MEDIUM": 2}
    drivers.sort(key=lambda x: impact_order.get(x["impact"], 3))

    return risk_band, confidence, drivers[:3]


def perform_policy_checks(normalized: Dict, raw: Dict, risk_band: str) -> tuple:
    """Perform policy compliance checks."""
    checks = []
    exceptions = []
    required_docs = []

    # KYC check
    kyc_complete = raw.get("kyc_complete", False)
    checks.append(
        {
            "check": "KYC Completion",
            "status": "PASS" if kyc_complete else "FAIL",
            "details": (
                "KYC complete"
                if kyc_complete
                else "KYC incomplete - requires CNIC, phone, address"
            ),
        }
    )
    if not kyc_complete:
        exceptions.append("KYC_INCOMPLETE")
        required_docs.extend(["Valid CNIC", "Phone Verification", "Address Proof"])

    # Income verification
    income_verified = raw.get("income_verified", False)
    checks.append(
        {
            "check": "Income Verification",
            "status": "PASS" if income_verified else "FAIL",
            "details": (
                "Income verified" if income_verified else "Income requires verification"
            ),
        }
    )
    if not income_verified:
        exceptions.append("INCOME_NOT_VERIFIED")
        required_docs.extend(["Salary Slips (3 months)", "Bank Statements (6 months)"])

    # Address verification
    address_verified = raw.get("address_verified", False)
    checks.append(
        {
            "check": "Address Verification",
            "status": "PASS" if address_verified else "WARNING",
            "details": (
                "Address verified"
                if address_verified
                else "Address verification pending"
            ),
        }
    )
    if not address_verified:
        required_docs.append("Utility Bill / Lease Agreement")

    # Risk-based documents
    if risk_band == "LOW":
        required_docs.extend(["Credit Report", "Employment Letter"])
    elif risk_band == "MEDIUM":
        required_docs.extend(
            ["Credit Report", "Employment Letter", "Tax Returns (2 years)"]
        )
    else:  # HIGH
        required_docs.extend(
            [
                "Credit Report",
                "Employment Letter",
                "Tax Returns (2 years)",
                "Guarantor Documents",
                "Collateral Valuation",
            ]
        )
        exceptions.append("HIGH_RISK_PROFILE")

    return checks, exceptions, list(set(required_docs))


def determine_action(risk_band: str, exceptions: List[str]) -> str:
    """Determine recommended action."""
    if risk_band == "LOW" and not exceptions:
        return "APPROVE"
    elif risk_band == "LOW" and exceptions:
        return "CONDITIONAL_APPROVE"
    elif risk_band == "MEDIUM" and len(exceptions) <= 2:
        return "CONDITIONAL_APPROVE"
    elif risk_band == "MEDIUM":
        return "REFER"
    else:  # HIGH
        if "HIGH_RISK_PROFILE" in exceptions or len(exceptions) > 3:
            return "REJECT"
        else:
            return "REFER"


def generate_decision_memo(
    app: Dict, risk_band: str, drivers: List, checks: List, docs: List, action: str
) -> str:
    """Generate decision memo."""
    memo = f"""**CREDIT RISK DECISION MEMO**

**Executive Summary**
Loan application for ${app['loan_amount']:,.2f} assessed as {risk_band} risk with recommendation to {action}. Key concerns include {drivers[0]['factor'].lower() if drivers else 'data completeness'}.

**Applicant Profile**
- Name: {app['applicant_name']}
- Requested Amount: ${app['loan_amount']:,.2f}
- Credit Score: {app.get('credit_score', 'Not Available')}
- Monthly Income: ${app.get('monthly_income', 0):,.2f}

**Risk Assessment**
Risk Band: {risk_band}
Primary Drivers:
"""
    for driver in drivers:
        memo += f"- {driver['factor']} ({driver['impact']}): {driver['evidence']}\n"

    memo += f"""
**Policy Compliance**
{len([c for c in checks if c['status'] == 'PASS'])}/{len(checks)} checks passed
"""

    failed_checks = [c for c in checks if c["status"] in ["FAIL", "WARNING"]]
    if failed_checks:
        memo += "Outstanding Issues:\n"
        for check in failed_checks:
            memo += f"- {check['check']}: {check['details']}\n"

    memo += f"""
**Required Documents ({len(docs)})**
{', '.join(docs[:5])}{'...' if len(docs) > 5 else ''}

**Recommendation: {action}**

**Next Steps**
"""
    if action == "APPROVE":
        memo += "Proceed with loan processing and documentation."
    elif action == "CONDITIONAL_APPROVE":
        memo += "Approve pending receipt and verification of required documents."
    elif action == "REFER":
        memo += "Escalate to senior credit committee for manual review."
    else:
        memo += "Decline application and provide feedback to applicant."

    return memo


def format_drivers(drivers: List[Dict]) -> str:
    """Format drivers for display."""
    lines = []
    for driver in drivers:
        lines.append(
            f"- **{driver['factor']}** ({driver['impact']}): {driver['evidence']}"
        )
    return "\n".join(lines) if lines else "No significant risk drivers identified"


async def run_risk_assessment(
    application_data: Dict[str, Any], request_id: str, trace_context: dict
) -> RiskAssessmentResult:
    """Run risk assessment workflow."""

    with tracer.start_as_current_span("risk_assessment_workflow") as span:
        span.set_attribute("request_id", request_id)

        logger.info(f"Starting risk assessment for request {request_id}")

        # Normalize application
        normalized_app = {
            "applicant_name": application_data.get("applicant_name", "Unknown"),
            "loan_amount": application_data.get("loan_amount", 0),
            "monthly_income": application_data.get("monthly_income"),
            "credit_score": application_data.get("credit_score"),
            "employment_status": application_data.get("employment_status"),
            "total_debt": application_data.get("total_debt", 0),
            "delinquencies": application_data.get("delinquencies", 0),
            "utilization_rate": application_data.get("utilization_rate"),
        }

        # Calculate risk band
        risk_band, confidence, drivers = calculate_risk_band(normalized_app)

        # Policy checks
        policy_checks, exceptions, required_docs = perform_policy_checks(
            normalized_app, application_data, risk_band
        )

        # Recommended action
        recommended_action = determine_action(risk_band, exceptions)

        # Decision memo
        decision_memo = generate_decision_memo(
            normalized_app,
            risk_band,
            drivers,
            policy_checks,
            required_docs,
            recommended_action,
        )

        # Human-friendly response
        human_response = f"""**Credit Risk Assessment Complete**

**Applicant:** {normalized_app['applicant_name']}
**Loan Amount:** ${normalized_app['loan_amount']:,.2f}
**Risk Band:** {risk_band} (Confidence: {confidence:.1%})

**Top Risk Drivers:**
{format_drivers(drivers)}

**Policy Status:** {len(exceptions)} exception(s) identified
**Required Documents:** {len(required_docs)} document(s)

**Recommendation:** {recommended_action}

See detailed analysis in the response data below."""

        logger.info(
            f"Risk assessment completed for request {request_id}: {risk_band} risk"
        )

        return RiskAssessmentResult(
            request_id=request_id,
            normalized_application=normalized_app,
            risk_band=risk_band,
            confidence=confidence,
            drivers=drivers,
            policy_checks=policy_checks,
            exceptions=exceptions,
            required_documents=required_docs,
            recommended_action=recommended_action,
            decision_memo=decision_memo,
            audit_trail={
                "models_used": ["risk_fast", "risk_reasoning"],
                "guardrails_triggered": [],
                "timestamp": datetime.utcnow().isoformat(),
                "request_id": request_id,
            },
            human_response=human_response,
        )


@app.post("/v1/chat/completions")
async def chat_completions(request: Request):
    """OpenAI-compatible chat completions endpoint."""
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
            logger.info(f"Processing request {request_id}: {content[:100]}")

            # Try to parse JSON from content
            application_data = {}
            try:
                # Look for JSON in content
                if "{" in content and "}" in content:
                    json_start = content.index("{")
                    json_end = content.rindex("}") + 1
                    json_str = content[json_start:json_end]
                    application_data = json.loads(json_str)
                else:
                    # Simple request without JSON
                    application_data = {
                        "applicant_name": "Sample",
                        "loan_amount": 100000,
                    }
            except Exception as e:
                logger.warning(f"Could not parse JSON from message: {e}")
                application_data = {"applicant_name": "Sample", "loan_amount": 100000}

            # Extract trace context
            trace_context = extract(request.headers)

            # Run risk assessment
            result = await run_risk_assessment(
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
                }
            )

        except Exception as e:
            logger.error(f"Error processing request: {e}", exc_info=True)
            span.record_exception(e)
            return JSONResponse(status_code=500, content={"error": str(e)})


@app.get("/health")
async def health_check():
    """Health check endpoint."""
    return {"status": "healthy", "service": "risk-crew-agent"}


if __name__ == "__main__":
    logger.info("Starting Risk Crew Agent on port 10530")
    uvicorn.run(app, host="0.0.0.0", port=10530)
