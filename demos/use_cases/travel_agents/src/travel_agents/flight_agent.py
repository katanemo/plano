import json
from fastapi import FastAPI, Request
from fastapi.responses import StreamingResponse
from openai import AsyncOpenAI
import os
import logging
import uvicorn
from datetime import datetime
import httpx
from typing import Optional
from opentelemetry.propagate import extract, inject

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

ROUTE_EXTRACTION_PROMPT = """Extract flight route and travel date. Support direct AND multi-leg flights.

Rules:
1. Patterns: "flight from X to Y", "X to Y to Z", "fly from X through Y to Z"
2. For multi-leg (e.g., "Seattle to Dubai to Lahore"), extract ALL cities in order
3. Extract dates: "tomorrow", "next week", "December 25", "12/25", "on Monday"
4. Use conversation context for missing details

Output format: {"cities": ["City1", "City2", ...], "date": "YYYY-MM-DD" or null}

Examples:
- "Flight from Seattle to Atlanta tomorrow" → {"cities": ["Seattle", "Atlanta"], "date": "2026-01-07"}
- "Seattle to Dubai to Lahore" → {"cities": ["Seattle", "Dubai", "Lahore"], "date": null}
- "Flights from LA through Chicago to NYC" → {"cities": ["LA", "Chicago", "NYC"], "date": null}

Today is January 6, 2026. Extract flight route:"""


async def extract_flight_route(messages: list, request: Request) -> dict:
    try:
        ctx = extract(request.headers)
        extra_headers = {}
        inject(extra_headers, context=ctx)

        response = await openai_client.chat.completions.create(
            model=EXTRACTION_MODEL,
            messages=[
                {"role": "system", "content": ROUTE_EXTRACTION_PROMPT},
                *[
                    {"role": m.get("role"), "content": m.get("content")}
                    for m in messages[-5:]
                ],
            ],
            temperature=0.1,
            max_tokens=100,
            extra_headers=extra_headers or None,
        )

        result = response.choices[0].message.content.strip()
        if "```json" in result:
            result = result.split("```json")[1].split("```")[0].strip()
        elif "```" in result:
            result = result.split("```")[1].split("```")[0].strip()

        route = json.loads(result)
        cities = route.get("cities", [])

        if not cities and (route.get("origin") or route.get("destination")):
            cities = [c for c in [route.get("origin"), route.get("destination")] if c]

        return {"cities": cities, "date": route.get("date")}

    except Exception as e:
        logger.error(f"Error extracting flight route: {e}")
        return {"cities": [], "date": None}


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


def build_flight_context(cities: list, airport_codes: list, legs_data: list) -> str:
    if len(cities) == 2:
        leg = legs_data[0]
        flight_data = {
            "flights": leg["flights"],
            "count": len(leg["flights"]),
            "origin_code": leg["origin_code"],
            "destination_code": leg["dest_code"],
        }
        if leg["flights"]:
            return f"""
Flight search results from {leg['origin']} ({leg['origin_code']}) to {leg['destination']} ({leg['dest_code']}):

Flight data in JSON format:
{json.dumps(flight_data, indent=2)}

Present these {len(leg['flights'])} flight(s) to the user clearly."""
        else:
            error = leg.get("error") or "No direct flights found"
            return f"""
Flight search from {leg['origin']} ({leg['origin_code']}) to {leg['destination']} ({leg['dest_code']}):

Result: {error}

Let the user know and suggest alternatives if appropriate."""

    route_str = " → ".join(
        [f"{city} ({code})" for city, code in zip(cities, airport_codes)]
    )
    context = f"\nMulti-leg flight search: {route_str}\n\n"

    for leg in legs_data:
        context += f"**Leg {leg['leg']}: {leg['origin']} ({leg['origin_code']}) → {leg['destination']} ({leg['dest_code']})**\n"
        if leg["flights"]:
            leg_data = {"flights": leg["flights"], "count": len(leg["flights"])}
            context += f"Flight data:\n{json.dumps(leg_data, indent=2)}\n\n"
        elif leg.get("error"):
            context += f"Error: {leg['error']}\n\n"
        else:
            context += "No direct flights found for this leg.\n\n"

    context += "Present this itinerary clearly. For each leg, show available flights by departure time. Note connection timing between legs."
    return context


app = FastAPI(title="Flight Information Agent", version="1.0.0")


@app.post("/v1/chat/completions")
async def handle_request(request: Request):
    request_body = await request.json()
    return StreamingResponse(
        invoke_flight_agent(request, request_body),
        media_type="text/event-stream",
    )


async def invoke_flight_agent(request: Request, request_body: dict):
    messages = request_body.get("messages", [])

    route = await extract_flight_route(messages, request)
    cities = route.get("cities", [])
    travel_date = route.get("date")

    # Build context based on what we could extract
    if len(cities) < 2:
        flight_context = """
Could not extract a complete flight route from the user's request.

Ask the user to provide both origin and destination cities.
Example: 'Flights from Seattle to Atlanta' or 'Seattle to Dubai to Lahore'"""
        airport_codes = []
        legs_data = []
    else:
        airport_codes = []
        failed_city = None
        for city in cities:
            code = await resolve_airport_code(city, request)
            if not code:
                failed_city = city
                break
            airport_codes.append(code)

        if failed_city:
            flight_context = f"""
Could not find airport code for "{failed_city}".

Ask the user to check the city name or provide a different city."""
            legs_data = []
        else:
            legs_data = []
            for i in range(len(cities) - 1):
                flight_data = await fetch_flights(
                    airport_codes[i], airport_codes[i + 1], travel_date
                )
                legs_data.append(
                    {
                        "leg": i + 1,
                        "origin": cities[i],
                        "origin_code": airport_codes[i],
                        "destination": cities[i + 1],
                        "dest_code": airport_codes[i + 1],
                        "flights": flight_data.get("flights", []),
                        "error": flight_data.get("error"),
                    }
                )

            flight_context = build_flight_context(cities, airport_codes, legs_data)

    response_messages = [{"role": "system", "content": SYSTEM_PROMPT}]
    for i, msg in enumerate(messages):
        content = msg.get("content", "")
        if i == len(messages) - 1 and msg.get("role") == "user":
            content += flight_context
        response_messages.append({"role": msg.get("role"), "content": content})

    logger.info(f"Sending {len(response_messages)} messages to LLM")

    try:
        ctx = extract(request.headers)
        extra_headers = {"x-envoy-max-retries": "3"}
        inject(extra_headers, context=ctx)

        stream = await openai_client.chat.completions.create(
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
        logger.error(f"Error generating response: {e}")
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
