import anthropic
import openai
import os
import logging
import pytest
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

# =============================================================================
# MODEL ALIAS TESTS
# =============================================================================


def test_assistant_message_with_null_content_and_tool_calls():
    """Test that assistant messages with null content and tool_calls are properly handled"""
    logger.info(
        "Testing assistant message with null content and tool_calls (multi-turn conversation)"
    )

    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = openai.OpenAI(
        api_key="test-key",
        base_url=f"{base_url}/v1",
    )

    # Simulate a multi-turn conversation where:
    # 1. User asks a question
    # 2. Assistant makes a tool call (with null content)
    # 3. Tool responds
    # 4. Assistant should provide final answer
    completion = client.chat.completions.create(
        model="gpt-4o",
        max_tokens=500,
        messages=[
            {
                "role": "system",
                "content": "You are a weather assistant. Use the get_weather tool to fetch weather information.",
            },
            {"role": "user", "content": "What's the weather in Seattle?"},
            {
                "role": "assistant",
                "content": None,  # This is the key test - null content with tool_calls
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
                "content": '{"location": "Seattle", "temperature": "10°C", "condition": "Partly cloudy"}',
            },
        ],
        tools=[
            {
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "description": "Get weather information for a city",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "city": {"type": "string", "description": "City name"}
                        },
                        "required": ["city"],
                    },
                },
            }
        ],
    )

    response_content = completion.choices[0].message.content
    logger.info(f"Response after tool call: {response_content}")

    # The assistant should provide a final response using the tool result
    assert response_content is not None
    assert len(response_content) > 0
    logger.info(
        "✓ Assistant message with null content and tool_calls handled correctly"
    )


def test_openai_client_with_alias_arch_summarize_v1():
    """Test OpenAI client using model alias 'arch.summarize.v1' which should resolve to '4o-mini'"""
    logger.info("Testing OpenAI client with alias 'arch.summarize.v1' -> '4o-mini'")

    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = openai.OpenAI(
        api_key="test-key",
        base_url=f"{base_url}/v1",
    )

    completion = client.chat.completions.create(
        model="arch.summarize.v1",  # This should resolve to 5o-mini
        max_completion_tokens=500,  # Increased token limit to avoid truncation and because the 5o-mini uses reasoning tokens
        messages=[
            {
                "role": "user",
                "content": "Hello, please respond with exactly: Hello from alias arch.summarize.v1!",
            }
        ],
    )

    response_content = completion.choices[0].message.content
    logger.info(f"Response from arch.summarize.v1 alias: {response_content}")
    assert response_content == "Hello from alias arch.summarize.v1!"


def test_openai_client_with_alias_arch_v1():
    """Test OpenAI client using model alias 'arch.v1' which should resolve to 'o3'"""
    logger.info("Testing OpenAI client with alias 'arch.v1' -> 'o3'")

    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = openai.OpenAI(
        api_key="test-key",
        base_url=f"{base_url}/v1",
    )

    completion = client.chat.completions.create(
        model="arch.v1",  # This should resolve to gpt-o3
        max_completion_tokens=500,
        messages=[
            {
                "role": "user",
                "content": "Hello, please respond with exactly: Hello from alias arch.v1!",
            }
        ],
    )

    response_content = completion.choices[0].message.content
    logger.info(f"Response from arch.v1 alias: {response_content}")
    assert response_content == "Hello from alias arch.v1!"


def test_anthropic_client_with_alias_arch_summarize_v1():
    """Test Anthropic client using model alias 'arch.summarize.v1' which should resolve to '4o-mini'"""
    logger.info("Testing Anthropic client with alias 'arch.summarize.v1' -> '4o-mini'")

    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = anthropic.Anthropic(api_key="test-key", base_url=base_url)

    message = client.messages.create(
        model="arch.summarize.v1",  # This should resolve to 5o-mini
        max_tokens=500,
        messages=[
            {
                "role": "user",
                "content": "Hello, please respond with exactly: Hello from alias arch.summarize.v1 via Anthropic!",
            }
        ],
    )

    response_content = "".join(b.text for b in message.content if b.type == "text")
    logger.info(
        f"Response from arch.summarize.v1 alias via Anthropic: {response_content}"
    )
    assert response_content == "Hello from alias arch.summarize.v1 via Anthropic!"


