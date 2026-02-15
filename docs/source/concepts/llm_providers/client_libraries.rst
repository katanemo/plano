.. _client_libraries:

Client Libraries
================

Plano provides a unified interface that works seamlessly with multiple client libraries and tools. You can use your preferred client library without changing your existing code - just point it to Plano's gateway endpoints.

Supported Clients
------------------

- **OpenAI SDK** - Full compatibility with OpenAI's official client
- **Anthropic SDK** - Native support for Anthropic's client library
- **cURL** - Direct HTTP requests for any programming language
- **Custom HTTP Clients** - Any HTTP client that supports REST APIs

Gateway Endpoints
-----------------

Plano exposes three main endpoints:

.. list-table::
   :header-rows: 1
   :widths: 40 60

   * - Endpoint
     - Purpose
   * - ``http://127.0.0.1:12000/v1/chat/completions``
     - OpenAI-compatible chat completions (LLM Gateway)
   * - ``http://127.0.0.1:12000/v1/responses``
     - OpenAI Responses API with :ref:`conversational state management <managing_conversational_state>` (LLM Gateway)
   * - ``http://127.0.0.1:12000/v1/messages``
     - Anthropic-compatible messages (LLM Gateway)

OpenAI (Python) SDK
-------------------

The OpenAI SDK works with any provider through Plano's OpenAI-compatible endpoint.

**Installation:**

.. code-block:: bash

    pip install openai

**Basic Usage:**

.. code-block:: python

    from openai import OpenAI

    # Point to Plano's LLM Gateway
    client = OpenAI(
        api_key="test-key",  # Can be any value for local testing
        base_url="http://127.0.0.1:12000/v1"
    )

    # Use any model configured in your plano_config.yaml
    completion = client.chat.completions.create(
        model="gpt-4o-mini",  # Or use :ref:`model aliases <model_aliases>` like "fast-model"
        max_tokens=50,
        messages=[
            {
                "role": "user",
                "content": "Hello, how are you?"
            }
        ]
    )

    print(completion.choices[0].message.content)

**Streaming Responses:**

.. code-block:: python

    from openai import OpenAI

    client = OpenAI(
        api_key="test-key",
        base_url="http://127.0.0.1:12000/v1"
    )

    stream = client.chat.completions.create(
        model="gpt-4o-mini",
        max_tokens=50,
        messages=[
            {
                "role": "user",
                "content": "Tell me a short story"
            }
        ],
        stream=True
    )

    # Collect streaming chunks
    for chunk in stream:
        if chunk.choices[0].delta.content:
            print(chunk.choices[0].delta.content, end="")

**Using with Non-OpenAI Models:**

The OpenAI SDK can be used with any provider configured in Plano:

.. code-block:: python

    # Using Claude model through OpenAI SDK
    completion = client.chat.completions.create(
        model="claude-3-5-sonnet-20241022",
        max_tokens=50,
        messages=[
            {
                "role": "user",
                "content": "Explain quantum computing briefly"
            }
        ]
    )

    # Using Ollama model through OpenAI SDK
    completion = client.chat.completions.create(
        model="llama3.1",
        max_tokens=50,
        messages=[
            {
                "role": "user",
                "content": "What's the capital of France?"
            }
        ]
    )

OpenAI Responses API (Conversational State)
-------------------------------------------

The OpenAI Responses API (``v1/responses``) enables multi-turn conversations with automatic state management. Plano handles conversation history for you, so you don't need to manually include previous messages in each request.

See :ref:`managing_conversational_state` for detailed configuration and storage backend options.

**Installation:**

.. code-block:: bash

    pip install openai

**Basic Multi-Turn Conversation:**

.. code-block:: python

    from openai import OpenAI

    # Point to Plano's LLM Gateway
    client = OpenAI(
        api_key="test-key",
        base_url="http://127.0.0.1:12000/v1"
    )

    # First turn - creates a new conversation
    response = client.chat.completions.create(
        model="gpt-4o-mini",
        messages=[
            {"role": "user", "content": "My name is Alice"}
        ]
    )

    # Extract response_id for conversation continuity
    response_id = response.id
    print(f"Assistant: {response.choices[0].message.content}")

    # Second turn - continues the conversation
    # Plano automatically retrieves and merges previous context
    response = client.chat.completions.create(
        model="gpt-4o-mini",
        messages=[
            {"role": "user", "content": "What's my name?"}
        ],
        metadata={"response_id": response_id}  # Reference previous conversation
    )

    print(f"Assistant: {response.choices[0].message.content}")
    # Output: "Your name is Alice"

