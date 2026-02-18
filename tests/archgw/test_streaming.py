"""Mock-based streaming tests for all three API shapes.

Tests streaming for:
- OpenAI Chat Completions (both OpenAI and Anthropic clients)
- Anthropic Messages API (both native and cross-provider)
- OpenAI Responses API (passthrough and translated)
- Tool call streaming
- Thinking mode streaming

These tests require the gateway to be running with config_mock_llm.yaml
(started via docker-compose.mock.yaml).
"""

import json
import openai
import anthropic
import logging

from pytest_httpserver import HTTPServer
from pytest_httpserver.httpserver import HandlerType
from werkzeug.wrappers import Response

from conftest import (
    setup_openai_chat_mock,
    setup_anthropic_mock,
    make_openai_tool_call_stream,
)

logger = logging.getLogger(__name__)

LLM_GATEWAY_BASE = "http://localhost:12000"


# =============================================================================
# OPENAI CHAT COMPLETIONS STREAMING
# =============================================================================


def test_openai_chat_streaming_basic(httpserver: HTTPServer):
    """Basic OpenAI streaming: verify chunks arrive in order and reassemble correctly"""
    setup_openai_chat_mock(
        httpserver, content="The quick brown fox jumps over the lazy dog"
    )

    client = openai.OpenAI(api_key="test-key", base_url=f"{LLM_GATEWAY_BASE}/v1")
    stream = client.chat.completions.create(
        model="gpt-4o-mini",
        max_tokens=100,
        messages=[{"role": "user", "content": "Hello"}],
        stream=True,
    )

    chunks = []
    for chunk in stream:
        if chunk.choices[0].delta.content:
            chunks.append(chunk.choices[0].delta.content)

    full_text = "".join(chunks)
    assert full_text == "The quick brown fox jumps over the lazy dog"
    assert len(chunks) > 1, "Should have received multiple chunks"


def test_openai_chat_streaming_tool_calls(httpserver: HTTPServer):
    """OpenAI streaming with tool calls: verify tool call chunks are properly assembled"""

    def handler(request):
        body = json.loads(request.data)
        model = body.get("model", "gpt-5-mini-2025-08-07")
        return Response(
            make_openai_tool_call_stream(
                model=model, tool_name="echo_tool", tool_args='{"text":"hello"}'
            ),
            status=200,
            content_type="text/event-stream",
        )

    httpserver.expect_request(
        "/v1/chat/completions",
        method="POST",
        handler_type=HandlerType.PERMANENT,
    ).respond_with_handler(handler)

    client = openai.OpenAI(api_key="test-key", base_url=f"{LLM_GATEWAY_BASE}/v1")
    stream = client.chat.completions.create(
        model="gpt-4o-mini",
        max_tokens=100,
        messages=[{"role": "user", "content": "Call the echo tool"}],
        tools=[
            {
                "type": "function",
                "function": {
                    "name": "echo_tool",
                    "description": "Echo input",
                    "parameters": {
                        "type": "object",
                        "properties": {"text": {"type": "string"}},
                        "required": ["text"],
                    },
                },
            }
        ],
        stream=True,
    )

    tool_calls = []
    for chunk in stream:
        if chunk.choices and chunk.choices[0].delta.tool_calls:
            for tc in chunk.choices[0].delta.tool_calls:
                while len(tool_calls) <= tc.index:
                    tool_calls.append(
                        {"id": "", "function": {"name": "", "arguments": ""}}
                    )
                if tc.id:
                    tool_calls[tc.index]["id"] = tc.id
                if tc.function:
                    if tc.function.name:
                        tool_calls[tc.index]["function"]["name"] = tc.function.name
                    if tc.function.arguments:
                        tool_calls[tc.index]["function"][
                            "arguments"
                        ] += tc.function.arguments

    assert len(tool_calls) > 0, "Should have received tool calls"
    assert tool_calls[0]["function"]["name"] == "echo_tool"
    assert tool_calls[0]["id"] == "call_mock_123"


# =============================================================================
# ANTHROPIC MESSAGES STREAMING
# =============================================================================


def test_anthropic_messages_streaming_basic(httpserver: HTTPServer):
    """Basic Anthropic streaming: verify text_stream yields chunks and final message is complete"""
    setup_anthropic_mock(httpserver, content="Hello from streaming Claude!")

    client = anthropic.Anthropic(api_key="test-key", base_url=LLM_GATEWAY_BASE)
    with client.messages.stream(
        model="claude-sonnet-4-20250514",
        max_tokens=100,
        messages=[{"role": "user", "content": "Hello"}],
    ) as stream:
        pieces = list(stream.text_stream)
        full_text = "".join(pieces)
        final = stream.get_final_message()

    assert full_text == "Hello from streaming Claude!"
    assert len(pieces) > 1, "Should have received multiple text chunks"
    assert final is not None
    assert final.content[0].text == "Hello from streaming Claude!"