def test_anthropic_client_with_alias_arch_v1():
    """Test Anthropic client using model alias 'arch.v1' which should resolve to 'o3'"""
    logger.info("Testing Anthropic client with alias 'arch.v1' -> 'o3'")

    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = anthropic.Anthropic(api_key="test-key", base_url=base_url)

    message = client.messages.create(
        model="arch.v1",  # This should resolve to o3
        max_tokens=500,
        messages=[
            {
                "role": "user",
                "content": "Hello, please respond with exactly: Hello from alias arch.v1 via Anthropic!",
            }
        ],
    )

    response_content = "".join(b.text for b in message.content if b.type == "text")
    logger.info(f"Response from arch.v1 alias via Anthropic: {response_content}")
    assert response_content == "Hello from alias arch.v1 via Anthropic!"


def test_openai_client_with_alias_streaming():
    """Test OpenAI client using model alias with streaming"""
    logger.info(
        "Testing OpenAI client with alias 'arch.summarize.v1' streaming -> '4o-mini'"
    )

    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = openai.OpenAI(
        api_key="test-key",
        base_url=f"{base_url}/v1",
    )

    stream = client.chat.completions.create(
        model="arch.summarize.v1",  # This should resolve to 5o-mini
        max_completion_tokens=500,
        messages=[
            {
                "role": "user",
                "content": "Hello, please respond with exactly: Hello from streaming alias!",
            }
        ],
        stream=True,
    )

    content_chunks = []
    for chunk in stream:
        if chunk.choices[0].delta.content:
            content_chunks.append(chunk.choices[0].delta.content)

    full_content = "".join(content_chunks)
    logger.info(f"Streaming response from arch.summarize.v1 alias: {full_content}")
    assert full_content == "Hello from streaming alias!"


def test_anthropic_client_with_alias_streaming():
    """Test Anthropic client using model alias with streaming"""
    logger.info(
        "Testing Anthropic client with alias 'arch.summarize.v1' streaming -> '4o-mini'"
    )

    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = anthropic.Anthropic(api_key="test-key", base_url=base_url)

    with client.messages.stream(
        model="arch.summarize.v1",  # This should resolve to 5o-mini
        max_tokens=500,
        messages=[
            {
                "role": "user",
                "content": "Hello, please respond with exactly: Hello from streaming alias via Anthropic!",
            }
        ],
    ) as stream:
        pieces = [t for t in stream.text_stream]
        full_text = "".join(pieces)

    logger.info(
        f"Streaming response from arch.summarize.v1 alias via Anthropic: {full_text}"
    )
    assert full_text == "Hello from streaming alias via Anthropic!"


def test_400_error_handling_with_alias():
    """Test that 400 errors from upstream are properly returned by archgw"""
    logger.info(
        "Testing 400 error handling with arch.summarize.v1 and invalid parameter"
    )

    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = openai.OpenAI(
        api_key="test-key",
        base_url=f"{base_url}/v1",
    )

    try:
        completion = client.chat.completions.create(
            model="arch.summarize.v1",  # This should resolve to gpt-5-mini-2025-08-07
            max_tokens=50,
            messages=[
                {
                    "role": "user",
                    "content": "Hello, this should trigger a 400 error due to invalid parameter name",
                }
            ],
        )
        # If we reach here, the request unexpectedly succeeded
        logger.error(
            f"Expected 400 error but got successful response: {completion.choices[0].message.content}"
        )
        assert False, "Expected 400 error but request succeeded"
    except openai.BadRequestError as e:
        # This is what we expect - a 400 Bad Request error
        logger.info(f"Correctly received 400 Bad Request error: {e}")
        assert e.status_code == 400, f"Expected status code 400, got {e.status_code}"
        logger.info("✓ 400 error handling working correctly")
    except Exception as e:
        # Any other exception is unexpected
        logger.error(
            f"Unexpected error type (should be BadRequestError): {type(e).__name__}: {e}"
        )
        assert False, f"Expected BadRequestError but got {type(e).__name__}: {e}"


