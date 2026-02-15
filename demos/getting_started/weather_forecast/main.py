import json
import os
import random
from fastapi import FastAPI, Response
from datetime import datetime, date, timedelta, timezone
import logging
from pydantic import BaseModel
from opentelemetry import trace
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.instrumentation.fastapi import FastAPIInstrumentor
from opentelemetry.exporter.otlp.proto.grpc.trace_exporter import OTLPSpanExporter
from opentelemetry.sdk.trace.export import BatchSpanProcessor
from opentelemetry.sdk.resources import Resource


resource = Resource.create(
    {
        "service.name": "weather-forecast-service",
    }
)

# Initialize the tracer provider
trace.set_tracer_provider(TracerProvider(resource=resource))
tracer = trace.get_tracer(__name__)

logger = logging.getLogger("uvicorn.error")
logger.setLevel(logging.INFO)

app = FastAPI()
FastAPIInstrumentor().instrument_app(app)

# Configure the OTLP exporter (Jaeger, Zipkin, etc.)
otlp_exporter = OTLPSpanExporter(
    endpoint=os.getenv("OLTP_HOST", "http://localhost:4317")
)
trace.get_tracer_provider().add_span_processor(BatchSpanProcessor(otlp_exporter))


@app.get("/healthz")
async def healthz():
    return {"status": "ok"}


class WeatherRequest(BaseModel):
    location: str
    days: int = 7
    units: str = "Farenheit"


@app.post("/weather")
async def weather(req: WeatherRequest, res: Response):
    weather_forecast = {
        "location": req.location,
        "temperature": [],
        "units": req.units,
    }
    for i in range(req.days):
        min_temp = random.randrange(50, 90)
        max_temp = random.randrange(min_temp + 5, min_temp + 20)
        if req.units.lower() == "celsius" or req.units.lower() == "c":
            min_temp = (min_temp - 32) * 5.0 / 9.0
            max_temp = (max_temp - 32) * 5.0 / 9.0
        weather_forecast["temperature"].append(
            {
                "date": str(date.today() + timedelta(days=i)),
                "temperature": {"min": min_temp, "max": max_temp},
                "units": req.units,
                "query_time": str(datetime.now(timezone.utc)),
            }
        )

    return weather_forecast


class DefaultTargetRequest(BaseModel):
    messages: list = []


@app.post("/default_target")
async def default_target(req: DefaultTargetRequest, res: Response):
    logger.info(f"Received messages: {req.messages}")
    resp = {
        "choices": [
            {
                "message": {
                    "role": "assistant",
                    "content": "I can help you with weather forecast",
                },
            }
        ],
        "model": "api_server",
    }
    logger.info(f"sending response: {json.dumps(resp)}")
    return resp