**Using with Any Provider:**

The Responses API works with any LLM provider configured in Plano:

.. code-block:: python

    # Multi-turn conversation with Claude
    response = client.chat.completions.create(
        model="claude-3-5-sonnet-20241022",
        messages=[
            {"role": "user", "content": "Let's discuss quantum physics"}
        ]
    )

    response_id = response.id

    # Continue conversation - Plano manages state regardless of provider
    response = client.chat.completions.create(
        model="claude-3-5-sonnet-20241022",
        messages=[
            {"role": "user", "content": "Tell me more about entanglement"}
        ],
        metadata={"response_id": response_id}
    )

**Key Benefits:**

* **Reduced payload size**: No need to send full conversation history in each request
* **Provider flexibility**: Use any configured LLM provider with state management
* **Automatic context merging**: Plano handles conversation continuity behind the scenes
* **Production-ready storage**: Configure :ref:`PostgreSQL or memory storage <managing_conversational_state>` based on your needs

Anthropic (Python) SDK
----------------------

The Anthropic SDK works with any provider through Plano's Anthropic-compatible endpoint.

**Installation:**

.. code-block:: bash

    pip install anthropic

**Basic Usage:**

.. code-block:: python

    import anthropic

    # Point to Plano's LLM Gateway
    client = anthropic.Anthropic(
        api_key="test-key",  # Can be any value for local testing
        base_url="http://127.0.0.1:12000"
    )

    # Use any model configured in your plano_config.yaml
    message = client.messages.create(
        model="claude-3-5-sonnet-20241022",
        max_tokens=50,
        messages=[
            {
                "role": "user",
                "content": "Hello, please respond briefly!"
            }
        ]
    )

    print(message.content[0].text)

**Streaming Responses:**

.. code-block:: python

    import anthropic

    client = anthropic.Anthropic(
        api_key="test-key",
        base_url="http://127.0.0.1:12000"
    )

    with client.messages.stream(
        model="claude-3-5-sonnet-20241022",
        max_tokens=50,
        messages=[
            {
                "role": "user",
                "content": "Tell me about artificial intelligence"
            }
        ]
    ) as stream:
        # Collect text deltas
        for text in stream.text_stream:
            print(text, end="")

        # Get final assembled message
        final_message = stream.get_final_message()
        final_text = "".join(block.text for block in final_message.content if block.type == "text")

**Using with Non-Anthropic Models:**

The Anthropic SDK can be used with any provider configured in Plano:

.. code-block:: python

    # Using OpenAI model through Anthropic SDK
    message = client.messages.create(
        model="gpt-4o-mini",
        max_tokens=50,
        messages=[
            {
                "role": "user",
                "content": "Explain machine learning in simple terms"
            }
        ]
    )

    # Using Ollama model through Anthropic SDK
    message = client.messages.create(
        model="llama3.1",
        max_tokens=50,
        messages=[
            {
                "role": "user",
                "content": "What is Python programming?"
            }
        ]
    )

cURL Examples
-------------

For direct HTTP requests or integration with any programming language:

**OpenAI-Compatible Endpoint:**

.. code-block:: bash

    # Basic request
    curl -X POST http://127.0.0.1:12000/v1/chat/completions \
      -H "Content-Type: application/json" \
      -H "Authorization: Bearer test-key" \
      -d '{
        "model": "gpt-4o-mini",
        "messages": [
          {"role": "user", "content": "Hello!"}
        ],
        "max_tokens": 50
      }'

    # Using :ref:`model aliases <model_aliases>`
    curl -X POST http://127.0.0.1:12000/v1/chat/completions \
      -H "Content-Type: application/json" \
      -d '{
        "model": "fast-model",
        "messages": [
          {"role": "user", "content": "Summarize this text..."}
        ],
        "max_tokens": 100
      }'

    # Streaming request
    curl -X POST http://127.0.0.1:12000/v1/chat/completions \
      -H "Content-Type: application/json" \
      -d '{
        "model": "gpt-4o-mini",
        "messages": [
          {"role": "user", "content": "Tell me a story"}
        ],
        "stream": true,
        "max_tokens": 200
      }'