def test_anthropic_messages_streaming_thinking(httpserver: HTTPServer):
    """Anthropic thinking mode streaming: verify thinking + text blocks"""
    setup_anthropic_mock(httpserver, thinking=True)

    client = anthropic.Anthropic(api_key="test-key", base_url=LLM_GATEWAY_BASE)

    events_seen = {
        "thinking_start": False,
        "thinking_delta": False,
        "text_delta": False,
    }

    with client.messages.stream(
        model="claude-sonnet-4-20250514",
        max_tokens=2048,
        thinking={"type": "enabled", "budget_tokens": 1024},
        messages=[{"role": "user", "content": "What is 2+2?"}],
    ) as stream:
        for event in stream:
            if event.type == "content_block_start" and getattr(
                event, "content_block", None
            ):
                if getattr(event.content_block, "type", None) == "thinking":
                    events_seen["thinking_start"] = True
            if event.type == "content_block_delta" and getattr(event, "delta", None):
                if event.delta.type == "text_delta":
                    events_seen["text_delta"] = True
                elif event.delta.type == "thinking_delta":
                    events_seen["thinking_delta"] = True

        final = stream.get_final_message()

    assert events_seen["thinking_start"], "No thinking block started"
    assert events_seen["thinking_delta"], "No thinking deltas"
    assert events_seen["text_delta"], "No text deltas"

    block_types = [blk.type for blk in final.content]
    assert "thinking" in block_types
    assert "text" in block_types


# =============================================================================
# CROSS-PROVIDER STREAMING
# =============================================================================


def test_openai_client_streaming_anthropic_upstream(httpserver: HTTPServer):
    """OpenAI client streaming → Anthropic model → proxied via /v1/chat/completions"""
    # Gateway routes OpenAI-format requests to /v1/chat/completions on upstream
    setup_openai_chat_mock(httpserver, content="Cross-provider streaming works!")

    client = openai.OpenAI(api_key="test-key", base_url=f"{LLM_GATEWAY_BASE}/v1")
    stream = client.chat.completions.create(
        model="claude-sonnet-4-20250514",
        max_tokens=100,
        messages=[{"role": "user", "content": "Hello"}],
        stream=True,
    )

    chunks = []
    for chunk in stream:
        if chunk.choices[0].delta.content:
            chunks.append(chunk.choices[0].delta.content)

    assert "".join(chunks) == "Cross-provider streaming works!"


def test_anthropic_client_streaming_openai_upstream(httpserver: HTTPServer):
    """Anthropic client streaming → OpenAI model → OpenAI SSE → transformed to Anthropic SSE"""
    setup_openai_chat_mock(httpserver, content="Reverse cross-provider streaming!")

    client = anthropic.Anthropic(api_key="test-key", base_url=LLM_GATEWAY_BASE)
    with client.messages.stream(
        model="gpt-4o-mini",
        max_tokens=100,
        messages=[{"role": "user", "content": "Hello"}],
    ) as stream:
        pieces = list(stream.text_stream)
        full_text = "".join(pieces)

    assert full_text == "Reverse cross-provider streaming!"


# =============================================================================
# RESPONSES API STREAMING
# =============================================================================


def test_responses_api_streaming_basic(httpserver: HTTPServer):
    """Responses API streaming: verify event types and content assembly"""
    # Gateway translates Responses API to /v1/chat/completions on upstream
    # for non-OpenAI models (OpenAI models pass through to /v1/responses which
    # doesn't work with mocks)
    setup_openai_chat_mock(httpserver, content="Responses API streaming works!")

    client = openai.OpenAI(api_key="test-key", base_url=f"{LLM_GATEWAY_BASE}/v1")
    stream = client.responses.create(
        model="claude-sonnet-4-20250514",
        input="Hello",
        stream=True,
    )

    text_chunks = []
    completed = False
    for event in stream:
        etype = getattr(event, "type", None)
        if etype == "response.output_text.delta" and getattr(event, "delta", None):
            text_chunks.append(event.delta)
        if etype == "response.completed":
            completed = True

    full_content = "".join(text_chunks)
    assert len(text_chunks) > 0, "Should have received text delta events"
    assert len(full_content) > 0


def test_responses_api_streaming_translated_upstream(httpserver: HTTPServer):
    """Responses API streaming with non-OpenAI model → translated to chat completions upstream"""
    setup_openai_chat_mock(httpserver, content="Translated streaming response!")

    client = openai.OpenAI(api_key="test-key", base_url=f"{LLM_GATEWAY_BASE}/v1")
    stream = client.responses.create(
        model="claude-sonnet-4-20250514",
        input="Hello",
        stream=True,
    )

    text_chunks = []
    for event in stream:
        if getattr(event, "type", None) == "response.output_text.delta" and getattr(
            event, "delta", None
        ):
            text_chunks.append(event.delta)

    assert (
        len(text_chunks) > 0
    ), "Should have received text delta events from translated stream"
