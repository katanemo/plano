import logging
import os
import uuid
from datetime import datetime
from typing import Dict, List, Optional

import uvicorn
from fastapi import FastAPI, HTTPException
from opentelemetry import trace
from opentelemetry.exporter.otlp.proto.http.trace_exporter import OTLPSpanExporter
from opentelemetry.instrumentation.fastapi import FastAPIInstrumentor
from opentelemetry.sdk.resources import Resource
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import BatchSpanProcessor
from pydantic import BaseModel, Field

# Logging
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - [CASE_SERVICE] - %(levelname)s - %(message)s",
)
logger = logging.getLogger(__name__)

# OpenTelemetry setup
OTLP_ENDPOINT = os.getenv("OTLP_ENDPOINT", "http://jaeger:4318/v1/traces")
resource = Resource.create({"service.name": "case-service"})
tracer_provider = TracerProvider(resource=resource)
otlp_exporter = OTLPSpanExporter(endpoint=OTLP_ENDPOINT)
tracer_provider.add_span_processor(BatchSpanProcessor(otlp_exporter))
trace.set_tracer_provider(tracer_provider)
tracer = trace.get_tracer(__name__)

# FastAPI app
app = FastAPI(title="Case Management Service", version="1.0.0")
FastAPIInstrumentor.instrument_app(app)

# In-memory case store (use database in production)
case_store: Dict[str, Dict] = {}


# Data models
class CreateCaseRequest(BaseModel):
    applicant_name: str = Field(..., description="Full name of the loan applicant")
    loan_amount: float = Field(..., description="Requested loan amount", gt=0)
    risk_band: str = Field(
        ..., description="Risk classification", pattern="^(LOW|MEDIUM|HIGH)$"
    )
    confidence: float = Field(..., description="Confidence score", ge=0.0, le=1.0)
    recommended_action: str = Field(
        ...,
        description="Recommended action",
        pattern="^(APPROVE|CONDITIONAL_APPROVE|REFER|REJECT)$",
    )
    required_documents: List[str] = Field(default_factory=list)
    policy_exceptions: Optional[List[str]] = Field(default_factory=list)
    notes: Optional[str] = None


class CaseResponse(BaseModel):
    case_id: str
    status: str
    created_at: str
    applicant_name: str
    loan_amount: float
    risk_band: str
    recommended_action: str


class CaseDetail(CaseResponse):
    confidence: float
    required_documents: List[str]
    policy_exceptions: List[str]
    notes: Optional[str]
    updated_at: str


@app.post("/cases", response_model=CaseResponse)
async def create_case(request: CreateCaseRequest):
    """Create a new credit risk case."""
    with tracer.start_as_current_span("create_case") as span:
        case_id = f"CASE-{uuid.uuid4().hex[:8].upper()}"
        created_at = datetime.utcnow().isoformat()

        span.set_attribute("case_id", case_id)
        span.set_attribute("risk_band", request.risk_band)
        span.set_attribute("recommended_action", request.recommended_action)

        case_data = {
            "case_id": case_id,
            "status": "OPEN",
            "created_at": created_at,
            "updated_at": created_at,
            "applicant_name": request.applicant_name,
            "loan_amount": request.loan_amount,
            "risk_band": request.risk_band,
            "confidence": request.confidence,
            "recommended_action": request.recommended_action,
            "required_documents": request.required_documents,
            "policy_exceptions": request.policy_exceptions or [],
            "notes": request.notes,
        }

        case_store[case_id] = case_data

        logger.info(
            f"Created case {case_id} for {request.applicant_name} - {request.risk_band} risk"
        )

        return CaseResponse(
            case_id=case_id,
            status="OPEN",
            created_at=created_at,
            applicant_name=request.applicant_name,
            loan_amount=request.loan_amount,
            risk_band=request.risk_band,
            recommended_action=request.recommended_action,
        )


@app.get("/cases/{case_id}", response_model=CaseDetail)
async def get_case(case_id: str):
    """Retrieve a case by ID."""
    with tracer.start_as_current_span("get_case") as span:
        span.set_attribute("case_id", case_id)

        if case_id not in case_store:
            raise HTTPException(status_code=404, detail=f"Case {case_id} not found")

        case_data = case_store[case_id]
        logger.info(f"Retrieved case {case_id}")

        return CaseDetail(**case_data)


@app.get("/cases", response_model=List[CaseResponse])
async def list_cases(limit: int = 50):
    """List all cases."""
    with tracer.start_as_current_span("list_cases"):
        cases = [
            CaseResponse(
                case_id=case["case_id"],
                status=case["status"],
                created_at=case["created_at"],
                applicant_name=case["applicant_name"],
                loan_amount=case["loan_amount"],
                risk_band=case["risk_band"],
                recommended_action=case["recommended_action"],
            )
            for case in list(case_store.values())[:limit]
        ]

        logger.info(f"Listed {len(cases)} cases")
        return cases


@app.get("/health")
async def health_check():
    """Health check endpoint."""
    return {
        "status": "healthy",
        "service": "case-service",
        "cases_count": len(case_store),
    }


if __name__ == "__main__":
    logger.info("Starting Case Service on port 10540")
    uvicorn.run(app, host="0.0.0.0", port=10540)