def test_400_error_handling_unsupported_parameter():
    """Test that 400 errors for unsupported parameters are properly returned by archgw"""
    logger.info("Testing 400 error handling with unsupported max_tokens parameter")

    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = openai.OpenAI(
        api_key="test-key",
        base_url=f"{base_url}/v1",
    )

    try:
        # Use the deprecated max_tokens parameter which should trigger a 400 error
        completion = client.chat.completions.create(
            model="arch.summarize.v1",  # This should resolve to gpt-5-mini-2025-08-07
            max_tokens=150,  # This parameter is unsupported for newer models, should use max_completion_tokens
            messages=[
                {
                    "role": "user",
                    "content": "Hello, this should trigger a 400 error due to unsupported max_tokens parameter",
                }
            ],
        )
        # If we reach here, the request unexpectedly succeeded
        logger.error(
            f"Expected 400 error but got successful response: {completion.choices[0].message.content}"
        )
        assert False, "Expected 400 error but request succeeded"
    except openai.BadRequestError as e:
        # This is what we expect - a 400 Bad Request error
        logger.info(f"Correctly received 400 Bad Request error: {e}")
        assert e.status_code == 400, f"Expected status code 400, got {e.status_code}"
        assert "max_tokens" in str(e), "Expected error message to mention max_tokens"
        logger.info("✓ 400 error handling for unsupported parameters working correctly")
    except Exception as e:
        # Any other exception is unexpected
        logger.error(
            f"Unexpected error type (should be BadRequestError): {type(e).__name__}: {e}"
        )
        assert False, f"Expected BadRequestError but got {type(e).__name__}: {e}"


def test_nonexistent_alias():
    """Test that using a non-existent alias falls back to treating it as a direct model name"""
    logger.info(
        "Testing non-existent alias 'nonexistent.alias' should be treated as direct model"
    )

    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = openai.OpenAI(
        api_key="test-key",
        base_url=f"{base_url}/v1",
    )

    try:
        completion = client.chat.completions.create(
            model="nonexistent.alias",  # This alias doesn't exist
            max_completion_tokens=50,
            messages=[
                {
                    "role": "user",
                    "content": "Hello, this should fail or use as direct model name",
                }
            ],
        )
        logger.info("Non-existent alias was handled gracefully")
        # If it succeeds, it means the alias was passed through as a direct model name
        logger.info(f"Response: {completion.choices[0].message.content}")
    except Exception as e:
        logger.info(f"Non-existent alias resulted in error (expected): {e}")
        # This is also acceptable behavior


# =============================================================================
# DIRECT MODEL TESTS (for comparison)
# =============================================================================


def test_direct_model_4o_mini_openai():
    """Test OpenAI client using direct model name '4o-mini'"""
    logger.info("Testing OpenAI client with direct model '4o-mini'")

    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = openai.OpenAI(
        api_key="test-key",
        base_url=f"{base_url}/v1",
    )

    completion = client.chat.completions.create(
        model="gpt-4o-mini",  # Direct model name
        max_completion_tokens=50,
        messages=[
            {
                "role": "user",
                "content": "Hello, please respond with exactly: Hello from direct 4o-mini!",
            }
        ],
    )

    response_content = completion.choices[0].message.content
    logger.info(f"Response from direct 4o-mini: {response_content}")
    assert response_content == "Hello from direct 4o-mini!"


def test_direct_model_4o_mini_anthropic():
    """Test Anthropic client using direct model name '4o-mini'"""
    logger.info("Testing Anthropic client with direct model '4o-mini'")

    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = anthropic.Anthropic(api_key="test-key", base_url=base_url)

    message = client.messages.create(
        model="gpt-4o-mini",  # Direct model name
        max_tokens=50,
        messages=[
            {
                "role": "user",
                "content": "Hello, please respond with exactly: Hello from direct 4o-mini via Anthropic!",
            }
        ],
    )

    response_content = "".join(b.text for b in message.content if b.type == "text")
    logger.info(f"Response from direct 4o-mini via Anthropic: {response_content}")
    assert response_content == "Hello from direct 4o-mini via Anthropic!"


