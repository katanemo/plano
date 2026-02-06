import json
import re
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
from urllib.parse import quote
from opentelemetry.propagate import extract, inject

# Set up logging
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - [WEATHER_AGENT] - %(levelname)s - %(message)s",
)
logger = logging.getLogger(__name__)


# Configuration for plano LLM gateway
LLM_GATEWAY_ENDPOINT = os.getenv(
    "LLM_GATEWAY_ENDPOINT", "http://host.docker.internal:12001/v1"
)
WEATHER_MODEL = "openai/gpt-5.2"
LOCATION_MODEL = "openai/gpt-4o-mini"

# Initialize OpenAI client for plano
openai_client_via_plano = AsyncOpenAI(
    base_url=LLM_GATEWAY_ENDPOINT,
    api_key="EMPTY",
)

# FastAPI app for REST server
app = FastAPI(title="Weather Forecast Agent", version="1.0.0")

# HTTP client for API calls
http_client = httpx.AsyncClient(timeout=10.0)


# Utility functions
def celsius_to_fahrenheit(temp_c: Optional[float]) -> Optional[float]:
    """Convert Celsius to Fahrenheit."""
    return round(temp_c * 9 / 5 + 32, 1) if temp_c is not None else None


def get_user_messages(messages: list) -> list:
    """Extract user messages from message list."""
    return [msg for msg in messages if msg.get("role") == "user"]


def get_last_user_content(messages: list) -> str:
    """Get the content of the most recent user message."""
    for msg in reversed(messages):
        if msg.get("role") == "user":
            return msg.get("content", "").lower()
    return ""


