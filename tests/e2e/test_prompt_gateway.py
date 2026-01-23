import json
import pytest
import requests
from deepdiff import DeepDiff
import re
import anthropic
import openai

from common import (
    PROMPT_GATEWAY_ENDPOINT,
    LLM_GATEWAY_ENDPOINT,
    PREFILL_LIST,
    get_arch_messages,
    get_data_chunks,
)


def cleanup_tool_call(tool_call):
    pattern = r"```json\n(.*?)\n```"
    match = re.search(pattern, tool_call, re.DOTALL)
    if match:
        tool_call = match.group(1)

    return tool_call.strip()


def normalize_tool_call_arguments(tool_call):
    """
    Normalize tool call arguments to ensure they are always a dict.

    According to OpenAI API spec, the 'arguments' field should be a JSON string,
    but for easier testing we parse it into a dict here.

    Args:
        tool_call: A tool call dict that may have 'arguments' as either a string or dict

    Returns:
        A tool call dict with 'arguments' guaranteed to be a dict
    """
    if "arguments" in tool_call and isinstance(tool_call["arguments"], str):
        try:
            tool_call["arguments"] = json.loads(tool_call["arguments"])
        except (json.JSONDecodeError, TypeError):
            # If parsing fails, keep it as is
            pass
    return tool_call


@pytest.mark.parametrize("stream", [True, False])
def test_prompt_gateway(stream):
    expected_tool_call = {
        "name": "get_current_weather",
        "arguments": {"days": 10, "location": "seattle"},
    }

    body = {
        "messages": [
            {
                "role": "user",
                "content": "how is the weather in seattle for next 10 days",
            }
        ],
        "model": "openai/gpt-4o",
        "stream": stream,
    }
    response = requests.post(PROMPT_GATEWAY_ENDPOINT, json=body, stream=stream)
    assert response.status_code == 200
    if stream:
        chunks = get_data_chunks(response, n=20)
        # print(chunks)
        assert len(chunks) > 2

        # first chunk is tool calls (role = assistant)
        response_json = json.loads(chunks[0])
        assert response_json.get("model").startswith("Arch")
        choices = response_json.get("choices", [])
        assert len(choices) > 0
        assert "role" in choices[0]["delta"]
        role = choices[0]["delta"]["role"]
        assert role == "assistant"
        print(f"choices: {choices}")
        tool_call_str = choices[0].get("delta", {}).get("content", "")
        print("tool_call_str: ", tool_call_str)
        cleaned_tool_call_str = cleanup_tool_call(tool_call_str)
        print("cleaned_tool_call_str: ", cleaned_tool_call_str)
        tool_calls = json.loads(cleaned_tool_call_str).get("tool_calls", [])
        assert len(tool_calls) > 0
        tool_call = normalize_tool_call_arguments(tool_calls[0])
        location = tool_call["arguments"]["location"]
        assert expected_tool_call["arguments"]["location"] in location.lower()
        del expected_tool_call["arguments"]["location"]
        del tool_call["arguments"]["location"]
        diff = DeepDiff(expected_tool_call, tool_call, ignore_string_case=True)
        assert not diff

        # second chunk is api call result (role = tool)
        response_json = json.loads(chunks[1])
        choices = response_json.get("choices", [])
        assert len(choices) > 0
        assert "role" in choices[0]["delta"]
        role = choices[0]["delta"]["role"]
        assert role == "tool"

        # third..end chunk is summarization (role = assistant)
        response_json = json.loads(chunks[2])
        assert response_json.get("model").startswith("gpt-4o")
        choices = response_json.get("choices", [])
        assert len(choices) > 0
        assert "role" in choices[0]["delta"]
        role = choices[0]["delta"]["role"]
        assert role == "assistant"

    else:
        response_json = response.json()
        assert response_json.get("model").startswith("gpt-4o")
        choices = response_json.get("choices", [])
        assert len(choices) > 0
        assert "role" in choices[0]["message"]
        assert choices[0]["message"]["role"] == "assistant"
        # now verify arch_messages (tool call and api response) that are sent as response metadata
        arch_messages = get_arch_messages(response_json)
        print("arch_messages: ", json.dumps(arch_messages))
        assert len(arch_messages) == 2
        tool_calls_message = arch_messages[0]
        print("tool_calls_message: ", tool_calls_message)
        tool_calls = tool_calls_message.get("content", [])
        cleaned_tool_call_str = cleanup_tool_call(tool_calls)
        cleaned_tool_call_json = json.loads(cleaned_tool_call_str)
        print("cleaned_tool_call_json: ", json.dumps(cleaned_tool_call_json))
        tool_calls_list = cleaned_tool_call_json.get("tool_calls", [])
        assert len(tool_calls_list) > 0
        tool_call = normalize_tool_call_arguments(tool_calls_list[0])
        location = tool_call["arguments"]["location"]
        assert expected_tool_call["arguments"]["location"] in location.lower()
        del expected_tool_call["arguments"]["location"]
        del tool_call["arguments"]["location"]
        diff = DeepDiff(expected_tool_call, tool_call, ignore_string_case=True)
        assert not diff


