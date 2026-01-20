import json
import os
import logging
import time
import uuid
import httpx
import uvicorn
from datetime import datetime
from typing import Optional

from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse, StreamingResponse
from openai import AsyncOpenAI
from opentelemetry.propagate import extract, inject
from crewai import Agent, Task, Crew, LLM
from crewai.tools import tool

from openai_protocol import create_chat_completion_chunk

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


SYSTEM_PROMPT = """You are a travel planning assistant specializing in flight information and travel conditions.

CRITICAL: You MUST respond with ONLY the final answer to the user.

DO NOT OUTPUT:
- "Thought:" or any internal thinking
- "Action:" or tool names
- "Action Input:" or parameters
- "Observation:" or tool results
- Any reasoning steps, planning, or internal deliberation

FORMATTING RULES:
- Respond in natural, conversational text ONLY
- NEVER use JSON, code blocks, or technical formats
- Present flight information in a clean, bullet-point list
- Use plain text with proper spacing and line breaks

Flight Information Format:
- Airline Name (Flight Number)
- Departure: Time from Airport Code, Gate info
- Arrival: Time at Airport Code
- Aircraft: Model name
- Status: Current status

Weather Information (when available):
- Present weather data in a clear, readable format
- Include temperature, conditions, and any travel advisories
- Integrate weather context with flight information naturally
- Mention how weather might affect travel plans if relevant

Your task:
1. Use tools silently (don't mention them to the user)
2. Convert technical data into friendly, readable text
3. Use 12-hour time format (e.g., "9:00 AM")
4. Organize flights chronologically by departure time
5. Include terminal/gate info when available
6. When weather data is provided, summarize it clearly and relate it to the travel plans
7. NOTE (Multi-agent context): If the conversation includes information from other sources (weather, hotels, etc.), incorporate it naturally and cohesively in your response."""


def build_flight_crew(
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

    @tool("resolve_airport_code")
    async def resolve_airport_code_tool(city_name: str) -> str:
        """Convert a city name to its primary airport IATA code.

        Args:
            city_name: Name of the city (e.g., 'Seattle', 'Atlanta', 'Karachi', 'Dubai')

        Returns:
            3-letter IATA airport code (e.g., 'SEA', 'ATL', 'KHI', 'DXB')

        Examples:
            Seattle → SEA
            Atlanta → ATL
            New York → JFK
            Dubai → DXB
            Karachi → KHI
            Lahore → LHE
        """
        code = await resolve_airport_code(city_name, request)
        if not code:
            return f"Error: Could not resolve airport code for '{city_name}'"
        return code

    @tool("search_flights")
    async def search_flights(
        origin_code: str, destination_code: str, travel_date: Optional[str] = None
    ):
        """Search for flights between two airports using their IATA codes.

        Args:
            origin_code: Origin airport IATA code (3 letters, e.g., 'SEA', 'KHI')
            destination_code: Destination airport IATA code (3 letters, e.g., 'ATL', 'DXB')
            travel_date: Travel date in YYYY-MM-DD format. If not provided, defaults to TODAY.

        Note: Flight data is only available for today and up to 2 days ahead.

        IMPORTANT: Use the resolve_airport_code tool first if you only have city names.
        """
        # Default to today's date if not provided
        if not travel_date:
            travel_date = datetime.now().strftime("%Y-%m-%d")

        # Validate that we have proper IATA codes (3 letters)
        if len(origin_code) != 3 or len(destination_code) != 3:
            return {
                "error": f"Invalid airport codes. Expected 3-letter IATA codes, got origin='{origin_code}' and destination='{destination_code}'. Use resolve_airport_code tool first to convert city names to codes.",
            }

        flight_data = await fetch_flights(origin_code, destination_code, travel_date)
        return {
            "origin_code": origin_code,
            "destination_code": destination_code,
            "travel_date": travel_date,
            "flights": flight_data.get("flights", []),
            "count": flight_data.get("count", 0),
            "error": flight_data.get("error"),
        }

    llm = LLM(
        model=FLIGHT_MODEL,
        api_key="EMPTY",
        base_url=LLM_GATEWAY_ENDPOINT,
        temperature=request_body.get("temperature", 0.7),
        max_tokens=request_body.get("max_tokens", 1000),
        stream=streaming,
        extra_headers=extra_headers,
    )

    agent = Agent(
        role="Flight Information Specialist",
        goal="Provide accurate, clear flight options and details for travelers.",
        backstory=SYSTEM_PROMPT,
        tools=[resolve_airport_code_tool, search_flights],
        llm=llm,
        verbose=True,
        reasoning=False,
    )

    task = Task(
        description=(
            "Answer the user's request based on this conversation:\n{conversation}\n\n"
            "CRITICAL: NOTE you are part of a multi-agent setup, so if the conversation includes information from other sources that are not flight-related, incorporate it naturally.\n"
            "Output ONLY your final answer to the user. Do NOT show:\n"
            "- Thought, Action, Action Input, Observation, or any reasoning steps\n"
            "- Tool names, parameters, or results\n"
            "- Planning or internal deliberation\n\n"
            "Tool workflow (execute silently):\n"
            "1. City names → use resolve_airport_code to get IATA codes\n"
            "2. Use search_flights with the codes\n"
            "3. Present results conversationally\n\n"
            "Output requirements:\n"
            "- Natural conversational text only\n"
            "- NO JSON, code blocks, or technical formatting\n"
            "- Clean bullet points with readable times (9:00 AM format)\n"
            "- Direct answer with no reasoning shown"
        ),
        expected_output=(
            "A direct answer to the user in plain text with flight options. "
            "NO Thought/Action/Observation. NO code blocks. NO JSON. "
            "Just natural language with bullet points."
        ),
        agent=agent,
    )

    return Crew(agents=[agent], tasks=[task], stream=streaming, verbose=False)


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
    """Generate flight information using a CrewAI agent."""
    messages = request_body.get("messages", [])
    crew = build_flight_crew(request, request_body, streaming=False)
    conversation = json.dumps(messages, indent=2)

    try:
        result = crew.kickoff(inputs={"conversation": conversation})
        if hasattr(result, "raw"):
            return result.raw
        return str(result)
    except Exception as e:
        logger.error(f"Error generating response: {e}")
        return "I'm having trouble retrieving flight information right now. Please try again."


async def invoke_flight_agent_stream(
    request: Request,
    request_body: dict,
    model: str,
):
    messages = request_body.get("messages", [])
    crew = build_flight_crew(request, request_body, streaming=True)
    conversation = json.dumps(messages, indent=2)

    try:
        streaming = crew.kickoff(inputs={"conversation": conversation})
        for chunk in streaming:
            content = getattr(chunk, "content", None)
            if content is None:
                content = str(chunk)
            if not content:
                continue
            yield f"data: {create_chat_completion_chunk(model, content).model_dump_json()}\n\n"

        yield f"data: {create_chat_completion_chunk(model, '', 'stop').model_dump_json()}\n\n"
        yield "data: [DONE]\n\n"
    except Exception as e:
        logger.error(f"Error streaming response: {e}")
        error_message = "I'm having trouble retrieving flight information right now. Please try again."
        yield f"data: {create_chat_completion_chunk(model, error_message, 'stop').model_dump_json()}\n\n"
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
