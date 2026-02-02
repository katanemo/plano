import openai
import pytest
import os
import logging
import sys

# Set up logging
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - %(name)s - %(levelname)s - %(message)s",
    handlers=[logging.StreamHandler(sys.stdout)],
)
logger = logging.getLogger(__name__)

LLM_GATEWAY_ENDPOINT = os.getenv(
    "LLM_GATEWAY_ENDPOINT", "http://localhost:12000/v1/chat/completions"
)


# -----------------------
# v1/responses API tests
# -----------------------
def test_openai_responses_api_non_streaming_passthrough():
    """Build a v1/responses API request (pass-through) and ensure gateway accepts it"""
    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = openai.OpenAI(api_key="test-key", base_url=f"{base_url}/v1")

    # Simple responses API request using a direct model (pass-through)
    resp = client.responses.create(
        model="gpt-4o", input="Hello via responses passthrough"
    )

    # Print the response content - handle both responses format and chat completions format
    print(f"\n{'='*80}")
    print(f"Model: {resp.model}")
    print(f"Output: {resp.output_text}")
    print(f"{'='*80}\n")

    # Minimal sanity checks
    assert resp is not None
    assert (
        getattr(resp, "id", None) is not None
        or getattr(resp, "output", None) is not None
    )


def test_openai_responses_api_with_streaming_passthrough():
    """Build a v1/responses API streaming request (pass-through) and ensure gateway accepts it"""
    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = openai.OpenAI(api_key="test-key", base_url=f"{base_url}/v1")

    # Simple streaming responses API request using a direct model (pass-through)
    stream = client.responses.create(
        model="gpt-4o",
        input="Write a short haiku about coding",
        stream=True,
    )

    # Collect streamed content using the official Responses API streaming shape
    text_chunks = []
    final_message = None

    for event in stream:
        # The Python SDK surfaces a high-level Responses streaming interface.
        # We rely on its typed helpers instead of digging into model_extra.
        if getattr(event, "type", None) == "response.output_text.delta" and getattr(
            event, "delta", None
        ):
            # Each delta contains a text fragment
            text_chunks.append(event.delta)

        # Track the final response message if provided by the SDK
        if getattr(event, "type", None) == "response.completed" and getattr(
            event, "response", None
        ):
            final_message = event.response

    full_content = "".join(text_chunks)

    # Print the streaming response
    print(f"\n{'='*80}")
    print(
        f"Model: {getattr(final_message, 'model', 'unknown') if final_message else 'unknown'}"
    )
    print(f"Streamed Output: {full_content}")
    print(f"{'='*80}\n")

    assert len(text_chunks) > 0, "Should have received streaming text deltas"
    assert len(full_content) > 0, "Should have received content"


def test_openai_responses_api_non_streaming_with_tools_passthrough():
    """Responses API with a function/tool definition (pass-through)"""
    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = openai.OpenAI(api_key="test-key", base_url=f"{base_url}/v1", max_retries=0)

    # Define a simple tool/function for the Responses API
    tools = [
        {
            "type": "function",
            "name": "echo_tool",
            "description": "Echo back the provided input",
            "parameters": {
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"],
            },
        }
    ]

    resp = client.responses.create(
        model="openai/gpt-5-mini-2025-08-07",
        input="Call the echo tool",
        tools=tools,
    )

    assert resp is not None
    assert (
        getattr(resp, "id", None) is not None
        or getattr(resp, "output", None) is not None
    )


def test_openai_responses_api_with_streaming_with_tools_passthrough():
    """Responses API with a function/tool definition (streaming, pass-through)"""
    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = openai.OpenAI(api_key="test-key", base_url=f"{base_url}/v1", max_retries=0)

    tools = [
        {
            "type": "function",
            "name": "echo_tool",
            "description": "Echo back the provided input",
            "parameters": {
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"],
            },
        }
    ]

    stream = client.responses.create(
        model="openai/gpt-5-mini-2025-08-07",
        input="Call the echo tool",
        tools=tools,
        stream=True,
    )

    text_chunks = []
    tool_calls = []

    for event in stream:
        etype = getattr(event, "type", None)

        # Collect streamed text output
        if etype == "response.output_text.delta" and getattr(event, "delta", None):
            text_chunks.append(event.delta)

        # Collect streamed tool call arguments
        if etype == "response.function_call_arguments.delta" and getattr(
            event, "delta", None
        ):
            tool_calls.append(event.delta)

    full_text = "".join(text_chunks)

    print(f"\n{'='*80}")
    print("Responses tools streaming test")
    print(f"Streamed text: {full_text}")
    print(f"Tool call argument chunks: {len(tool_calls)}")
    print(f"{'='*80}\n")

    # We expect either streamed text output or streamed tool-call arguments
    assert (
        full_text or tool_calls
    ), "Expected streamed text or tool call argument deltas from Responses tools stream"