async def get_weather_data(
    request: Request,
    messages: list,
    days: int = 1,
    traceparent_header: str = None,
    request_id: str = None,
):
    """Extract location from user's conversation and fetch weather data from Open-Meteo API.

    This function does two things:
    1. Uses an LLM to extract the location from the user's message
    2. Fetches weather data for that location from Open-Meteo

    Currently returns only current day weather. Want to add multi-day forecasts?
    """
    instructions = """You are a city name extractor. Look at the FINAL user message ONLY and extract the city name.

The FINAL user message will be the LAST message with role "user" in the conversation.

IMPORTANT: Ignore all previous messages. Focus ONLY on the FINAL user message.

Examples of what to extract from the FINAL user message:
- "What's the weather in Seattle?" → Seattle
- "What's the weather in San Francisco?" → San Francisco
- "What about Dubai?" → Dubai
- "How's the weather in Tokyo today?" → Tokyo
- "Tell me about Lahore" → Lahore
- "What about there?" → Look at conversation for the last mentioned city

Output ONLY the city name. Nothing else. One word or city name only.
If no city can be found, output: NOT_FOUND"""

    try:
        user_messages = [
            msg.get("content") for msg in messages if msg.get("role") == "user"
        ]

        if not user_messages:
            location = "New York"
        else:
            ctx = extract(request.headers)
            extra_headers = {}
            if request_id:
                extra_headers["x-request-id"] = request_id
            inject(extra_headers, context=ctx)
            # For location extraction, pass full conversation for context (e.g., "there" = previous destination)
            response = await openai_client_via_plano.chat.completions.create(
                model=LOCATION_MODEL,
                messages=[
                    {"role": "system", "content": instructions},
                    *[
                        {"role": msg.get("role"), "content": msg.get("content")}
                        for msg in messages
                    ],
                ],
                temperature=0.1,
                max_completion_tokens=10,
                extra_headers=extra_headers if extra_headers else None,
            )

            location = response.choices[0].message.content.strip().strip("\"'`.,!?")
            logger.info(f"Location extraction result: '{location}'")

            if not location or location.upper() == "NOT_FOUND":
                location = "New York"
                logger.info(f"Location not found, defaulting to: {location}")

    except Exception as e:
        logger.error(f"Error extracting location: {e}")
        location = "New York"

    logger.info(f"Fetching weather for location: '{location}' (days: {days})")

    # Step 2: Fetch weather data for the extracted location
    try:
        # Geocode city to get coordinates
        geocode_url = f"https://geocoding-api.open-meteo.com/v1/search?name={quote(location)}&count=1&language=en&format=json"
        geocode_response = await http_client.get(geocode_url)

        if geocode_response.status_code != 200 or not geocode_response.json().get(
            "results"
        ):
            logger.warning(f"Could not geocode {location}, using New York")
            location = "New York"
            geocode_url = f"https://geocoding-api.open-meteo.com/v1/search?name={quote(location)}&count=1&language=en&format=json"
            geocode_response = await http_client.get(geocode_url)

        geocode_data = geocode_response.json()
        if not geocode_data.get("results"):
            return {
                "location": location,
                "weather": {
                    "date": datetime.now().strftime("%Y-%m-%d"),
                    "day_name": datetime.now().strftime("%A"),
                    "temperature_c": None,
                    "temperature_f": None,
                    "weather_code": None,
                    "error": "Could not retrieve weather data",
                },
            }

        result = geocode_data["results"][0]
        location_name = result.get("name", location)
        latitude = result["latitude"]
        longitude = result["longitude"]

        logger.info(
            f"Geocoded '{location}' to {location_name} ({latitude}, {longitude})"
        )

        # Get weather forecast
        weather_url = (
            f"https://api.open-meteo.com/v1/forecast?"
            f"latitude={latitude}&longitude={longitude}&"
            f"current=temperature_2m&"
            f"daily=sunrise,sunset,temperature_2m_max,temperature_2m_min,weather_code&"
            f"forecast_days={days}&timezone=auto"
        )

        weather_response = await http_client.get(weather_url)
        if weather_response.status_code != 200:
            return {
                "location": location_name,
                "weather": {
                    "date": datetime.now().strftime("%Y-%m-%d"),
                    "day_name": datetime.now().strftime("%A"),
                    "temperature_c": None,
                    "temperature_f": None,
                    "weather_code": None,
                    "error": "Could not retrieve weather data",
                },
            }

        weather_data = weather_response.json()
        current_temp = weather_data.get("current", {}).get("temperature_2m")
        daily = weather_data.get("daily", {})

        # Build forecast for requested number of days
        forecast = []
        for i in range(days):
            date_str = daily["time"][i]
            date_obj = datetime.fromisoformat(date_str.replace("Z", "+00:00"))

            temp_max = (
                daily.get("temperature_2m_max", [])[i]
                if daily.get("temperature_2m_max")
                else None
            )
            temp_min = (
                daily.get("temperature_2m_min", [])[i]
                if daily.get("temperature_2m_min")
                else None
            )
            weather_code = (
                daily.get("weather_code", [0])[i] if daily.get("weather_code") else 0
            )
            sunrise = daily.get("sunrise", [])[i] if daily.get("sunrise") else None
            sunset = daily.get("sunset", [])[i] if daily.get("sunset") else None

            # Use current temp for today, otherwise use max temp
            temp_c = (
                temp_max
                if temp_max is not None
                else (current_temp if i == 0 and current_temp else temp_min)
            )

            forecast.append(
                {
                    "date": date_str.split("T")[0],
                    "day_name": date_obj.strftime("%A"),
                    "temperature_c": round(temp_c, 1) if temp_c is not None else None,
                    "temperature_f": celsius_to_fahrenheit(temp_c),
                    "temperature_max_c": (
                        round(temp_max, 1) if temp_max is not None else None
                    ),
                    "temperature_min_c": (
                        round(temp_min, 1) if temp_min is not None else None
                    ),
                    "weather_code": weather_code,
                    "sunrise": sunrise.split("T")[1] if sunrise else None,
                    "sunset": sunset.split("T")[1] if sunset else None,
                }
            )

        return {"location": location_name, "forecast": forecast}

    except Exception as e:
        logger.error(f"Error getting weather data: {e}")
        return {
            "location": location,
            "weather": {
                "date": datetime.now().strftime("%Y-%m-%d"),
                "day_name": datetime.now().strftime("%A"),
                "temperature_c": None,
                "temperature_f": None,
                "weather_code": None,
                "error": "Could not retrieve weather data",
            },
        }


@app.post("/v1/chat/completions")
async def handle_request(request: Request):
    """HTTP endpoint for chat completions with streaming support."""

    request_body = await request.json()
    messages = request_body.get("messages", [])
    # Respect the stream parameter - orchestrator controls this based on agent position in chain
    is_streaming = request_body.get("stream", True)

    logger.info(
        "messages detail json dumps: %s",
        json.dumps(messages, indent=2),
    )

    traceparent_header = request.headers.get("traceparent")
    request_id = request.headers.get("x-request-id")

    return StreamingResponse(
        invoke_weather_agent(request, request_body, traceparent_header, request_id),
        media_type="text/plain",
        headers={
            "content-type": "text/event-stream",
        },
    )