def test_anthropic_thinking_mode_streaming():
    # Anthropic base_url should be the root, not /v1/chat/completions
    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")

    client = anthropic.Anthropic(
        api_key=os.environ.get("ANTHROPIC_API_KEY", "test-key"),
        base_url=base_url,
    )

    thinking_block_started = False
    thinking_delta_seen = False
    text_delta_seen = False

    with client.messages.stream(
        model="claude-sonnet-4-20250514",
        max_tokens=2048,
        thinking={"type": "enabled", "budget_tokens": 1024},  # <- idiomatic
        messages=[{"role": "user", "content": "Explain briefly what 2+2 equals"}],
    ) as stream:
        for event in stream:
            # 1) detect when a thinking block starts
            if event.type == "content_block_start" and getattr(
                event, "content_block", None
            ):
                if getattr(event.content_block, "type", None) == "thinking":
                    thinking_block_started = True

            # 2) collect text vs thinking deltas
            if event.type == "content_block_delta" and getattr(event, "delta", None):
                if event.delta.type == "text_delta":
                    text_delta_seen = True
                elif event.delta.type == "thinking_delta":
                    # some SDKs expose .thinking, others .text for this delta; not needed here
                    thinking_delta_seen = True

        final = stream.get_final_message()

    # Basic integrity
    assert final is not None
    assert final.content and len(final.content) > 0

    # Normal text should have streamed
    assert text_delta_seen, "Expected normal text deltas in stream"

    # With thinking enabled, we expect a thinking block and at least one thinking delta
    assert thinking_block_started, "No thinking block started"
    assert thinking_delta_seen, "No thinking deltas observed"

    # Optional: double-check on the assembled message
    final_block_types = [blk.type for blk in final.content]
    assert "text" in final_block_types
    assert "thinking" in final_block_types


@pytest.mark.skip("unreliable - bedrock tests are flaky in CI")
def test_openai_client_with_coding_model_alias_and_tools():
    """Test OpenAI client using 'coding-model' alias (maps to Bedrock) with coding question and tools"""
    logger.info("Testing OpenAI client with 'coding-model' alias -> Bedrock with tools")

    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = openai.OpenAI(
        api_key="test-key",
        base_url=f"{base_url}/v1",
    )

    completion = client.chat.completions.create(
        model="coding-model",  # This should resolve to us.amazon.nova-premier-v1:0
        max_tokens=1000,
        messages=[
            {
                "role": "user",
                "content": "I need to write a Python function that calculates the factorial of a number. Can you help me write and run it?",
            }
        ],
        tools=[
            {
                "type": "function",
                "function": {
                    "name": "run_python_code",
                    "description": "Execute Python code and return the result",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "code": {
                                "type": "string",
                                "description": "Python code to execute",
                            }
                        },
                        "required": ["code"],
                    },
                },
            }
        ],
        tool_choice="auto",
    )

    response_content = completion.choices[0].message.content
    tool_calls = completion.choices[0].message.tool_calls
    # Should get either text response or tool calls for coding assistance
    assert response_content is not None or (
        tool_calls is not None and len(tool_calls) > 0
    )


@pytest.mark.skip("unreliable - bedrock tests are flaky in CI")
def test_anthropic_client_with_coding_model_alias_and_tools():
    """Test Anthropic client using 'coding-model' alias (maps to Bedrock) with coding question and tools"""
    logger.info(
        "Testing Anthropic client with 'coding-model' alias -> Bedrock with tools"
    )

    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = anthropic.Anthropic(api_key="test-key", base_url=base_url)

    message = client.messages.create(
        model="coding-model",  # This should resolve to us.amazon.nova-premier-v1:0
        max_tokens=1000,
        messages=[
            {
                "role": "user",
                "content": "I need to write a Python function that calculates the factorial of a number. Can you help me write and run it?",
            }
        ],
        tools=[
            {
                "name": "run_python_code",
                "description": "Execute Python code and return the result",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "code": {
                            "type": "string",
                            "description": "Python code to execute",
                        }
                    },
                    "required": ["code"],
                },
            }
        ],
        tool_choice={"type": "auto"},
    )

    text_content = "".join(b.text for b in message.content if b.type == "text")
    tool_use_blocks = [b for b in message.content if b.type == "tool_use"]

    logger.info(f"Response from coding-model alias via Anthropic: {text_content}")
    logger.info(f"Tool use blocks: {len(tool_use_blocks)}")

    # Should get either text response or tool use blocks for coding assistance
    assert text_content or len(tool_use_blocks) > 0