def test_openai_responses_api_non_streaming_upstream_chat_completions():
    """Send a v1/responses request using the grok alias to verify translation/routing"""
    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = openai.OpenAI(api_key="test-key", base_url=f"{base_url}/v1")

    resp = client.responses.create(
        model="arch.grok.v1", input="Hello, translate this via grok alias"
    )

    # Print the response content - handle both responses format and chat completions format
    print(f"\n{'='*80}")
    print(f"Model: {resp.model}")
    print(f"Output: {resp.output_text}")
    print(f"{'='*80}\n")

    assert resp is not None
    assert resp.id is not None


def test_openai_responses_api_with_streaming_upstream_chat_completions():
    """Build a v1/responses API streaming request (pass-through) and ensure gateway accepts it"""
    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = openai.OpenAI(api_key="test-key", base_url=f"{base_url}/v1")

    # Simple streaming responses API request using a direct model (pass-through)
    stream = client.responses.create(
        model="arch.grok.v1",
        input="Write a short haiku about coding",
        stream=True,
    )

    # Collect streamed content using the official Responses API streaming shape
    text_chunks = []
    final_message = None

    for event in stream:
        # The Python SDK surfaces a high-level Responses streaming interface.
        # We rely on its typed helpers instead of digging into model_extra.
        if getattr(event, "type", None) == "response.output_text.delta" and getattr(
            event, "delta", None
        ):
            # Each delta contains a text fragment
            text_chunks.append(event.delta)

        # Track the final response message if provided by the SDK
        if getattr(event, "type", None) == "response.completed" and getattr(
            event, "response", None
        ):
            final_message = event.response

    full_content = "".join(text_chunks)

    # Print the streaming response
    print(f"\n{'='*80}")
    print(
        f"Model: {getattr(final_message, 'model', 'unknown') if final_message else 'unknown'}"
    )
    print(f"Streamed Output: {full_content}")
    print(f"{'='*80}\n")

    assert len(text_chunks) > 0, "Should have received streaming text deltas"
    assert len(full_content) > 0, "Should have received content"


def test_openai_responses_api_non_streaming_with_tools_upstream_chat_completions():
    """Responses API wioutputling routed to grok via alias"""
    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = openai.OpenAI(api_key="test-key", base_url=f"{base_url}/v1")

    tools = [
        {
            "type": "function",
            "name": "echo_tool",
            "description": "Echo back the provided input",
            "parameters": {
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"],
            },
        }
    ]

    resp = client.responses.create(
        model="arch.grok.v1",
        input="Call the echo tool",
        tools=tools,
    )

    assert resp.id is not None

    print(f"\n{'='*80}")
    print(f"Model: {resp.model}")
    print(f"Output: {resp.output_text}")
    print(f"{'='*80}\n")


def test_openai_responses_api_streaming_with_tools_upstream_chat_completions():
    """Responses API with a function/tool definition (streaming, pass-through)"""
    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = openai.OpenAI(api_key="test-key", base_url=f"{base_url}/v1", max_retries=0)

    tools = [
        {
            "type": "function",
            "name": "echo_tool",
            "description": "Echo back the provided input",
            "parameters": {
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"],
            },
        }
    ]

    stream = client.responses.create(
        model="arch.grok.v1",
        input="Call the echo tool",
        tools=tools,
        stream=True,
    )

    text_chunks = []
    tool_calls = []

    for event in stream:
        etype = getattr(event, "type", None)

        # Collect streamed text output
        if etype == "response.output_text.delta" and getattr(event, "delta", None):
            text_chunks.append(event.delta)

        # Collect streamed tool call arguments
        if etype == "response.function_call_arguments.delta" and getattr(
            event, "delta", None
        ):
            tool_calls.append(event.delta)

    full_text = "".join(text_chunks)

    print(f"\n{'='*80}")
    print("Responses tools streaming test")
    print(f"Streamed text: {full_text}")
    print(f"Tool call argument chunks: {len(tool_calls)}")
    print(f"{'='*80}\n")

    # We expect either streamed text output or streamed tool-call arguments
    assert (
        full_text or tool_calls
    ), "Expected streamed text or tool call argument deltas from Responses tools stream"


