import json

from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse, StreamingResponse
from openai import AsyncOpenAI
import os
import logging
import uvicorn
from datetime import datetime
import httpx
from typing import Optional
import uuid
import time
from opentelemetry.propagate import extract, inject
from pydantic import BaseModel, Field
from langchain_openai import ChatOpenAI
from langchain_core.tools import tool
from langchain.agents import create_agent

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - [FLIGHT_AGENT] - %(levelname)s - %(message)s",
)
logger = logging.getLogger(__name__)

LLM_GATEWAY_ENDPOINT = os.getenv(
    "LLM_GATEWAY_ENDPOINT", "http://host.docker.internal:12000/v1"
)
FLIGHT_MODEL = "openai/gpt-4o"
EXTRACTION_MODEL = "openai/gpt-4o-mini"

AEROAPI_BASE_URL = "https://aeroapi.flightaware.com/aeroapi"
AEROAPI_KEY = os.getenv("AEROAPI_KEY")

http_client = httpx.AsyncClient(timeout=30.0)
openai_client = AsyncOpenAI(base_url=LLM_GATEWAY_ENDPOINT, api_key="EMPTY")


class FlightSearchInput(BaseModel):
    origin_city: str = Field(..., description="Origin city for the flight search")
    destination_city: str = Field(
        ..., description="Destination city for the flight search"
    )
    travel_date: Optional[str] = Field(
        None,
        description="Optional travel date in YYYY-MM-DD format. If not provided, defaults to today.",
    )


SYSTEM_PROMPT = """You are a travel planning assistant specializing in flight information. You support both direct flights AND multi-leg connecting flights.

Flight data fields:
- airline: Full airline name (e.g., "Delta Air Lines")
- flight_number: Flight identifier (e.g., "DL123")
- departure_time/arrival_time: ISO 8601 timestamps
- origin/destination: Airport IATA codes
- aircraft_type: Aircraft model code (e.g., "B739")
- status: Flight status (e.g., "Scheduled", "Delayed")
- terminal_origin/gate_origin: Departure terminal and gate (may be null)

Your task:
1. Present flights clearly with airline, flight number, readable times, airports, and aircraft
2. Organize chronologically by departure time
3. Convert ISO timestamps to readable format (e.g., "11:00 AM")
4. Include terminal/gate info when available
5. For multi-leg flights: present each leg separately with connection timing

Multi-agent context: If the conversation includes information from other sources, incorporate it naturally into your response."""


def build_flight_agent(
    request: Request,
    request_body: dict,
    streaming: bool,
):
    ctx = extract(request.headers)
    extra_headers = {"x-envoy-max-retries": "3"}
    request_id = request.headers.get("x-request-id")
    if request_id:
        extra_headers["x-request-id"] = request_id
    inject(extra_headers, context=ctx)

    @tool("search_flights", args_schema=FlightSearchInput)
    async def search_flights(
        origin_city: str, destination_city: str, travel_date: Optional[str] = None
    ):
        """Search for flights between two cities. Supports optional travel date."""
        origin_code = await resolve_airport_code(origin_city, request)
        dest_code = await resolve_airport_code(destination_city, request)

        if not origin_code or not dest_code:
            return {
                "error": "Could not resolve airport codes for provided cities.",
                "origin_city": origin_city,
                "destination_city": destination_city,
            }

        flight_data = await fetch_flights(origin_code, dest_code, travel_date)
        return {
            "origin_city": origin_city,
            "destination_city": destination_city,
            "origin_code": origin_code,
            "destination_code": dest_code,
            "travel_date": travel_date or datetime.now().strftime("%Y-%m-%d"),
            "flights": flight_data.get("flights", []),
            "count": flight_data.get("count", 0),
            "error": flight_data.get("error"),
        }

    llm = ChatOpenAI(
        model=FLIGHT_MODEL,
        api_key="EMPTY",
        base_url=LLM_GATEWAY_ENDPOINT,
        temperature=request_body.get("temperature", 0.7),
        max_tokens=request_body.get("max_tokens", 1000),
        streaming=streaming,
        default_headers=extra_headers,
    )

    return create_agent(
        model=llm,
        tools=[search_flights],
        system_prompt=SYSTEM_PROMPT,
    )


