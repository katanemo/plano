import json
from fastapi import FastAPI, Request
from fastapi.responses import StreamingResponse
from openai import AsyncOpenAI
import os
import logging
import time
import uuid
import uvicorn
from datetime import datetime, timedelta
import httpx
from typing import Optional
from opentelemetry.propagate import extract, inject

# Set up logging
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - [FLIGHT_AGENT] - %(levelname)s - %(message)s",
)
logger = logging.getLogger(__name__)

# Configuration
LLM_GATEWAY_ENDPOINT = os.getenv(
    "LLM_GATEWAY_ENDPOINT", "http://host.docker.internal:12000/v1"
)
FLIGHT_MODEL = "openai/gpt-4o"
EXTRACTION_MODEL = "openai/gpt-4o-mini"

# FlightAware AeroAPI configuration
AEROAPI_BASE_URL = "https://aeroapi.flightaware.com/aeroapi"
AEROAPI_KEY = os.getenv("AEROAPI_KEY")

# HTTP client for API calls
http_client = httpx.AsyncClient(timeout=30.0)

# Initialize OpenAI client
openai_client_via_plano = AsyncOpenAI(
    base_url=LLM_GATEWAY_ENDPOINT,
    api_key="EMPTY",
)

# System prompt for flight agent
SYSTEM_PROMPT = """You are a travel planning assistant specializing in flight information in a multi-agent system. You will receive flight data in JSON format with these fields:

- "airline": Full airline name (e.g., "Delta Air Lines")
- "flight_number": Flight identifier (e.g., "DL123")
- "departure_time": ISO 8601 timestamp for scheduled departure (e.g., "2025-12-24T23:00:00Z")
- "arrival_time": ISO 8601 timestamp for scheduled arrival (e.g., "2025-12-25T04:40:00Z")
- "origin": Origin airport IATA code (e.g., "ATL")
- "destination": Destination airport IATA code (e.g., "SEA")
- "aircraft_type": Aircraft model code (e.g., "A21N", "B739")
- "status": Flight status (e.g., "Scheduled", "Delayed")
- "terminal_origin": Departure terminal (may be null)
- "gate_origin": Departure gate (may be null)

Your task:
1. Read the JSON flight data carefully
2. Present each flight clearly with: airline, flight number, departure/arrival times (convert from ISO format to readable time), airports, and aircraft type
3. Organize flights chronologically by departure time
4. Convert ISO timestamps to readable format (e.g., "11:00 PM" or "23:00")
5. Include terminal/gate info when available
6. Use natural, conversational language

Important: If the conversation includes information from other agents (like weather details), acknowledge and build upon that context naturally. Your primary focus is flights, but maintain awareness of the full conversation.

Remember: All the data you need is in the JSON. Use it directly."""


async def extract_flight_route(messages: list, request: Request) -> dict:
    """Extract origin, destination, and date from conversation using LLM."""

    extraction_prompt = """Extract flight origin, destination cities, and travel date from the conversation.

    Rules:
    1. Look for patterns: "flight from X to Y", "flights to Y", "fly from X"
    2. Extract dates like "tomorrow", "next week", "December 25", "12/25", "on Monday"
    3. Use conversation context to fill in missing details
    4. Return JSON: {"origin": "City" or null, "destination": "City" or null, "date": "YYYY-MM-DD" or null}

    Examples:
    - "Flight from Seattle to Atlanta tomorrow" -> {"origin": "Seattle", "destination": "Atlanta", "date": "2025-12-24"}
    - "What flights go to New York?" -> {"origin": null, "destination": "New York", "date": null}
    - "Flights to Miami on Christmas" -> {"origin": null, "destination": "Miami", "date": "2025-12-25"}
    - "Show me flights from LA to NYC next Monday" -> {"origin": "LA", "destination": "NYC", "date": "2025-12-30"}

    Today is December 23, 2025. Extract flight route and date:"""

    try:
        ctx = extract(request.headers)
        extra_headers = {}
        inject(extra_headers, context=ctx)

        response = await openai_client_via_plano.chat.completions.create(
            model=EXTRACTION_MODEL,
            messages=[
                {"role": "system", "content": extraction_prompt},
                *[
                    {"role": msg.get("role"), "content": msg.get("content")}
                    for msg in messages[-5:]
                ],
            ],
            temperature=0.1,
            max_tokens=100,
            extra_headers=extra_headers if extra_headers else None,
        )

        result = response.choices[0].message.content.strip()
        if "```json" in result:
            result = result.split("```json")[1].split("```")[0].strip()
        elif "```" in result:
            result = result.split("```")[1].split("```")[0].strip()

        route = json.loads(result)
        return {
            "origin": route.get("origin"),
            "destination": route.get("destination"),
            "date": route.get("date"),
        }
    except Exception as e:
        logger.error(f"Error extracting flight route: {e}")
        return {"origin": None, "destination": None, "date": None}


