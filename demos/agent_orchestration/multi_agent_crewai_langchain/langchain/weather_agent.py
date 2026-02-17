import json
import logging
import os
import time
import uuid
from datetime import datetime
from typing import Optional
from urllib.parse import quote

import httpx
import uvicorn
from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse, StreamingResponse
from langchain.agents import create_agent
from langchain_core.tools import tool
from langchain_openai import ChatOpenAI
from openai import AsyncOpenAI
from opentelemetry.propagate import extract, inject
from pydantic import BaseModel, Field

from openai_protocol import create_chat_completion_chunk

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - [WEATHER_AGENT] - %(levelname)s - %(message)s",
)
logger = logging.getLogger(__name__)

LLM_GATEWAY_ENDPOINT = os.getenv(
    "LLM_GATEWAY_ENDPOINT", "http://host.docker.internal:12000/v1"
)
WEATHER_MODEL = "gpt-4o"
LOCATION_MODEL = "gpt-4o-mini"

openai_client_via_plano = AsyncOpenAI(
    base_url=LLM_GATEWAY_ENDPOINT,
    api_key="EMPTY",
)

app = FastAPI(title="Weather Forecast Agent", version="1.0.0")

http_client = httpx.AsyncClient(timeout=10.0)


def celsius_to_fahrenheit(temp_c: Optional[float]) -> Optional[float]:
    return round(temp_c * 9 / 5 + 32, 1) if temp_c is not None else None