async def resolve_airport_code(city_name: str, request: Request) -> Optional[str]:
    if not city_name:
        return None

    try:
        ctx = extract(request.headers)
        extra_headers = {}
        inject(extra_headers, context=ctx)

        response = await openai_client.chat.completions.create(
            model=EXTRACTION_MODEL,
            messages=[
                {
                    "role": "system",
                    "content": "Convert city names to primary airport IATA codes. Return only the 3-letter code. Examples: Seattle→SEA, Atlanta→ATL, New York→JFK, Dubai→DXB, Lahore→LHE",
                },
                {"role": "user", "content": city_name},
            ],
            temperature=0.1,
            max_tokens=10,
            extra_headers=extra_headers or None,
        )

        code = response.choices[0].message.content.strip().upper()
        code = code.strip("\"'`.,!? \n\t")
        return code if len(code) == 3 else None

    except Exception as e:
        logger.error(f"Error resolving airport code for {city_name}: {e}")
        return None


async def fetch_flights(
    origin_code: str, dest_code: str, travel_date: Optional[str] = None
) -> dict:
    """Fetch flights between two airports. Note: FlightAware limits to 2 days ahead."""
    search_date = travel_date or datetime.now().strftime("%Y-%m-%d")

    search_date_obj = datetime.strptime(search_date, "%Y-%m-%d")
    today = datetime.now().replace(hour=0, minute=0, second=0, microsecond=0)
    days_ahead = (search_date_obj - today).days

    if days_ahead > 2:
        logger.warning(
            f"Date {search_date} is {days_ahead} days ahead, exceeds FlightAware limit"
        )
        return {
            "origin_code": origin_code,
            "destination_code": dest_code,
            "flights": [],
            "count": 0,
            "error": f"FlightAware API only provides data up to 2 days ahead. Requested date ({search_date}) is {days_ahead} days away.",
        }

    try:
        url = f"{AEROAPI_BASE_URL}/airports/{origin_code}/flights/to/{dest_code}"
        headers = {"x-apikey": AEROAPI_KEY}
        params = {
            "start": f"{search_date}T00:00:00Z",
            "end": f"{search_date}T23:59:59Z",
            "connection": "nonstop",
            "max_pages": 1,
        }

        response = await http_client.get(url, headers=headers, params=params)

        if response.status_code != 200:
            logger.error(
                f"FlightAware API error {response.status_code}: {response.text}"
            )
            return {
                "origin_code": origin_code,
                "destination_code": dest_code,
                "flights": [],
                "count": 0,
            }

        data = response.json()
        flights = []

        for flight_group in data.get("flights", [])[:5]:
            segments = flight_group.get("segments", [])
            if not segments:
                continue

            flight = segments[0]
            flights.append(
                {
                    "airline": flight.get("operator"),
                    "flight_number": flight.get("ident_iata") or flight.get("ident"),
                    "departure_time": flight.get("scheduled_out"),
                    "arrival_time": flight.get("scheduled_in"),
                    "origin": flight["origin"].get("code_iata")
                    if isinstance(flight.get("origin"), dict)
                    else None,
                    "destination": flight["destination"].get("code_iata")
                    if isinstance(flight.get("destination"), dict)
                    else None,
                    "aircraft_type": flight.get("aircraft_type"),
                    "status": flight.get("status"),
                    "terminal_origin": flight.get("terminal_origin"),
                    "gate_origin": flight.get("gate_origin"),
                }
            )

        logger.info(f"Found {len(flights)} flights from {origin_code} to {dest_code}")
        return {
            "origin_code": origin_code,
            "destination_code": dest_code,
            "flights": flights,
            "count": len(flights),
        }

    except Exception as e:
        logger.error(f"Error fetching flights: {e}")
        return {
            "origin_code": origin_code,
            "destination_code": dest_code,
            "flights": [],
            "count": 0,
        }