async def resolve_airport_code(city_name: str, request: Request) -> Optional[str]:
    """Convert city name to airport code using LLM."""
    if not city_name:
        return None

    try:
        ctx = extract(request.headers)
        extra_headers = {}
        inject(extra_headers, context=ctx)

        response = await openai_client_via_plano.chat.completions.create(
            model=EXTRACTION_MODEL,
            messages=[
                {
                    "role": "system",
                    "content": "Convert city names to primary airport IATA codes. Return only the 3-letter code. Examples: Seattle→SEA, Atlanta→ATL, New York→JFK, London→LHR",
                },
                {"role": "user", "content": city_name},
            ],
            temperature=0.1,
            max_tokens=10,
            extra_headers=extra_headers if extra_headers else None,
        )

        code = response.choices[0].message.content.strip().upper()
        code = code.strip("\"'`.,!? \n\t")
        return code if len(code) == 3 else None
    except Exception as e:
        logger.error(f"Error resolving airport code for {city_name}: {e}")
        return None


async def get_flights(
    origin_code: str, dest_code: str, travel_date: Optional[str] = None
) -> Optional[dict]:
    """Get flights between two airports using FlightAware API.

    Args:
        origin_code: Origin airport IATA code
        dest_code: Destination airport IATA code
        travel_date: Travel date in YYYY-MM-DD format, defaults to today

    Note: FlightAware API limits searches to 2 days in the future.
    """
    try:
        # Use provided date or default to today
        if travel_date:
            search_date = travel_date
        else:
            search_date = datetime.now().strftime("%Y-%m-%d")

        # Validate date is not too far in the future (FlightAware limit: 2 days)
        search_date_obj = datetime.strptime(search_date, "%Y-%m-%d")
        today = datetime.now().replace(hour=0, minute=0, second=0, microsecond=0)
        days_ahead = (search_date_obj - today).days

        if days_ahead > 2:
            logger.warning(
                f"Requested date {search_date} is {days_ahead} days ahead, exceeds FlightAware 2-day limit"
            )
            return {
                "origin_code": origin_code,
                "destination_code": dest_code,
                "flights": [],
                "count": 0,
                "error": f"FlightAware API only provides flight data up to 2 days in the future. The requested date ({search_date}) is {days_ahead} days ahead. Please search for today, tomorrow, or the day after.",
            }

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
            return None

        data = response.json()
        flights = []

        # Log raw API response for debugging
        logger.info(f"FlightAware API returned {len(data.get('flights', []))} flights")

        for idx, flight_group in enumerate(
            data.get("flights", [])[:5]
        ):  # Limit to 5 flights
            # FlightAware API nests data in segments array
            segments = flight_group.get("segments", [])
            if not segments:
                continue

            flight = segments[0]  # Get first segment (direct flights only have one)

            # Extract airport codes from nested objects
            flight_origin = None
            flight_dest = None

            if isinstance(flight.get("origin"), dict):
                flight_origin = flight["origin"].get("code_iata")

            if isinstance(flight.get("destination"), dict):
                flight_dest = flight["destination"].get("code_iata")

            # Build flight object
            flights.append(
                {
                    "airline": flight.get("operator"),
                    "flight_number": flight.get("ident_iata") or flight.get("ident"),
                    "departure_time": flight.get("scheduled_out"),
                    "arrival_time": flight.get("scheduled_in"),
                    "origin": flight_origin,
                    "destination": flight_dest,
                    "aircraft_type": flight.get("aircraft_type"),
                    "status": flight.get("status"),
                    "terminal_origin": flight.get("terminal_origin"),
                    "gate_origin": flight.get("gate_origin"),
                }
            )

        return {
            "origin_code": origin_code,
            "destination_code": dest_code,
            "flights": flights,
            "count": len(flights),
        }
    except Exception as e:
        logger.error(f"Error fetching flights: {e}")
        return None


app = FastAPI(title="Flight Information Agent", version="1.0.0")


@app.post("/v1/chat/completions")
async def handle_request(request: Request):
    """HTTP endpoint for chat completions with streaming support."""
    request_body = await request.json()
    messages = request_body.get("messages", [])

    return StreamingResponse(
        invoke_flight_agent(request, request_body),
        media_type="text/plain",
        headers={"content-type": "text/event-stream"},
    )