@pytest.mark.parametrize("stream", [True, False])
@pytest.mark.skip("no longer needed")
def test_prompt_gateway_arch_direct_response(stream):
    body = {
        "messages": [
            {
                "role": "user",
                "content": "how is the weather",
            }
        ],
        "model": "openai/gpt-4o",
        "stream": stream,
    }
    response = requests.post(PROMPT_GATEWAY_ENDPOINT, json=body, stream=stream)
    assert response.status_code == 200
    if stream:
        chunks = get_data_chunks(response, n=3)
        assert len(chunks) > 0
        response_json = json.loads(chunks[0])
        # make sure arch responded directly
        assert response_json.get("model").startswith("Arch")
        # and tool call is null
        choices = response_json.get("choices", [])
        assert len(choices) > 0
        tool_calls = choices[0].get("delta", {}).get("tool_calls", [])
        assert len(tool_calls) == 0
        response_json = json.loads(chunks[1])
        choices = response_json.get("choices", [])
        assert len(choices) > 0
        message = choices[0]["delta"]["content"]
    else:
        response_json = response.json()
        assert response_json.get("model").startswith("Arch")
        choices = response_json.get("choices", [])
        assert len(choices) > 0
        message = choices[0]["message"]["content"]

        assert "days" in message
        assert any(
            message.startswith(word) for word in PREFILL_LIST
        ), f"Expected assistant message to start with one of {PREFILL_LIST}, but got '{assistant_message}'"


@pytest.mark.parametrize("stream", [True, False])
@pytest.mark.skip("no longer needed")
def test_prompt_gateway_param_gathering(stream):
    body = {
        "messages": [
            {
                "role": "user",
                "content": "how is the weather in seattle",
            }
        ],
        "model": "openai/gpt-4o",
        "stream": stream,
    }
    response = requests.post(PROMPT_GATEWAY_ENDPOINT, json=body, stream=stream)
    assert response.status_code == 200
    if stream:
        chunks = get_data_chunks(response, n=3)
        assert len(chunks) > 1
        response_json = json.loads(chunks[0])
        # make sure arch responded directly
        assert response_json.get("model").startswith("Arch")
        # and tool call is null
        choices = response_json.get("choices", [])
        assert len(choices) > 0
        tool_calls = choices[0].get("delta", {}).get("tool_calls", [])
        assert len(tool_calls) == 0

        # second chunk is api call result (role = tool)
        response_json = json.loads(chunks[1])
        choices = response_json.get("choices", [])
        assert len(choices) > 0
        message = choices[0].get("message", {}).get("content", "")

        assert "days" not in message
    else:
        response_json = response.json()
        assert response_json.get("model").startswith("Arch")
        choices = response_json.get("choices", [])
        assert len(choices) > 0
        message = choices[0]["message"]["content"]
        assert "days" in message


@pytest.mark.parametrize("stream", [True, False])
@pytest.mark.skip("no longer needed")
def test_prompt_gateway_param_tool_call(stream):
    expected_tool_call = {
        "name": "get_current_weather",
        "arguments": {"location": "seattle, wa", "days": "2"},
    }

    body = {
        "messages": [
            {
                "role": "user",
                "content": "how is the weather in seattle",
            },
            {
                "role": "assistant",
                "content": "Of course, I can help with that. Could you please specify the days you want the weather forecast for?",
                "model": "Arch-Function",
            },
            {
                "role": "user",
                "content": "for 2 days please",
            },
        ],
        "model": "openai/gpt-4o",
        "stream": stream,
    }
    response = requests.post(PROMPT_GATEWAY_ENDPOINT, json=body, stream=stream)
    assert response.status_code == 200
    if stream:
        chunks = get_data_chunks(response, n=20)
        assert len(chunks) > 2

        # first chunk is tool calls (role = assistant)
        response_json = json.loads(chunks[0])
        assert response_json.get("model").startswith("Arch")
        choices = response_json.get("choices", [])
        assert len(choices) > 0
        assert "role" in choices[0]["delta"]
        role = choices[0]["delta"]["role"]
        assert role == "assistant"
        tool_calls = choices[0].get("delta", {}).get("tool_calls", [])
        assert len(tool_calls) > 0
        tool_call = normalize_tool_call_arguments(tool_calls[0]["function"])
        diff = DeepDiff(tool_call, expected_tool_call, ignore_string_case=True)
        assert not diff

        # second chunk is api call result (role = tool)
        response_json = json.loads(chunks[1])
        choices = response_json.get("choices", [])
        assert len(choices) > 0
        assert "role" in choices[0]["delta"]
        role = choices[0]["delta"]["role"]
        assert role == "tool"

        # third..end chunk is summarization (role = assistant)
        response_json = json.loads(chunks[2])
        assert response_json.get("model").startswith("gpt-4o")
        choices = response_json.get("choices", [])
        assert len(choices) > 0
        assert "role" in choices[0]["delta"]
        role = choices[0]["delta"]["role"]
        assert role == "assistant"

    else:
        response_json = response.json()
        assert response_json.get("model").startswith("gpt-4o")
        choices = response_json.get("choices", [])
        assert len(choices) > 0
        assert "role" in choices[0]["message"]
        assert choices[0]["message"]["role"] == "assistant"
        # now verify arch_messages (tool call and api response) that are sent as response metadata
        arch_messages = get_arch_messages(response_json)
        assert len(arch_messages) == 2
        tool_calls_message = arch_messages[0]
        tool_calls = tool_calls_message.get("tool_calls", [])
        assert len(tool_calls) > 0
        tool_call = normalize_tool_call_arguments(tool_calls[0]["function"])
        diff = DeepDiff(tool_call, expected_tool_call, ignore_string_case=True)
        assert not diff