app = FastAPI(title="Flight Information Agent", version="1.0.0")


@app.post("/v1/chat/completions")
async def handle_request(request: Request):
    request_body = await request.json()
    is_streaming = request_body.get("stream", True)
    model = request_body.get("model", FLIGHT_MODEL)

    if is_streaming:
        return StreamingResponse(
            invoke_flight_agent_stream(request, request_body, model),
            media_type="text/event-stream",
            headers={"content-type": "text/event-stream"},
        )

    content = await invoke_flight_agent(request, request_body)
    return JSONResponse(
        {
            "id": f"chatcmpl-{uuid.uuid4().hex[:8]}",
            "object": "chat.completion",
            "created": int(time.time()),
            "model": model,
            "choices": [
                {
                    "index": 0,
                    "message": {"role": "assistant", "content": content},
                    "finish_reason": "stop",
                }
            ],
        }
    )


async def invoke_flight_agent(request: Request, request_body: dict):
    """Generate flight information using a LangChain agent with the modern create_agent API."""
    messages = request_body.get("messages", [])
    agent = build_flight_agent(request, request_body, streaming=False)

    try:
        # Invoke agent with messages
        result = await agent.ainvoke({"messages": messages})

        # Extract final response from messages
        final_message = result["messages"][-1]
        content = (
            final_message.content
            if hasattr(final_message, "content")
            else str(final_message)
        )
        return content
    except Exception as e:
        logger.error(f"Error generating response: {e}")
        return "I'm having trouble retrieving flight information right now. Please try again."


def build_openai_chunk(model: str, content: str, finish_reason: Optional[str] = None):
    return {
        "id": f"chatcmpl-{uuid.uuid4().hex[:8]}",
        "object": "chat.completion.chunk",
        "created": int(time.time()),
        "model": model,
        "choices": [
            {
                "index": 0,
                "delta": {"content": content} if content else {},
                "finish_reason": finish_reason,
            }
        ],
    }


async def invoke_flight_agent_stream(
    request: Request,
    request_body: dict,
    model: str,
):
    messages = request_body.get("messages", [])
    agent = build_flight_agent(request, request_body, streaming=True)

    try:
        async for event in agent.astream_events(
            {"messages": messages},
            version="v2",
        ):
            if event.get("event") != "on_chat_model_stream":
                continue
            chunk = event.get("data", {}).get("chunk")
            content = getattr(chunk, "content", None)
            if not content:
                continue
            if isinstance(content, list):
                content = "".join(
                    piece for piece in content if isinstance(piece, str)
                ).strip()
                if not content:
                    continue
            yield f"data: {json.dumps(build_openai_chunk(model, content))}\n\n"

        yield f"data: {json.dumps(build_openai_chunk(model, '', 'stop'))}\n\n"
        yield "data: [DONE]\n\n"
    except Exception as e:
        logger.error(f"Error streaming response: {e}")
        error_message = "I'm having trouble retrieving flight information right now. Please try again."
        yield f"data: {json.dumps(build_openai_chunk(model, error_message, 'stop'))}\n\n"
        yield "data: [DONE]\n\n"


@app.get("/health")
async def health_check():
    return {"status": "healthy", "agent": "flight_information"}


def start_server(host: str = "0.0.0.0", port: int = 10520):
    uvicorn.run(
        app,
        host=host,
        port=port,
        log_config={
            "version": 1,
            "disable_existing_loggers": False,
            "formatters": {
                "default": {
                    "format": "%(asctime)s - [FLIGHT_AGENT] - %(levelname)s - %(message)s"
                }
            },
            "handlers": {
                "default": {
                    "formatter": "default",
                    "class": "logging.StreamHandler",
                    "stream": "ext://sys.stdout",
                }
            },
            "root": {"level": "INFO", "handlers": ["default"]},
        },
    )


if __name__ == "__main__":
    start_server()
