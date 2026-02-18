"""Mock-based tests for the OpenAI Responses API (/v1/responses).

Tests translation to chat completions via the gateway, tool calling,
streaming, mixed content types, and multi-turn state management.

Note: The gateway translates all Responses API requests to /v1/chat/completions
on the upstream when using base_url-configured providers. Direct /v1/responses
passthrough is tested by the live e2e tests on main/nightly.

These tests require the gateway to be running with config_mock_llm.yaml
(started via docker-compose.mock.yaml).
"""

import openai
import logging

from pytest_httpserver import HTTPServer

from conftest import (
    setup_openai_chat_mock,
)

logger = logging.getLogger(__name__)

LLM_GATEWAY_BASE = "http://localhost:12000"


# =============================================================================
# NON-STREAMING TESTS
# =============================================================================


def test_responses_api_non_streaming(httpserver: HTTPServer):
    """Responses API non-streaming → translated to /v1/chat/completions"""
    captured = setup_openai_chat_mock(httpserver, content="Hello from Responses API!")

    client = openai.OpenAI(api_key="test-key", base_url=f"{LLM_GATEWAY_BASE}/v1")
    resp = client.responses.create(
        model="claude-sonnet-4-20250514",
        input="Hello via responses API",
    )

    assert resp is not None
    assert resp.id is not None
    assert len(resp.output_text) > 0


def test_responses_api_non_streaming_openai_model(httpserver: HTTPServer):
    """Responses API non-streaming with OpenAI model → translated to /v1/chat/completions"""
    captured = setup_openai_chat_mock(
        httpserver, content="Hello from GPT via Responses!"
    )

    client = openai.OpenAI(api_key="test-key", base_url=f"{LLM_GATEWAY_BASE}/v1")
    resp = client.responses.create(
        model="gpt-4o",
        input="Hello via responses API",
    )

    assert resp is not None
    assert resp.id is not None
    assert len(resp.output_text) > 0


# =============================================================================
# STREAMING TESTS
# =============================================================================


def test_responses_api_streaming(httpserver: HTTPServer):
    """Responses API streaming → translated to /v1/chat/completions"""
    setup_openai_chat_mock(httpserver, content="Streaming from Responses API!")

    client = openai.OpenAI(api_key="test-key", base_url=f"{LLM_GATEWAY_BASE}/v1")
    stream = client.responses.create(
        model="claude-sonnet-4-20250514",
        input="Write a haiku",
        stream=True,
    )

    text_chunks = []
    for event in stream:
        if getattr(event, "type", None) == "response.output_text.delta" and getattr(
            event, "delta", None
        ):
            text_chunks.append(event.delta)

    assert len(text_chunks) > 0, "Should have received streaming text deltas"


def test_responses_api_streaming_openai_model(httpserver: HTTPServer):
    """Responses API streaming with OpenAI model → translated to /v1/chat/completions"""
    setup_openai_chat_mock(httpserver, content="Streaming from GPT via Responses!")

    client = openai.OpenAI(api_key="test-key", base_url=f"{LLM_GATEWAY_BASE}/v1")
    stream = client.responses.create(
        model="gpt-4o",
        input="Write a haiku",
        stream=True,
    )

    text_chunks = []
    for event in stream:
        if getattr(event, "type", None) == "response.output_text.delta" and getattr(
            event, "delta", None
        ):
            text_chunks.append(event.delta)

    assert len(text_chunks) > 0, "Should have received streaming text deltas"


# =============================================================================
# TOOL CALLING TESTS
# =============================================================================


def test_responses_api_with_tools(httpserver: HTTPServer):
    """Responses API with tools → translated to /v1/chat/completions"""
    setup_openai_chat_mock(httpserver, content="Tool response via Claude")

    client = openai.OpenAI(api_key="test-key", base_url=f"{LLM_GATEWAY_BASE}/v1")
    tools = [
        {
            "type": "function",
            "name": "echo_tool",
            "description": "Echo back the provided input: hello_world",
            "parameters": {
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"],
            },
        }
    ]

    resp = client.responses.create(
        model="claude-sonnet-4-20250514",
        input="Call the echo tool",
        tools=tools,
    )

    assert resp.id is not None


def test_responses_api_streaming_with_tools(httpserver: HTTPServer):
    """Responses API streaming with tools → translated to /v1/chat/completions"""
    setup_openai_chat_mock(httpserver, content="Streamed tool via Claude")

    client = openai.OpenAI(
        api_key="test-key", base_url=f"{LLM_GATEWAY_BASE}/v1", max_retries=0
    )
    tools = [
        {
            "type": "function",
            "name": "echo_tool",
            "description": "Echo back the provided input: hello_world",
            "parameters": {
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"],
            },
        }
    ]

    stream = client.responses.create(
        model="claude-sonnet-4-20250514",
        input="Call the echo tool with hello_world",
        tools=tools,
        stream=True,
    )

    text_chunks = []
    tool_calls = []
    for event in stream:
        etype = getattr(event, "type", None)
        if etype == "response.output_text.delta" and getattr(event, "delta", None):
            text_chunks.append(event.delta)
        if etype == "response.function_call_arguments.delta" and getattr(
            event, "delta", None
        ):
            tool_calls.append(event.delta)

    assert text_chunks or tool_calls, "Expected streamed text or tool call deltas"