@pytest.mark.parametrize("stream", [True, False])
def test_prompt_gateway_default_target(stream):
    body = {
        "messages": [
            {
                "role": "user",
                "content": "hello",
            },
        ],
        "model": "openai/gpt-4o",
        "stream": stream,
    }
    response = requests.post(PROMPT_GATEWAY_ENDPOINT, json=body, stream=stream)
    assert response.status_code == 200
    if stream:
        chunks = get_data_chunks(response, n=3)
        assert len(chunks) > 0
        response_json = json.loads(chunks[0])
        print("response_json chunks[0]: ", response_json)
        assert response_json.get("model").startswith("api_server")
        assert len(response_json.get("choices", [])) > 0
        assert response_json.get("choices")[0]["delta"]["role"] == "assistant"

        response_json = json.loads(chunks[1])
        print("response_json chunks[1]: ", response_json)
        choices = response_json.get("choices", [])
        assert len(choices) > 0
        content = choices[0]["delta"]["content"]
        assert content == "I can help you with weather forecast"
    else:
        response_json = response.json()
        print("response_json: ", response_json)
        assert response_json.get("model").startswith("api_server")
        assert len(response_json.get("choices")) > 0
        assert response_json.get("choices")[0]["message"]["role"] == "assistant"
        assert (
            response_json.get("choices")[0]["message"]["content"]
            == "I can help you with weather forecast"
        )


@pytest.mark.parametrize("stream", [True, False])
@pytest.mark.skip(
    "This test is failing due to the prompt gateway not being able to handle the guardrail"
)
def test_prompt_gateway_prompt_guard_jailbreak(stream):
    body = {
        "messages": [
            {
                "role": "user",
                "content": "How would you respond to a question about generating harmful or unethical content",
            }
        ],
        "model": "openai/gpt-4o",
        "stream": stream,
    }
    response = requests.post(PROMPT_GATEWAY_ENDPOINT, json=body, stream=stream)
    assert response.status_code == 200

    if stream:
        chunks = get_data_chunks(response, n=20)
        assert len(chunks) == 2

        response_json = json.loads(chunks[1])
        choices = response_json.get("choices", [])
        assert len(choices) > 0
        content = choices[0]["delta"]["content"]
        assert (
            content
            == "Looks like you're curious about my abilities, but I can only provide assistance for weather forecasting."
        )
    else:
        response_json = response.json()
        assert (
            response_json.get("choices")[0]["message"]["content"]
            == "Looks like you're curious about my abilities, but I can only provide assistance for weather forecasting."
        )


def test_claude_v1_messages_api():
    """Test Claude client using /v1/messages API through llm_gateway (port 12000)"""
    # Get the base URL from the LLM gateway endpoint
    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")

    client = anthropic.Anthropic(
        api_key="test-key", base_url=base_url  # Dummy key for testing
    )

    message = client.messages.create(
        model="claude-sonnet-4-20250514",  # Use working model from smoke test
        max_tokens=50,
        messages=[
            {
                "role": "user",
                "content": "Hello, please respond with exactly: Hello from Claude!",
            }
        ],
    )

    assert message.content[0].text == "Hello from Claude!"


