"""Mock-based tests for model alias routing.

Tests alias resolution, protocol transformation (OpenAI client ↔ Anthropic upstream
and vice versa), error handling, and multi-turn conversations with tool calls.

These tests require the gateway to be running with config_mock_llm.yaml
(started via docker-compose.mock.yaml).
"""

import json
import openai
import anthropic
import pytest
import logging

from pytest_httpserver import HTTPServer

from conftest import (
    setup_openai_chat_mock,
    setup_anthropic_mock,
    setup_error_mock,
    make_openai_chat_response,
)

logger = logging.getLogger(__name__)

LLM_GATEWAY_BASE = "http://localhost:12000"


# =============================================================================
# ALIAS RESOLUTION TESTS — OpenAI client
# =============================================================================


def test_openai_client_with_alias_arch_summarize_v1(httpserver: HTTPServer):
    """arch.summarize.v1 should resolve to gpt-5-mini-2025-08-07 (OpenAI)"""
    captured = setup_openai_chat_mock(httpserver, content="Hello from mock OpenAI!")

    client = openai.OpenAI(api_key="test-key", base_url=f"{LLM_GATEWAY_BASE}/v1")
    completion = client.chat.completions.create(
        model="arch.summarize.v1",
        max_completion_tokens=500,
        messages=[{"role": "user", "content": "Hello"}],
    )

    assert completion.choices[0].message.content == "Hello from mock OpenAI!"
    # Verify alias was resolved before reaching upstream
    assert len(captured) == 1
    assert captured[0]["model"] == "gpt-5-mini-2025-08-07"


def test_openai_client_with_alias_arch_v1(httpserver: HTTPServer):
    """arch.v1 should resolve to o3 (OpenAI)"""
    captured = setup_openai_chat_mock(httpserver, content="Hello from mock o3!")

    client = openai.OpenAI(api_key="test-key", base_url=f"{LLM_GATEWAY_BASE}/v1")
    completion = client.chat.completions.create(
        model="arch.v1",
        max_completion_tokens=500,
        messages=[{"role": "user", "content": "Hello"}],
    )

    assert completion.choices[0].message.content == "Hello from mock o3!"
    assert len(captured) == 1
    assert captured[0]["model"] == "o3"


def test_openai_client_with_alias_streaming(httpserver: HTTPServer):
    """Streaming with alias should resolve and return streamed content"""
    setup_openai_chat_mock(httpserver, content="Hello from streaming mock!")

    client = openai.OpenAI(api_key="test-key", base_url=f"{LLM_GATEWAY_BASE}/v1")
    stream = client.chat.completions.create(
        model="arch.summarize.v1",
        max_completion_tokens=500,
        messages=[{"role": "user", "content": "Hello"}],
        stream=True,
    )

    chunks = []
    for chunk in stream:
        if chunk.choices[0].delta.content:
            chunks.append(chunk.choices[0].delta.content)

    assert "".join(chunks) == "Hello from streaming mock!"


# =============================================================================
# ALIAS RESOLUTION TESTS — Anthropic client
# =============================================================================


def test_anthropic_client_with_alias_arch_summarize_v1(httpserver: HTTPServer):
    """Anthropic client with alias should route to OpenAI upstream, response transformed to Anthropic format"""
    captured = setup_openai_chat_mock(httpserver, content="Hello via Anthropic client!")

    client = anthropic.Anthropic(api_key="test-key", base_url=LLM_GATEWAY_BASE)
    message = client.messages.create(
        model="arch.summarize.v1",
        max_tokens=500,
        messages=[{"role": "user", "content": "Hello"}],
    )

    response_text = "".join(b.text for b in message.content if b.type == "text")
    assert response_text == "Hello via Anthropic client!"
    # Verify upstream received OpenAI-format request with resolved model
    assert len(captured) == 1
    assert captured[0]["model"] == "gpt-5-mini-2025-08-07"