# =============================================================================
# MIXED CONTENT TYPES
# =============================================================================


def test_responses_api_mixed_content_types(httpserver: HTTPServer):
    """Responses API with mixed content types (string and array) in input"""
    setup_openai_chat_mock(httpserver, content="Weather Seattle")

    client = openai.OpenAI(api_key="test-key", base_url=f"{LLM_GATEWAY_BASE}/v1")
    resp = client.responses.create(
        model="claude-sonnet-4-20250514",
        input=[
            {
                "role": "developer",
                "content": "Generate a short chat title based on the user's message.",
            },
            {
                "role": "user",
                "content": [
                    {"type": "input_text", "text": "What is the weather in Seattle"}
                ],
            },
        ],
    )

    assert resp is not None
    assert resp.id is not None
    assert len(resp.output_text) > 0


# =============================================================================
# STATE MANAGEMENT (multi-turn via previous_response_id)
# =============================================================================


def test_conversation_state_management_two_turn(httpserver: HTTPServer):
    """Two-turn conversation using previous_response_id for state management.

    Turn 1: Send initial message → get response_id
    Turn 2: Send with previous_response_id → verify state was combined
    """
    captured = setup_openai_chat_mock(
        httpserver, content="I remember your name is Alice!"
    )

    client = openai.OpenAI(api_key="test-key", base_url=f"{LLM_GATEWAY_BASE}/v1")

    # Turn 1
    resp1 = client.responses.create(
        model="claude-sonnet-4-20250514",
        input="My name is Alice and I like pizza.",
    )
    response_id_1 = resp1.id
    assert response_id_1 is not None
    assert len(resp1.output_text) > 0

    # Turn 2 with previous_response_id
    resp2 = client.responses.create(
        model="claude-sonnet-4-20250514",
        input="What is my name?",
        previous_response_id=response_id_1,
    )
    response_id_2 = resp2.id
    assert response_id_2 is not None
    assert response_id_2 != response_id_1

    # Verify the upstream received both turns' messages in the second request
    assert len(captured) == 2
    second_request = captured[1]
    messages = second_request.get("messages", [])
    # Should have messages from both turns (user + assistant from turn 1, plus user from turn 2)
    assert (
        len(messages) >= 3
    ), f"Expected >= 3 messages in second turn, got {len(messages)}: {messages}"


def test_conversation_state_management_two_turn_streaming(httpserver: HTTPServer):
    """Two-turn streaming conversation using previous_response_id."""
    captured = setup_openai_chat_mock(httpserver, content="Alice likes pizza!")

    client = openai.OpenAI(api_key="test-key", base_url=f"{LLM_GATEWAY_BASE}/v1")

    # Turn 1: streaming
    stream1 = client.responses.create(
        model="claude-sonnet-4-20250514",
        input="My name is Alice and I like pizza.",
        stream=True,
    )

    text_chunks_1 = []
    response_id_1 = None
    for event in stream1:
        if getattr(event, "type", None) == "response.output_text.delta" and getattr(
            event, "delta", None
        ):
            text_chunks_1.append(event.delta)
        if getattr(event, "type", None) == "response.completed" and getattr(
            event, "response", None
        ):
            response_id_1 = event.response.id

    assert response_id_1 is not None
    assert len(text_chunks_1) > 0

    # Turn 2: streaming with previous_response_id
    stream2 = client.responses.create(
        model="claude-sonnet-4-20250514",
        input="What do I like?",
        previous_response_id=response_id_1,
        stream=True,
    )

    text_chunks_2 = []
    response_id_2 = None
    for event in stream2:
        if getattr(event, "type", None) == "response.output_text.delta" and getattr(
            event, "delta", None
        ):
            text_chunks_2.append(event.delta)
        if getattr(event, "type", None) == "response.completed" and getattr(
            event, "response", None
        ):
            response_id_2 = event.response.id

    assert response_id_2 is not None
    assert response_id_2 != response_id_1
    assert len(text_chunks_2) > 0

    # Verify second turn included first turn's context
    assert len(captured) == 2
    second_request = captured[1]
    messages = second_request.get("messages", [])
    assert (
        len(messages) >= 3
    ), f"Expected >= 3 messages in second turn, got {len(messages)}"