async def get_weather_data(
    request: Request,
    messages: list,
    days: int = 1,
    request_id: str = None,
    city_override: Optional[str] = None,
):
    instructions = """You are a city name extractor. Look at the FINAL user message ONLY and extract the city name.

    The FINAL user message will be the LAST message with role "user" in the conversation.

    IMPORTANT: Ignore all previous messages. Focus ONLY on the FINAL user message.

    Examples of what to extract from the FINAL user message:
    - "What's the weather in Seattle?" -> Seattle
    - "What's the weather in San Francisco?" -> San Francisco
    - "What about Dubai?" -> Dubai
    - "How's the weather in Tokyo today?" -> Tokyo
    - "Tell me about Lahore" -> Lahore
    - "What about there?" -> Look at conversation for the last mentioned city

    Output ONLY the city name. Nothing else. One word or city name only.
    If no city can be found, output: NOT_FOUND"""

    location = city_override
    if not location:
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
                    max_tokens=10,
                    extra_headers=extra_headers if extra_headers else None,
                )

                location = response.choices[0].message.content.strip().strip("\"'`.,!?")

                if not location or location.upper() == "NOT_FOUND":
                    location = "New York"
                    logger.info("Location not found, defaulting to: %s", location)

        except Exception as e:
            logger.error("Error extracting location: %s", e)
            location = "New York"

    logger.info("Fetching weather for location: '%s' (days: %s)", location, days)

    try:
        geocode_url = (
            "https://geocoding-api.open-meteo.com/v1/search?"
            f"name={quote(location)}&count=1&language=en&format=json"
        )
        geocode_response = await http_client.get(geocode_url)

        if geocode_response.status_code != 200 or not geocode_response.json().get(
            "results"
        ):
            logger.warning("Could not geocode %s, using New York", location)
            location = "New York"
            geocode_url = (
                "https://geocoding-api.open-meteo.com/v1/search?"
                f"name={quote(location)}&count=1&language=en&format=json"
            )
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
            "Geocoded '%s' to %s (%s, %s)", location, location_name, latitude, longitude
        )

        weather_url = (
            "https://api.open-meteo.com/v1/forecast?"
            f"latitude={latitude}&longitude={longitude}&"
            "current=temperature_2m&"
            "daily=sunrise,sunset,temperature_2m_max,temperature_2m_min,weather_code&"
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
        logger.error("Error getting weather data: %s", e)
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


class WeatherToolInput(BaseModel):
    city: str = Field(..., description="City name to look up weather for")
    days: int = Field(
        1,
        ge=1,
        le=16,
        description="Number of forecast days (1-16). Defaults to 1 (current).",
    )


WEATHER_SYSTEM_PROMPT = """You are a weather and travel conditions assistant in a multi-agent system. You will receive weather data in JSON format with these fields:

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
    7. When flight information is present in the conversation, summarize it clearly:
       - Present flight details in a readable format (airline, times, gates, status)
       - Integrate flight and weather information cohesively
       - Mention how weather might affect the flights if relevant
    8. NOTE (Multi-agent context): If the conversation includes information from other agents and sources (flights, hotels, etc.), incorporate it naturally and provide a comprehensive travel summary.

    Remember: Only use the provided data. If fields are null, mention data is unavailable."""


def build_weather_agent(
    request: Request,
    request_body: dict,
    streaming: bool,
):
    messages = request_body.get("messages", [])
    ctx = extract(request.headers)
    extra_headers = {"x-envoy-max-retries": "3"}
    request_id = request.headers.get("x-request-id")
    if request_id:
        extra_headers["x-request-id"] = request_id
        logger.debug("Request ID set: [redacted]")
    inject(extra_headers, context=ctx)

    @tool("get_weather_forecast", args_schema=WeatherToolInput)
    async def get_weather_forecast(city: str, days: int = 1):
        """Fetch a structured weather forecast for a city."""
        return await get_weather_data(
            request,
            messages,
            days,
            request_id=request_id,
            city_override=city,
        )

    llm = ChatOpenAI(
        model=WEATHER_MODEL,
        api_key="EMPTY",
        base_url=LLM_GATEWAY_ENDPOINT,
        temperature=request_body.get("temperature", 0.7),
        max_tokens=request_body.get("max_tokens", 1000),
        streaming=streaming,
        default_headers=extra_headers,
    )

    return create_agent(
        model=llm,
        tools=[get_weather_forecast],
        system_prompt=WEATHER_SYSTEM_PROMPT,
    )


@app.post("/v1/chat/completions")
async def handle_request(request: Request):
    request_body = await request.json()
    is_streaming = request_body.get("stream", True)

    try:
        model = request_body.get("model", WEATHER_MODEL)

        if is_streaming:
            return StreamingResponse(
                invoke_weather_agent_stream(request, request_body, model),
                media_type="text/event-stream",
                headers={"content-type": "text/event-stream"},
            )

        content = await invoke_weather_agent(request, request_body)
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
    except Exception as e:
        logger.error("Error generating weather response: %s", e)
        if is_streaming:
            return StreamingResponse(
                invoke_weather_agent_error_stream(
                    request_body,
                    "I'm having trouble retrieving weather information right now. Please try again.",
                ),
                media_type="text/event-stream",
                headers={"content-type": "text/event-stream"},
            )
        return JSONResponse(
            {
                "error": {
                    "message": "I'm having trouble retrieving weather information right now. Please try again.",
                    "type": "server_error",
                }
            },
            status_code=500,
        )


async def invoke_weather_agent(
    request: Request,
    request_body: dict,
):
    messages = request_body.get("messages", [])
    agent = build_weather_agent(request, request_body, streaming=False)

    result = await agent.ainvoke({"messages": messages})
    final_message = result["messages"][-1]
    return (
        final_message.content
        if hasattr(final_message, "content")
        else str(final_message)
    )


async def invoke_weather_agent_stream(
    request: Request,
    request_body: dict,
    model: str,
):
    messages = request_body.get("messages", [])
    agent = build_weather_agent(request, request_body, streaming=True)

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
            yield f"data: {create_chat_completion_chunk(model, content).model_dump_json()}\n\n"

        yield f"data: {create_chat_completion_chunk(model, '', 'stop').model_dump_json()}\n\n"
        yield "data: [DONE]\n\n"
    except Exception as e:
        logger.error("Error streaming weather response: %s", e)
        error_message = "I'm having trouble retrieving weather information right now. Please try again."
        yield f"data: {create_chat_completion_chunk(model, error_message, 'stop').model_dump_json()}\n\n"
        yield "data: [DONE]\n\n"


async def invoke_weather_agent_error_stream(request_body: dict, error_message: str):
    model = request_body.get("model", WEATHER_MODEL)
    yield f"data: {create_chat_completion_chunk(model, error_message, 'stop').model_dump_json()}\n\n"
    yield "data: [DONE]\n\n"


@app.get("/health")
async def health_check():
    return {"status": "healthy", "agent": "weather_forecast"}


def start_server(host: str = "localhost", port: int = 10510):
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
    start_server(host="0.0.0.0", port=10510)