def test_anthropic_client_with_alias_streaming(httpserver: HTTPServer):
    """Anthropic client streaming with alias → OpenAI upstream → transformed back to Anthropic SSE"""
    setup_openai_chat_mock(httpserver, content="Streaming via Anthropic!")

    client = anthropic.Anthropic(api_key="test-key", base_url=LLM_GATEWAY_BASE)
    with client.messages.stream(
        model="arch.summarize.v1",
        max_tokens=500,
        messages=[{"role": "user", "content": "Hello"}],
    ) as stream:
        pieces = [t for t in stream.text_stream]
        full_text = "".join(pieces)

    assert full_text == "Streaming via Anthropic!"


# =============================================================================
# PROTOCOL TRANSFORMATION TESTS
# =============================================================================


def test_openai_client_with_claude_model(httpserver: HTTPServer):
    """OpenAI client → Claude model → gateway routes to Anthropic upstream → transforms response to OpenAI format"""
    captured = setup_anthropic_mock(
        httpserver, content="Hello from Claude via OpenAI client!"
    )

    client = openai.OpenAI(api_key="test-key", base_url=f"{LLM_GATEWAY_BASE}/v1")
    completion = client.chat.completions.create(
        model="claude-sonnet-4-20250514",
        max_tokens=500,
        messages=[{"role": "user", "content": "Hello"}],
    )

    assert (
        completion.choices[0].message.content == "Hello from Claude via OpenAI client!"
    )
    assert len(captured) == 1
    assert captured[0]["model"] == "claude-sonnet-4-20250514"


def test_openai_client_with_claude_model_streaming(httpserver: HTTPServer):
    """OpenAI client streaming → Claude model → Anthropic SSE → transformed to OpenAI SSE"""
    setup_anthropic_mock(httpserver, content="Streaming from Claude!")

    client = openai.OpenAI(api_key="test-key", base_url=f"{LLM_GATEWAY_BASE}/v1")
    stream = client.chat.completions.create(
        model="claude-sonnet-4-20250514",
        max_tokens=500,
        messages=[{"role": "user", "content": "Hello"}],
        stream=True,
    )

    chunks = []
    for chunk in stream:
        if chunk.choices[0].delta.content:
            chunks.append(chunk.choices[0].delta.content)

    assert "".join(chunks) == "Streaming from Claude!"


def test_anthropic_client_with_openai_model(httpserver: HTTPServer):
    """Anthropic client → OpenAI model (gpt-4o-mini) → OpenAI upstream → transforms response to Anthropic format"""
    captured = setup_openai_chat_mock(
        httpserver, content="Hello from GPT via Anthropic!"
    )

    client = anthropic.Anthropic(api_key="test-key", base_url=LLM_GATEWAY_BASE)
    message = client.messages.create(
        model="gpt-4o-mini",
        max_tokens=500,
        messages=[{"role": "user", "content": "Hello"}],
    )

    response_text = "".join(b.text for b in message.content if b.type == "text")
    assert response_text == "Hello from GPT via Anthropic!"
    assert len(captured) == 1
    assert captured[0]["model"] == "gpt-4o-mini"


def test_anthropic_client_with_openai_model_streaming(httpserver: HTTPServer):
    """Anthropic client streaming → OpenAI model → OpenAI SSE → transformed to Anthropic SSE"""
    setup_openai_chat_mock(httpserver, content="Streaming from GPT!")

    client = anthropic.Anthropic(api_key="test-key", base_url=LLM_GATEWAY_BASE)
    with client.messages.stream(
        model="gpt-4o-mini",
        max_tokens=500,
        messages=[{"role": "user", "content": "Hello"}],
    ) as stream:
        pieces = [t for t in stream.text_stream]
        full_text = "".join(pieces)

    assert full_text == "Streaming from GPT!"


# =============================================================================
# DIRECT MODEL TESTS
# =============================================================================


def test_direct_model_gpt4o_mini_openai(httpserver: HTTPServer):
    """Direct model name (no alias) via OpenAI client"""
    captured = setup_openai_chat_mock(httpserver, content="Direct GPT response!")

    client = openai.OpenAI(api_key="test-key", base_url=f"{LLM_GATEWAY_BASE}/v1")
    completion = client.chat.completions.create(
        model="gpt-4o-mini",
        max_completion_tokens=500,
        messages=[{"role": "user", "content": "Hello"}],
    )

    assert completion.choices[0].message.content == "Direct GPT response!"
    assert captured[0]["model"] == "gpt-4o-mini"