@pytest.mark.skip("unreliable - bedrock tests are flaky in CI")
def test_anthropic_client_with_coding_model_alias_and_tools_streaming():
    """Test Anthropic client using 'coding-model' alias (maps to Bedrock) with coding question and tools - streaming"""
    logger.info(
        "Testing Anthropic client with 'coding-model' alias -> Bedrock with tools (streaming)"
    )

    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = anthropic.Anthropic(api_key="test-key", base_url=base_url)

    text_chunks = []
    tool_use_blocks = []
    all_events = []  # Capture all events for debugging

    try:
        with client.messages.stream(
            model="coding-model",  # This should resolve to us.amazon.nova-premier-v1:0
            max_tokens=1000,
            messages=[
                {
                    "role": "user",
                    "content": "I need to write a Python function that calculates the factorial of a number. Can you help me write and run it?",
                }
            ],
            tools=[
                {
                    "name": "run_python_code",
                    "description": "Execute Python code and return the result",
                    "input_schema": {
                        "type": "object",
                        "properties": {
                            "code": {
                                "type": "string",
                                "description": "Python code to execute",
                            }
                        },
                        "required": ["code"],
                    },
                }
            ],
            tool_choice={"type": "auto"},
        ) as stream:
            for event in stream:
                # Extract index if available
                index = getattr(event, "index", None)

                # Log and capture all events for debugging
                all_events.append(
                    {"type": event.type, "index": index, "event": str(event)[:200]}
                )
                logger.info(f"Event #{len(all_events)}: {event.type} [index={index}]")

                # Collect text deltas
                if event.type == "content_block_delta" and hasattr(event, "delta"):
                    if event.delta.type == "text_delta":
                        text_chunks.append(event.delta.text)

                # Collect tool use blocks
                if event.type == "content_block_start" and hasattr(
                    event, "content_block"
                ):
                    if event.content_block.type == "tool_use":
                        tool_use_blocks.append(event.content_block)

            final_message = stream.get_final_message()
    except Exception as e:
        logger.error(f"Exception during streaming: {type(e).__name__}: {e}")
        logger.error(f"Events received before error: {len(all_events)}")
        logger.error(f"Text chunks collected: {len(text_chunks)}")
        logger.error(f"Tool use blocks collected: {len(tool_use_blocks)}")
        logger.error("\nLast 20 events before crash:")
        for evt in all_events[-20:]:
            logger.error(f"  {evt['type']:30s} index={evt['index']}")
        raise

    full_text = "".join(text_chunks)
    logger.info(f"Streaming response from coding-model with tools: {full_text}")
    logger.info(f"Total events received: {len(all_events)}")
    logger.info(
        f"Text chunks: {len(text_chunks)}, Tool use blocks: {len(tool_use_blocks)}"
    )

    # Should get either text response or tool use blocks for coding assistance
    # Modified assertion to be more lenient and provide better error messages
    assert (
        full_text or len(tool_use_blocks) > 0
    ), f"Expected text or tool use. Got text_len={len(full_text)}, tools={len(tool_use_blocks)}, events={len(all_events)}"

    # Verify final message structure
    assert final_message is not None, "Final message should not be None"
    assert (
        final_message.content and len(final_message.content) > 0
    ), f"Final message should have content. Got: {final_message.content if final_message else 'None'}"


@pytest.mark.skip("unreliable - bedrock tests are flaky in CI")
def test_anthropic_client_streaming_with_bedrock():
    """Test Anthropic client using 'coding-model' alias (maps to Bedrock) with streaming"""
    logger.info(
        "Testing Anthropic client with 'coding-model' alias -> Bedrock (streaming)"
    )

    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = anthropic.Anthropic(api_key="test-key", base_url=base_url)

    text_chunks = []

    with client.messages.stream(
        model="coding-model",  # This should resolve to us.amazon.nova-premier-v1:0
        max_tokens=500,
        messages=[
            {
                "role": "user",
                "content": "Write a short 4-line sonnet about coding.",
            }
        ],
    ) as stream:
        for event in stream:
            # Collect text deltas
            if event.type == "content_block_delta" and hasattr(event, "delta"):
                if event.delta.type == "text_delta":
                    text_chunks.append(event.delta.text)

        final_message = stream.get_final_message()

    full_text = "".join(text_chunks)
    logger.info(f"Response: {full_text}")

    # Should get a text response
    assert len(full_text) > 0, "Expected text response from streaming"

    # Verify final message structure
    assert final_message is not None
    assert final_message.content and len(final_message.content) > 0