def test_openai_responses_api_non_streaming_upstream_bedrock():
    """Send a v1/responses request using the coding-model alias to verify Bedrock translation/routing"""
    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = openai.OpenAI(api_key="test-key", base_url=f"{base_url}/v1")

    resp = client.responses.create(
        model="coding-model",
        input="Hello, translate this via coding-model alias to Bedrock",
    )

    # Print the response content - handle both responses format and chat completions format
    print(f"\n{'='*80}")
    print(f"Model: {resp.model}")
    print(f"Output: {resp.output_text}")
    print(f"{'='*80}\n")

    assert resp is not None
    assert resp.id is not None


def test_openai_responses_api_with_streaming_upstream_bedrock():
    """Build a v1/responses API streaming request routed to Bedrock via coding-model alias"""
    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = openai.OpenAI(api_key="test-key", base_url=f"{base_url}/v1")

    # Simple streaming responses API request using coding-model alias
    stream = client.responses.create(
        model="coding-model",
        input="Write a short haiku about coding",
        stream=True,
    )

    # Collect streamed content using the official Responses API streaming shape
    text_chunks = []
    final_message = None

    for event in stream:
        # The Python SDK surfaces a high-level Responses streaming interface.
        # We rely on its typed helpers instead of digging into model_extra.
        if getattr(event, "type", None) == "response.output_text.delta" and getattr(
            event, "delta", None
        ):
            # Each delta contains a text fragment
            text_chunks.append(event.delta)

        # Track the final response message if provided by the SDK
        if getattr(event, "type", None) == "response.completed" and getattr(
            event, "response", None
        ):
            final_message = event.response

    full_content = "".join(text_chunks)

    # Print the streaming response
    print(f"\n{'='*80}")
    print(
        f"Model: {getattr(final_message, 'model', 'unknown') if final_message else 'unknown'}"
    )
    print(f"Streamed Output: {full_content}")
    print(f"{'='*80}\n")

    assert len(text_chunks) > 0, "Should have received streaming text deltas"
    assert len(full_content) > 0, "Should have received content"


def test_openai_responses_api_non_streaming_with_tools_upstream_bedrock():
    """Responses API with tools routed to Bedrock via coding-model alias"""
    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = openai.OpenAI(api_key="test-key", base_url=f"{base_url}/v1")

    tools = [
        {
            "type": "function",
            "name": "echo_tool",
            "description": "Echo back the provided input",
            "parameters": {
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"],
            },
        }
    ]

    resp = client.responses.create(
        model="coding-model",
        input="Call the echo tool",
        tools=tools,
    )

    assert resp.id is not None

    print(f"\n{'='*80}")
    print(f"Model: {resp.model}")
    print(f"Output: {resp.output_text}")
    print(f"{'='*80}\n")


def test_openai_responses_api_streaming_with_tools_upstream_bedrock():
    """Responses API with a function/tool definition streaming to Bedrock via coding-model alias"""
    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = openai.OpenAI(api_key="test-key", base_url=f"{base_url}/v1", max_retries=0)

    tools = [
        {
            "type": "function",
            "name": "echo_tool",
            "description": "Echo back the provided input",
            "parameters": {
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"],
            },
        }
    ]

    stream = client.responses.create(
        model="coding-model",
        input="Call the echo tool",
        tools=tools,
        stream=True,
    )

    text_chunks = []
    tool_calls = []

    for event in stream:
        etype = getattr(event, "type", None)

        # Collect streamed text output
        if etype == "response.output_text.delta" and getattr(event, "delta", None):
            text_chunks.append(event.delta)

        # Collect streamed tool call arguments
        if etype == "response.function_call_arguments.delta" and getattr(
            event, "delta", None
        ):
            tool_calls.append(event.delta)

    full_text = "".join(text_chunks)

    print(f"\n{'='*80}")
    print("Responses tools streaming test (Bedrock)")
    print(f"Streamed text: {full_text}")
    print(f"Tool call argument chunks: {len(tool_calls)}")
    print(f"{'='*80}\n")

    # We expect either streamed text output or streamed tool-call arguments
    assert (
        full_text or tool_calls
    ), "Expected streamed text or tool call argument deltas from Responses tools stream"