async def invoke_flight_agent(request: Request, request_body: dict):
    """Generate streaming chat completions."""
    messages = request_body.get("messages", [])

    # Step 1: Extract origin, destination, and date
    route = await extract_flight_route(messages, request)
    origin = route.get("origin")
    destination = route.get("destination")
    travel_date = route.get("date")

    # Step 2: Short circuit if missing origin or destination
    if not origin or not destination:
        missing = []
        if not origin:
            missing.append("origin city")
        if not destination:
            missing.append("destination city")

        error_message = f"I need both origin and destination cities to search for flights. Please provide the {' and '.join(missing)}. For example: 'Flights from Seattle to Atlanta'"

        error_chunk = {
            "id": f"chatcmpl-{uuid.uuid4().hex[:8]}",
            "object": "chat.completion.chunk",
            "created": int(time.time()),
            "model": request_body.get("model", FLIGHT_MODEL),
            "choices": [
                {
                    "index": 0,
                    "delta": {"content": error_message},
                    "finish_reason": "stop",
                }
            ],
        }
        yield f"data: {json.dumps(error_chunk)}\n\n"
        yield "data: [DONE]\n\n"
        return

    # Step 3: Resolve airport codes
    origin_code = await resolve_airport_code(origin, request)
    dest_code = await resolve_airport_code(destination, request)

    if not origin_code or not dest_code:
        error_chunk = {
            "id": f"chatcmpl-{uuid.uuid4().hex[:8]}",
            "object": "chat.completion.chunk",
            "created": int(time.time()),
            "model": request_body.get("model", FLIGHT_MODEL),
            "choices": [
                {
                    "index": 0,
                    "delta": {
                        "content": f"I couldn't find airport codes for {origin if not origin_code else destination}. Please check the city name."
                    },
                    "finish_reason": "stop",
                }
            ],
        }
        yield f"data: {json.dumps(error_chunk)}\n\n"
        yield "data: [DONE]\n\n"
        return

    # Step 4: Get live flight data
    flight_data = await get_flights(origin_code, dest_code, travel_date)

    # Determine date display for messages
    date_display = travel_date if travel_date else "today"

    if not flight_data or not flight_data.get("flights"):
        # Check if there's a specific error message (e.g., date too far in future)
        error_detail = flight_data.get("error") if flight_data else None
        if error_detail:
            no_flights_message = error_detail
        else:
            no_flights_message = f"No direct flights found from {origin} ({origin_code}) to {destination} ({dest_code}) for {date_display}."

        error_chunk = {
            "id": f"chatcmpl-{uuid.uuid4().hex[:8]}",
            "object": "chat.completion.chunk",
            "created": int(time.time()),
            "model": request_body.get("model", FLIGHT_MODEL),
            "choices": [
                {
                    "index": 0,
                    "delta": {"content": no_flights_message},
                    "finish_reason": "stop",
                }
            ],
        }
        yield f"data: {json.dumps(error_chunk)}\n\n"
        yield "data: [DONE]\n\n"
        return

    # Step 5: Prepare context for LLM - append flight data to last user message
    flight_context = f"""

Flight search results from {origin} ({origin_code}) to {destination} ({dest_code}):

Flight data in JSON format:
{json.dumps(flight_data, indent=2)}

Present these {len(flight_data.get('flights', []))} flight(s) to the user in a clear, readable format."""

    # Build message history with flight data appended to the last user message
    response_messages = [{"role": "system", "content": SYSTEM_PROMPT}]

    for i, msg in enumerate(messages):
        # Append flight data to the last user message
        if i == len(messages) - 1 and msg.get("role") == "user":
            response_messages.append(
                {"role": "user", "content": msg.get("content") + flight_context}
            )
        else:
            response_messages.append(
                {"role": msg.get("role"), "content": msg.get("content")}
            )

    # Log what we're sending to the LLM for debugging
    logger.info(f"Sending messages to LLM: {json.dumps(response_messages, indent=2)}")

    # Step 6: Stream response
    try:
        ctx = extract(request.headers)
        extra_headers = {"x-envoy-max-retries": "3"}
        inject(extra_headers, context=ctx)

        stream = await openai_client_via_plano.chat.completions.create(
            model=FLIGHT_MODEL,
            messages=response_messages,
            temperature=request_body.get("temperature", 0.7),
            max_tokens=request_body.get("max_tokens", 1000),
            stream=True,
            extra_headers=extra_headers,
        )

        async for chunk in stream:
            if chunk.choices:
                yield f"data: {chunk.model_dump_json()}\n\n"

        yield "data: [DONE]\n\n"

    except Exception as e:
        logger.error(f"Error generating flight response: {e}")
        error_chunk = {
            "id": f"chatcmpl-{uuid.uuid4().hex[:8]}",
            "object": "chat.completion.chunk",
            "created": int(time.time()),
            "model": request_body.get("model", FLIGHT_MODEL),
            "choices": [
                {
                    "index": 0,
                    "delta": {
                        "content": "I apologize, but I'm having trouble retrieving flight information right now. Please try again."
                    },
                    "finish_reason": "stop",
                }
            ],
        }
        yield f"data: {json.dumps(error_chunk)}\n\n"
        yield "data: [DONE]\n\n"


@app.get("/health")
async def health_check():
    """Health check endpoint."""
    return {"status": "healthy", "agent": "flight_information"}


def start_server(host: str = "localhost", port: int = 10520):
    """Start the REST server."""
    uvicorn.run(
        app,
        host=host,
        port=port,
        log_config={
            "version": 1,
            "disable_existing_loggers": False,
            "formatters": {
                "default": {
                    "format": "%(asctime)s - [FLIGHT_AGENT] - %(levelname)s - %(message)s",
                },
            },
            "handlers": {
                "default": {
                    "formatter": "default",
                    "class": "logging.StreamHandler",
                    "stream": "ext://sys.stdout",
                },
            },
            "root": {
                "level": "INFO",
                "handlers": ["default"],
            },
        },
    )


if __name__ == "__main__":
    start_server(host="0.0.0.0", port=10520)