async def invoke_weather_agent(
    request: Request,
    request_body: dict,
    traceparent_header: str = None,
    request_id: str = None,
):
    """Generate streaming chat completions."""
    messages = request_body.get("messages", [])

    # Detect if user wants multi-day forecast
    last_user_msg = get_last_user_content(messages)
    days = 1

    if "forecast" in last_user_msg or "week" in last_user_msg:
        days = 7
    elif "tomorrow" in last_user_msg:
        days = 2

    # Extract specific number of days if mentioned (e.g., "5 day forecast")
    import re

    day_match = re.search(r"(\d{1,2})\s+day", last_user_msg)
    if day_match:
        requested_days = int(day_match.group(1))
        days = min(requested_days, 16)  # API supports max 16 days

    # Get live weather data (location extraction happens inside this function)
    weather_data = await get_weather_data(
        request, messages, days, traceparent_header, request_id
    )

    # Create weather context to append to user message
    forecast_type = "forecast" if days > 1 else "current weather"
    weather_context = f"""

Weather data for {weather_data['location']} ({forecast_type}):
{json.dumps(weather_data, indent=2)}

Present the weather information to the user in a clear, readable format. If there is information from other agents, start your response with a summary of that information."""

    # System prompt for weather agent
    instructions = """You are a weather assistant in a multi-agent system. You will receive weather data in JSON format with these fields:

    - "location": City name
    - "forecast": Array of weather objects, each with date, day_name, temperature_c, temperature_f, temperature_max_c, temperature_min_c, weather_code, sunrise, sunset
    - weather_code: WMO code (0=clear, 1-3=partly cloudy, 45-48=fog, 51-67=rain, 71-86=snow, 95-99=thunderstorm)

    Your task:
    1. Present the weather/forecast clearly for the location
    2. For single day: show current conditions
    3. For multi-day: show each day with date and conditions
    4. Include temperature in both Celsius and Fahrenheit
    5. Describe conditions naturally based on weather_code
    6. Use conversational language

    Multi-agent context: You are part of a larger system. If the conversation includes additional context or information from other sources, acknowledge and incorporate it naturally into your response. Your primary focus is weather, but be aware of the full conversation context.

    Remember: Only use the provided data. If fields are null, mention data is unavailable."""

    # Build message history with weather data appended to the last user message
    response_messages = [{"role": "system", "content": instructions}]

    for i, msg in enumerate(messages):
        # Append weather data to the last user message
        if i == len(messages) - 1 and msg.get("role") == "user":
            response_messages.append(
                {"role": "user", "content": msg.get("content") + weather_context}
            )
        else:
            response_messages.append(
                {"role": msg.get("role"), "content": msg.get("content")}
            )

    try:
        ctx = extract(request.headers)
        extra_headers = {"x-envoy-max-retries": "3"}
        if request_id:
            extra_headers["x-request-id"] = request_id
        inject(extra_headers, context=ctx)

        stream = await openai_client_via_plano.chat.completions.create(
            model=WEATHER_MODEL,
            messages=response_messages,
            temperature=request_body.get("temperature", 0.7),
            max_completion_tokens=request_body.get("max_tokens", 3000),
            stream=True,
            extra_headers=extra_headers,
        )

        async for chunk in stream:
            if chunk.choices:
                yield f"data: {chunk.model_dump_json()}\n\n"

        yield "data: [DONE]\n\n"

    except Exception as e:
        logger.error(f"Error generating weather response: {e}")
        error_chunk = {
            "id": f"chatcmpl-{uuid.uuid4().hex[:8]}",
            "object": "chat.completion.chunk",
            "created": int(time.time()),
            "model": request_body.get("model", WEATHER_MODEL),
            "choices": [
                {
                    "index": 0,
                    "delta": {
                        "content": "I apologize, but I'm having trouble retrieving weather information right now. Please try again."
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
    return {"status": "healthy", "agent": "weather_forecast"}


def start_server(host: str = "localhost", port: int = 10510):
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
                    "format": "%(asctime)s - [WEATHER_AGENT] - %(levelname)s - %(message)s",
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
    start_server(host="0.0.0.0", port=10510)
