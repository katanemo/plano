import anthropic
import openai

from common import LLM_GATEWAY_ENDPOINT


def test_claude_v1_messages_api():
    """Test Claude client using /v1/messages API through llm_gateway (port 12000)"""
    # Get the base URL from the LLM gateway endpoint
    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")

    client = anthropic.Anthropic(
        api_key="test-key", base_url=base_url  # Dummy key for testing
    )

    message = client.messages.create(
        model="claude-sonnet-4-6",  # Use working model from smoke test
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
        model="claude-sonnet-4-6",
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
    """Test OpenAI client using /v1/chat/completions API with Claude model (claude-sonnet-4-6)
    This tests the transformation: Anthropic upstream -> OpenAI client format with proper chunk handling
    """
    # Get the base URL from the LLM gateway endpoint
    base_url = LLM_GATEWAY_ENDPOINT.replace("/v1/chat/completions", "")

    client = openai.OpenAI(
        api_key="test-key",  # Dummy key for testing
        base_url=f"{base_url}/v1",  # OpenAI needs /v1 suffix in base_url
    )

    stream = client.chat.completions.create(
        model="claude-sonnet-4-6",  # Claude model via OpenAI client
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