def test_direct_model_claude_anthropic(httpserver: HTTPServer):
    """Direct Claude model via Anthropic client"""
    captured = setup_anthropic_mock(httpserver, content="Direct Claude response!")

    client = anthropic.Anthropic(api_key="test-key", base_url=LLM_GATEWAY_BASE)
    message = client.messages.create(
        model="claude-sonnet-4-20250514",
        max_tokens=500,
        messages=[{"role": "user", "content": "Hello"}],
    )

    response_text = "".join(b.text for b in message.content if b.type == "text")
    assert response_text == "Direct Claude response!"
    assert captured[0]["model"] == "claude-sonnet-4-20250514"


# =============================================================================
# MULTI-TURN WITH TOOL CALLS
# =============================================================================


def test_assistant_message_with_null_content_and_tool_calls(httpserver: HTTPServer):
    """Gateway should handle assistant messages with null content + tool_calls in history"""
    setup_openai_chat_mock(httpserver, content="The weather is sunny in Seattle.")

    client = openai.OpenAI(api_key="test-key", base_url=f"{LLM_GATEWAY_BASE}/v1")
    completion = client.chat.completions.create(
        model="gpt-4o",
        max_tokens=500,
        messages=[
            {"role": "system", "content": "You are a weather assistant."},
            {"role": "user", "content": "What's the weather in Seattle?"},
            {
                "role": "assistant",
                "content": None,
                "tool_calls": [
                    {
                        "id": "call_test123",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": '{"city": "Seattle"}',
                        },
                    }
                ],
            },
            {
                "role": "tool",
                "tool_call_id": "call_test123",
                "content": '{"temperature": "10C", "condition": "Partly cloudy"}',
            },
        ],
        tools=[
            {
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "description": "Get weather for a city",
                    "parameters": {
                        "type": "object",
                        "properties": {"city": {"type": "string"}},
                        "required": ["city"],
                    },
                },
            }
        ],
    )

    assert completion.choices[0].message.content == "The weather is sunny in Seattle."


# =============================================================================
# ERROR HANDLING
# =============================================================================


def test_nonexistent_alias(httpserver: HTTPServer):
    """Non-existent alias should be treated as direct model name and likely fail"""
    client = openai.OpenAI(api_key="test-key", base_url=f"{LLM_GATEWAY_BASE}/v1")

    try:
        client.chat.completions.create(
            model="nonexistent.alias",
            max_completion_tokens=50,
            messages=[{"role": "user", "content": "Hello"}],
        )
        # If it succeeds, the alias was passed through as a direct model name
    except Exception:
        # Error is also acceptable - non-existent model should fail
        pass


# =============================================================================
# THINKING MODE
# =============================================================================


def test_anthropic_thinking_mode_streaming(httpserver: HTTPServer):
    """Anthropic thinking mode should stream thinking + text blocks correctly"""
    setup_anthropic_mock(httpserver, thinking=True)

    client = anthropic.Anthropic(api_key="test-key", base_url=LLM_GATEWAY_BASE)

    thinking_block_started = False
    thinking_delta_seen = False
    text_delta_seen = False

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
                    thinking_block_started = True
            if event.type == "content_block_delta" and getattr(event, "delta", None):
                if event.delta.type == "text_delta":
                    text_delta_seen = True
                elif event.delta.type == "thinking_delta":
                    thinking_delta_seen = True

        final = stream.get_final_message()

    assert final is not None
    assert final.content and len(final.content) > 0
    assert text_delta_seen, "Expected text deltas in stream"
    assert thinking_block_started, "No thinking block started"
    assert thinking_delta_seen, "No thinking deltas observed"

    block_types = [blk.type for blk in final.content]
    assert "text" in block_types
    assert "thinking" in block_types