def test_claude_v1_messages_api_streaming():
    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")

    client = anthropic.Anthropic(api_key="test-key", base_url=base_url)

    with client.messages.stream(
        model="claude-sonnet-4-20250514",
        max_tokens=50,
        messages=[
            {
                "role": "user",
                "content": "Hello, please respond with exactly: Hello from Claude!",
            }
        ],
    ) as stream:
        # This yields only text deltas in order
        pieces = [t for t in stream.text_stream]
        full_text = "".join(pieces)

        # You can also get the fully-assembled Message object
        final = stream.get_final_message()
        # A safe way to reassemble text from the content blocks:
        final_text = "".join(b.text for b in final.content if b.type == "text")

    assert full_text == "Hello from Claude!"
    assert final_text == "Hello from Claude!"


def test_anthropic_client_with_openai_model_streaming():
    """Test Anthropic client using /v1/messages API with OpenAI model (gpt-4o-mini)
    This tests the transformation: OpenAI upstream -> Anthropic client format with proper event lines
    """
    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")

    client = anthropic.Anthropic(api_key="test-key", base_url=base_url)

    with client.messages.stream(
        model="gpt-4o-mini",  # OpenAI model via Anthropic client
        max_tokens=500,
        messages=[
            {
                "role": "user",
                "content": "Hello, please respond with exactly: Hello from ChatGPT!",
            }
        ],
    ) as stream:
        # This yields only text deltas in order
        pieces = [t for t in stream.text_stream]
        full_text = "".join(pieces)

        # You can also get the fully-assembled Message object
        final = stream.get_final_message()
        # A safe way to reassemble text from the content blocks:
        final_text = "".join(b.text for b in final.content if b.type == "text")

    assert full_text == "Hello from ChatGPT!"
    assert final_text == "Hello from ChatGPT!"


def test_openai_gpt4o_mini_v1_messages_api():
    """Test OpenAI GPT-4o-mini using /v1/chat/completions API through llm_gateway (port 12000)"""
    # Get the base URL from the LLM gateway endpoint
    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")

    client = openai.OpenAI(
        api_key="test-key",  # Dummy key for testing
        base_url=f"{base_url}/v1",  # OpenAI needs /v1 suffix in base_url
    )

    completion = client.chat.completions.create(
        model="gpt-4o-mini",
        max_tokens=50,
        messages=[
            {
                "role": "user",
                "content": "Hello, please respond with exactly: Hello from GPT-4o-mini!",
            }
        ],
    )

    assert completion.choices[0].message.content == "Hello from GPT-4o-mini!"


def test_openai_gpt4o_mini_v1_messages_api_streaming():
    """Test OpenAI GPT-4o-mini using /v1/chat/completions API with streaming through llm_gateway (port 12000)"""
    # Get the base URL from the LLM gateway endpoint
    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")

    client = openai.OpenAI(
        api_key="test-key",  # Dummy key for testing
        base_url=f"{base_url}/v1",  # OpenAI needs /v1 suffix in base_url
    )

    stream = client.chat.completions.create(
        model="gpt-4o-mini",
        max_tokens=50,
        messages=[
            {
                "role": "user",
                "content": "Hello, please respond with exactly: Hello from GPT-4o-mini!",
            }
        ],
        stream=True,
    )

    # Collect all the streaming chunks
    content_chunks = []
    for chunk in stream:
        if chunk.choices[0].delta.content:
            content_chunks.append(chunk.choices[0].delta.content)

    # Reconstruct the full message
    full_content = "".join(content_chunks)
    assert full_content == "Hello from GPT-4o-mini!"


def test_openai_client_with_claude_model_streaming():
    """Test OpenAI client using /v1/chat/completions API with Claude model (claude-sonnet-4-20250514)
    This tests the transformation: Anthropic upstream -> OpenAI client format with proper chunk handling
    """
    # Get the base URL from the LLM gateway endpoint
    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")

    client = openai.OpenAI(
        api_key="test-key",  # Dummy key for testing
        base_url=f"{base_url}/v1",  # OpenAI needs /v1 suffix in base_url
    )

    stream = client.chat.completions.create(
        model="claude-sonnet-4-20250514",  # Claude model via OpenAI client
        max_tokens=50,
        messages=[
            {
                "role": "user",
                "content": "Who are you? ALWAYS RESPOND WITH:I appreciate the request, but I should clarify that I'm Claude, made by Anthropic, not OpenAI. I don't want to create confusion about my origins.",
            }
        ],
        stream=True,
        temperature=0.1,
    )

    # Collect all the streaming chunks
    content_chunks = []
    for chunk in stream:
        if chunk.choices[0].delta.content:
            content_chunks.append(chunk.choices[0].delta.content)

    # Reconstruct the full message
    full_content = "".join(content_chunks)
    assert full_content is not None