def test_openai_responses_api_non_streaming_upstream_anthropic():
    """Send a v1/responses request using the grok alias to verify translation/routing"""
    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = openai.OpenAI(api_key="test-key", base_url=f"{base_url}/v1")

    resp = client.responses.create(
        model="claude-sonnet-4-20250514", input="Hello, translate this via grok alias"
    )

    # Print the response content - handle both responses format and chat completions format
    print(f"\n{'='*80}")
    print(f"Model: {resp.model}")
    print(f"Output: {resp.output_text}")
    print(f"{'='*80}\n")

    assert resp is not None
    assert resp.id is not None


def test_openai_responses_api_with_streaming_upstream_anthropic():
    """Build a v1/responses API streaming request (pass-through) and ensure gateway accepts it"""
    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = openai.OpenAI(api_key="test-key", base_url=f"{base_url}/v1")

    # Simple streaming responses API request using a direct model (pass-through)
    stream = client.responses.create(
        model="claude-sonnet-4-20250514",
        input="Write a short haiku about coding",
        stream=True,
    )

    # Collect streamed content using the official Responses API streaming shape
    text_chunks = []
    final_message = None

    for event in stream:
        # The Python SDK surfaces a high-level Responses streaming interface.
        # We rely on its typed helpers instead of digging into model_extra.
        if getattr(event, "type", None) == "response.output_text.delta" and getattr(
            event, "delta", None
        ):
            # Each delta contains a text fragment
            text_chunks.append(event.delta)

        # Track the final response message if provided by the SDK
        if getattr(event, "type", None) == "response.completed" and getattr(
            event, "response", None
        ):
            final_message = event.response

    full_content = "".join(text_chunks)

    # Print the streaming response
    print(f"\n{'='*80}")
    print(
        f"Model: {getattr(final_message, 'model', 'unknown') if final_message else 'unknown'}"
    )
    print(f"Streamed Output: {full_content}")
    print(f"{'='*80}\n")

    assert len(text_chunks) > 0, "Should have received streaming text deltas"
    assert len(full_content) > 0, "Should have received content"


def test_openai_responses_api_non_streaming_with_tools_upstream_anthropic():
    """Responses API with tools routed to grok via alias"""
    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = openai.OpenAI(api_key="test-key", base_url=f"{base_url}/v1")

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

    print(f"\n{'='*80}")
    print(f"Model: {resp.model}")
    print(f"Output: {resp.output_text}")
    print(f"{'='*80}\n")


def test_openai_responses_api_streaming_with_tools_upstream_anthropic():
    """Responses API with a function/tool definition (streaming, pass-through)"""
    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = openai.OpenAI(api_key="test-key", base_url=f"{base_url}/v1", max_retries=0)

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

        # Collect streamed text output
        if etype == "response.output_text.delta" and getattr(event, "delta", None):
            text_chunks.append(event.delta)

        # Collect streamed tool call arguments
        if etype == "response.function_call_arguments.delta" and getattr(
            event, "delta", None
        ):
            tool_calls.append(event.delta)

    full_text = "".join(text_chunks)

    print(f"\n{'='*80}")
    print("Responses tools streaming test")
    print(f"Streamed text: {full_text}")
    print(f"Tool call argument chunks: {len(tool_calls)}")
    print(f"{'='*80}\n")

    # We expect either streamed text output or streamed tool-call arguments
    assert (
        full_text or tool_calls
    ), "Expected streamed text or tool call argument deltas from Responses tools stream"


def test_openai_responses_api_mixed_content_types():
    """Test Responses API with mixed content types (string and array) in input messages"""
    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = openai.OpenAI(api_key="test-key", base_url=f"{base_url}/v1")

    # This test mimics the request that was failing:
    # One message with string content, another with array content
    resp = client.responses.create(
        model="openai/gpt-5-mini-2025-08-07",
        input=[
            {
                "role": "developer",
                "content": "Generate a very short chat title (2-5 words max) based on the user's message.\n"
                "Rules:\n"
                "- Maximum 30 characters\n"
                "- No quotes, colons, hashtags, or markdown\n"
                "- Just the topic/intent, not a full sentence\n"
                '- If the message is a greeting like "hi" or "hello", respond with just "New conversation"\n'
                '- Be concise: "Weather in NYC" not "User asking about the weather in New York City"',
            },
            {
                "role": "user",
                "content": [
                    {"type": "input_text", "text": "What is the weather in Seattle"}
                ],
            },
        ],
    )

    # Print the response
    print(f"\n{'='*80}")
    print(f"Model: {resp.model}")
    print(f"Output: {resp.output_text}")
    print(f"{'='*80}\n")

    assert resp is not None
    assert resp.id is not None
    # Verify we got a reasonable title
    assert len(resp.output_text) > 0