@pytest.mark.skip("unreliable - bedrock tests are flaky in CI")
def test_openai_client_streaming_with_bedrock():
    """Test OpenAI client using 'coding-model' alias (maps to Bedrock) with streaming"""
    logger.info(
        "Testing OpenAI client with 'coding-model' alias -> Bedrock (streaming)"
    )

    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = openai.OpenAI(
        api_key="test-key",
        base_url=f"{base_url}/v1",
    )

    stream = client.chat.completions.create(
        model="coding-model",  # This should resolve to us.amazon.nova-premier-v1:0
        max_tokens=500,
        messages=[
            {
                "role": "user",
                "content": "Write a short 4-line sonnet about coding.",
            }
        ],
        stream=True,
    )

    content_chunks = []
    for chunk in stream:
        if chunk.choices and len(chunk.choices) > 0:
            delta = chunk.choices[0].delta
            if delta.content:
                content_chunks.append(delta.content)

    full_content = "".join(content_chunks)
    logger.info(f"Streaming response from coding-model: {full_content}")

    # Should get a text response
    assert len(full_content) > 0, "Expected text response from streaming"


@pytest.mark.skip("unreliable - bedrock tests are flaky in CI")
def test_openai_client_streaming_with_bedrock_and_tools():
    """Test OpenAI client using 'coding-model' alias (maps to Bedrock) with streaming and tools"""
    logger.info(
        "Testing OpenAI client with 'coding-model' alias -> Bedrock with tools (streaming)"
    )

    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")
    client = openai.OpenAI(
        api_key="test-key",
        base_url=f"{base_url}/v1",
    )

    stream = client.chat.completions.create(
        model="coding-model",  # This should resolve to us.amazon.nova-premier-v1:0
        max_tokens=1000,
        messages=[
            {
                "role": "user",
                "content": "I need to write a Python function that calculates the factorial of a number. Can you help me write and run it?. You should use the tool to run the code.",
            }
        ],
        tools=[
            {
                "type": "function",
                "function": {
                    "name": "run_python_code",
                    "description": "Execute Python code and return the result",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "code": {
                                "type": "string",
                                "description": "Python code to execute",
                            }
                        },
                        "required": ["code"],
                    },
                },
            }
        ],
        tool_choice="auto",
        stream=True,
    )

    content_chunks = []
    tool_calls = []
    chunk_count = 0

    for chunk in stream:
        chunk_count += 1
        if chunk.choices and len(chunk.choices) > 0:
            delta = chunk.choices[0].delta

            # Log what we see in each chunk
            has_content = delta.content is not None
            has_tool_calls = delta.tool_calls is not None

            if (
                chunk_count % 50 == 0 or has_tool_calls
            ):  # Log every 50th chunk or any chunk with tool calls
                logger.info(
                    f"Chunk {chunk_count}: content={has_content}, tool_calls={has_tool_calls}"
                )
                if has_tool_calls:
                    logger.info(f"  Tool calls in chunk: {delta.tool_calls}")

            # Collect text content
            if delta.content:
                content_chunks.append(delta.content)

            # Collect tool calls
            if delta.tool_calls:
                for tool_call in delta.tool_calls:
                    # Extend or create tool call entries
                    while len(tool_calls) <= tool_call.index:
                        tool_calls.append(
                            {
                                "id": "",
                                "type": "function",
                                "function": {"name": "", "arguments": ""},
                            }
                        )

                    if tool_call.id:
                        tool_calls[tool_call.index]["id"] = tool_call.id
                    if tool_call.function:
                        if tool_call.function.name:
                            tool_calls[tool_call.index]["function"][
                                "name"
                            ] = tool_call.function.name
                        if tool_call.function.arguments:
                            tool_calls[tool_call.index]["function"][
                                "arguments"
                            ] += tool_call.function.arguments

    full_content = "".join(content_chunks)
    logger.info(f"Streaming response from coding-model with tools: {full_content}")
    logger.info(f"Tool calls collected: {len(tool_calls)}")

    if tool_calls:
        for i, tc in enumerate(tool_calls):
            logger.info(f"  Tool call {i}: {tc['function']['name']}")

    # Should get either text response or tool calls for coding assistance
    assert (
        full_content or len(tool_calls) > 0
    ), f"Expected text or tool calls. Got text_len={len(full_content)}, tools={len(tool_calls)}"