**Anthropic-Compatible Endpoint:**

.. code-block:: bash

    # Basic request
    curl -X POST http://127.0.0.1:12000/v1/messages \
      -H "Content-Type: application/json" \
      -H "x-api-key: test-key" \
      -H "anthropic-version: 2023-06-01" \
      -d '{
        "model": "claude-3-5-sonnet-20241022",
        "max_tokens": 50,
        "messages": [
          {"role": "user", "content": "Hello Claude!"}
        ]
      }'

Cross-Client Compatibility
--------------------------

One of Plano's key features is cross-client compatibility. You can:

**Use OpenAI SDK with Claude Models:**

.. code-block:: python

    # OpenAI client calling Claude model
    from openai import OpenAI

    client = OpenAI(base_url="http://127.0.0.1:12000/v1", api_key="test")

    response = client.chat.completions.create(
        model="claude-3-5-sonnet-20241022",  # Claude model
        messages=[{"role": "user", "content": "Hello"}]
    )

**Use Anthropic SDK with OpenAI Models:**

.. code-block:: python

    # Anthropic client calling OpenAI model
    import anthropic

    client = anthropic.Anthropic(base_url="http://127.0.0.1:12000", api_key="test")

    response = client.messages.create(
        model="gpt-4o-mini",  # OpenAI model
        max_tokens=50,
        messages=[{"role": "user", "content": "Hello"}]
    )

**Mix and Match with** :ref:`Model Aliases <model_aliases>`:

.. code-block:: python

    # Same code works with different underlying models
    def ask_question(client, question):
        return client.chat.completions.create(
            model="reasoning-model",  # Alias could point to any provider
            messages=[{"role": "user", "content": question}]
        )

    # Works regardless of what "reasoning-model" actually points to
    openai_client = OpenAI(base_url="http://127.0.0.1:12000/v1", api_key="test")
    response = ask_question(openai_client, "Solve this math problem...")

Error Handling
--------------

**OpenAI SDK Error Handling:**

.. code-block:: python

    from openai import OpenAI
    import openai

    client = OpenAI(base_url="http://127.0.0.1:12000/v1", api_key="test")

    try:
        completion = client.chat.completions.create(
            model="nonexistent-model",
            messages=[{"role": "user", "content": "Hello"}]
        )
    except openai.NotFoundError as e:
        print(f"Model not found: {e}")
    except openai.APIError as e:
        print(f"API error: {e}")

**Anthropic SDK Error Handling:**

.. code-block:: python

    import anthropic

    client = anthropic.Anthropic(base_url="http://127.0.0.1:12000", api_key="test")

    try:
        message = client.messages.create(
            model="nonexistent-model",
            max_tokens=50,
            messages=[{"role": "user", "content": "Hello"}]
        )
    except anthropic.NotFoundError as e:
        print(f"Model not found: {e}")
    except anthropic.APIError as e:
        print(f"API error: {e}")

Best Practices
--------------

**Use** :ref:`Model Aliases <model_aliases>`:
Instead of hardcoding provider-specific model names, use semantic aliases:

.. code-block:: python

    # Good - uses semantic alias
    model = "fast-model"

    # Less ideal - hardcoded provider model
    model = "openai/gpt-4o-mini"

**Environment-Based Configuration:**
Use different :ref:`model aliases <model_aliases>` for different environments:

.. code-block:: python

    import os

    # Development uses cheaper/faster models
    model = os.getenv("MODEL_ALIAS", "dev.chat.v1")

    response = client.chat.completions.create(
        model=model,
        messages=[{"role": "user", "content": "Hello"}]
    )

**Graceful Fallbacks:**
Implement fallback logic for better reliability:

.. code-block:: python

    def chat_with_fallback(client, messages, primary_model="smart-model", fallback_model="fast-model"):
        try:
            return client.chat.completions.create(model=primary_model, messages=messages)
        except Exception as e:
            print(f"Primary model failed, trying fallback: {e}")
            return client.chat.completions.create(model=fallback_model, messages=messages)

See Also
--------

- :ref:`supported_providers` - Configure your providers and see available models
- :ref:`model_aliases` - Create semantic model names
- :ref:`llm_router` - Intelligent routing capabilities
