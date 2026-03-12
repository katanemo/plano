.. _llm_router:

LLM Routing
==============================================================

With the rapid proliferation of large language models (LLMs) — each optimized for different strengths, style, or latency/cost profile — routing has become an essential technique to operationalize the use of different models. Plano provides three distinct routing approaches to meet different use cases: :ref:`Model-based routing <model_based_routing>`, :ref:`Alias-based routing <alias_based_routing>`, and :ref:`Preference-aligned routing <preference_aligned_routing>`. This enables optimal performance, cost efficiency, and response quality by matching requests with the most suitable model from your available LLM fleet.

.. note::
  For details on supported model providers, configuration options, and client libraries, see :ref:`LLM Providers <llm_providers>`.

Routing Methods
---------------

.. _model_based_routing:

Model-based routing
~~~~~~~~~~~~~~~~~~~

Direct routing allows you to specify exact provider and model combinations using the format ``provider/model-name``:

- Use provider-specific names like ``openai/gpt-5.2`` or ``anthropic/claude-sonnet-4-5``
- Provides full control and transparency over which model handles each request
- Ideal for production workloads where you want predictable routing behavior

Configuration
^^^^^^^^^^^^^

Configure your LLM providers with specific provider/model names:

.. code-block:: yaml
    :caption: Model-based Routing Configuration

    listeners:
      egress_traffic:
        address: 0.0.0.0
        port: 12000
        message_format: openai
        timeout: 30s

    llm_providers:
      - model: openai/gpt-5.2
        access_key: $OPENAI_API_KEY
        default: true

      - model: openai/gpt-5
        access_key: $OPENAI_API_KEY

      - model: anthropic/claude-sonnet-4-5
        access_key: $ANTHROPIC_API_KEY

Client usage
^^^^^^^^^^^^

Clients specify exact models:

.. code-block:: python

    # Direct provider/model specification
    response = client.chat.completions.create(
        model="openai/gpt-5.2",
        messages=[{"role": "user", "content": "Hello!"}]
    )

    response = client.chat.completions.create(
        model="anthropic/claude-sonnet-4-5",
        messages=[{"role": "user", "content": "Write a story"}]
    )

.. _alias_based_routing:

Alias-based routing
~~~~~~~~~~~~~~~~~~~

Alias-based routing lets you create semantic model names that decouple your application from specific providers:

- Use meaningful names like ``fast-model``, ``reasoning-model``, or ``plano.summarize.v1`` (see :ref:`model_aliases`)
- Maps semantic names to underlying provider models for easier experimentation and provider switching
- Ideal for applications that want abstraction from specific model names while maintaining control

Configuration
^^^^^^^^^^^^^

Configure semantic aliases that map to underlying models:

.. code-block:: yaml
    :caption: Alias-based Routing Configuration

    listeners:
      egress_traffic:
        address: 0.0.0.0
        port: 12000
        message_format: openai
        timeout: 30s

    llm_providers:
      - model: openai/gpt-5.2
        access_key: $OPENAI_API_KEY

      - model: openai/gpt-5
        access_key: $OPENAI_API_KEY

      - model: anthropic/claude-sonnet-4-5
        access_key: $ANTHROPIC_API_KEY

    model_aliases:
      # Model aliases - friendly names that map to actual provider names
      fast-model:
        target: gpt-5.2

      reasoning-model:
        target: gpt-5

      creative-model:
        target: claude-sonnet-4-5

Client usage
^^^^^^^^^^^^

Clients use semantic names:

.. code-block:: python

    # Using semantic aliases
    response = client.chat.completions.create(
        model="fast-model",  # Routes to best available fast model
        messages=[{"role": "user", "content": "Quick summary please"}]
    )

    response = client.chat.completions.create(
        model="reasoning-model",  # Routes to best reasoning model
        messages=[{"role": "user", "content": "Solve this complex problem"}]
    )

.. _preference_aligned_routing:

Preference-aligned routing (Arch-Router)
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Preference-aligned routing uses the `Arch-Router <https://huggingface.co/katanemo/Arch-Router-1.5B>`_ model to pick the best LLM based on domain, action, and your configured preferences instead of hard-coding a model.

- **Domain**: High-level topic of the request (e.g., legal, healthcare, programming).
- **Action**: What the user wants to do (e.g., summarize, generate code, translate).
- **Routing preferences**: Your mapping from (domain, action) to preferred models.

Arch-Router analyzes each prompt to infer domain and action, then applies your preferences to select a model. This decouples **routing policy** (how to choose) from **model assignment** (what to run), making routing transparent, controllable, and easy to extend as you add or swap models.

Configuration
^^^^^^^^^^^^^

To configure preference-aligned dynamic routing, define routing preferences that map domains and actions to specific models:

.. code-block:: yaml
    :caption: Preference-Aligned Dynamic Routing Configuration

    listeners:
      egress_traffic:
        address: 0.0.0.0
        port: 12000
        message_format: openai
        timeout: 30s

    llm_providers:
      - model: openai/gpt-5.2
        access_key: $OPENAI_API_KEY
        default: true

      - model: openai/gpt-5
        access_key: $OPENAI_API_KEY
        routing_preferences:
          - name: code understanding
            description: understand and explain existing code snippets, functions, or libraries
          - name: complex reasoning
            description: deep analysis, mathematical problem solving, and logical reasoning

      - model: anthropic/claude-sonnet-4-5
        access_key: $ANTHROPIC_API_KEY
        routing_preferences:
          - name: creative writing
            description: creative content generation, storytelling, and writing assistance
          - name: code generation
            description: generating new code snippets, functions, or boilerplate based on user prompts

Client usage
^^^^^^^^^^^^

Clients can let the router decide or still specify aliases:

.. code-block:: python

    # Let Arch-Router choose based on content
    response = client.chat.completions.create(
        messages=[{"role": "user", "content": "Write a creative story about space exploration"}]
        # No model specified - router will analyze and choose claude-sonnet-4-5
    )


Arch-Router
-----------
The `Arch-Router <https://huggingface.co/katanemo/Arch-Router-1.5B>`_ is a state-of-the-art **preference-based routing model** specifically designed to address the limitations of traditional LLM routing. This compact 1.5B model delivers production-ready performance with low latency and high accuracy while solving key routing challenges.

**Addressing Traditional Routing Limitations:**

**Human Preference Alignment**
Unlike benchmark-driven approaches, Arch-Router learns to match queries with human preferences by using domain-action mappings that capture subjective evaluation criteria, ensuring routing decisions align with real-world user needs.

**Flexible Model Integration**
The system supports seamlessly adding new models for routing without requiring retraining or architectural modifications, enabling dynamic adaptation to evolving model landscapes.

**Preference-Encoded Routing**
Provides a practical mechanism to encode user preferences through domain-action mappings, offering transparent and controllable routing decisions that can be customized for specific use cases.

To support effective routing, Arch-Router introduces two key concepts:

- **Domain** – the high-level thematic category or subject matter of a request (e.g., legal, healthcare, programming).

- **Action** – the specific type of operation the user wants performed (e.g., summarization, code generation, booking appointment, translation).

Both domain and action configs are associated with preferred models or model variants. At inference time, Arch-Router analyzes the incoming prompt to infer its domain and action using semantic similarity, task indicators, and contextual cues. It then applies the user-defined routing preferences to select the model best suited to handle the request.

In summary, Arch-Router demonstrates:

- **Structured Preference Routing**: Aligns prompt request with model strengths using explicit domain–action mappings.

- **Transparent and Controllable**: Makes routing decisions transparent and configurable, empowering users to customize system behavior.

- **Flexible and Adaptive**: Supports evolving user needs, model updates, and new domains/actions without retraining the router.

- **Production-Ready Performance**: Optimized for low-latency, high-throughput applications in multi-model environments.


Self-hosting Arch-Router
------------------------

By default, Plano uses a hosted Arch-Router endpoint. To run Arch-Router locally, you can serve the model yourself using either **Ollama** or **vLLM**.

Using Ollama (recommended for local development)
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

1. **Install Ollama**

   Download and install from `ollama.ai <https://ollama.ai>`_.

2. **Pull and serve Arch-Router**

   .. code-block:: bash

       ollama pull hf.co/katanemo/Arch-Router-1.5B.gguf:Q4_K_M
       ollama serve

   This downloads the quantized GGUF model from HuggingFace and starts serving on ``http://localhost:11434``.

3. **Configure Plano to use local Arch-Router**

   .. code-block:: yaml

       routing:
         model: Arch-Router
         llm_provider: arch-router

       model_providers:
         - name: arch-router
           model: arch/hf.co/katanemo/Arch-Router-1.5B.gguf:Q4_K_M
           base_url: http://localhost:11434

         - model: openai/gpt-5.2
           access_key: $OPENAI_API_KEY
           default: true

         - model: anthropic/claude-sonnet-4-5
           access_key: $ANTHROPIC_API_KEY
           routing_preferences:
             - name: creative writing
               description: creative content generation, storytelling, and writing assistance

4. **Verify the model is running**

   .. code-block:: bash

       curl http://localhost:11434/v1/models

   You should see ``Arch-Router-1.5B`` listed in the response.

Using vLLM (recommended for production / EC2)
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

vLLM provides higher throughput and GPU optimizations suitable for production deployments.

1. **Install vLLM**

   .. code-block:: bash

       pip install vllm

2. **Download the model weights**

   The GGUF weights are downloaded automatically from HuggingFace on first use. To pre-download:

   .. code-block:: bash

       pip install huggingface_hub
       huggingface-cli download katanemo/Arch-Router-1.5B.gguf

3. **Start the vLLM server**

   After downloading, find the GGUF file and Jinja template in the HuggingFace cache:

   .. code-block:: bash

       # Find the downloaded files
       SNAPSHOT_DIR=$(ls -d ~/.cache/huggingface/hub/models--katanemo--Arch-Router-1.5B.gguf/snapshots/*/ | head -1)

       vllm serve ${SNAPSHOT_DIR}Arch-Router-1.5B-Q4_K_M.gguf \
           --host 0.0.0.0 \
           --port 10000 \
           --load-format gguf \
           --chat-template ${SNAPSHOT_DIR}template.jinja \
           --tokenizer katanemo/Arch-Router-1.5B \
           --served-model-name Arch-Router \
           --gpu-memory-utilization 0.3 \
           --tensor-parallel-size 1 \
           --enable-prefix-caching

4. **Configure Plano to use the vLLM endpoint**

   .. code-block:: yaml

       routing:
         model: Arch-Router
         llm_provider: arch-router

       model_providers:
         - name: arch-router
           model: Arch-Router
           base_url: http://<your-server-ip>:10000

         - model: openai/gpt-5.2
           access_key: $OPENAI_API_KEY
           default: true

         - model: anthropic/claude-sonnet-4-5
           access_key: $ANTHROPIC_API_KEY
           routing_preferences:
             - name: creative writing
               description: creative content generation, storytelling, and writing assistance

5. **Verify the server is running**

   .. code-block:: bash

       curl http://localhost:10000/health
       curl http://localhost:10000/v1/models


Combining Routing Methods
-------------------------

You can combine static model selection with dynamic routing preferences for maximum flexibility:

.. code-block:: yaml
    :caption: Hybrid Routing Configuration

    llm_providers:
      - model: openai/gpt-5.2
        access_key: $OPENAI_API_KEY
        default: true

      - model: openai/gpt-5
        access_key: $OPENAI_API_KEY
        routing_preferences:
          - name: complex_reasoning
            description: deep analysis and complex problem solving

      - model: anthropic/claude-sonnet-4-5
        access_key: $ANTHROPIC_API_KEY
        routing_preferences:
          - name: creative_tasks
            description: creative writing and content generation

    model_aliases:
      # Model aliases - friendly names that map to actual provider names
      fast-model:
        target: gpt-5.2

      reasoning-model:
        target: gpt-5

      # Aliases that can also participate in dynamic routing
      creative-model:
        target: claude-sonnet-4-5

This configuration allows clients to:

1. **Use direct model selection**: ``model="fast-model"``
2. **Let the router decide**: No model specified, router analyzes content

Example Use Cases
-----------------
Here are common scenarios where Arch-Router excels:

- **Coding Tasks**: Distinguish between code generation requests ("write a Python function"), debugging needs ("fix this error"), and code optimization ("make this faster"), routing each to appropriately specialized models.

- **Content Processing Workflows**: Classify requests as summarization ("summarize this document"), translation ("translate to Spanish"), or analysis ("what are the key themes"), enabling targeted model selection.

- **Multi-Domain Applications**: Accurately identify whether requests fall into legal, healthcare, technical, or general domains, even when the subject matter isn't explicitly stated in the prompt.

- **Conversational Routing**: Track conversation context to identify when topics shift between domains or when the type of assistance needed changes mid-conversation.

Best practices
--------------
- **💡Consistent Naming:**  Route names should align with their descriptions.

  - ❌ Bad:
    ```
    {"name": "math", "description": "handle solving quadratic equations"}
    ```
  - ✅ Good:
    ```
    {"name": "quadratic_equation", "description": "solving quadratic equations"}
    ```

- **💡 Clear Usage Description:**  Make your route names and descriptions specific, unambiguous, and minimizing overlap between routes. The Router performs better when it can clearly distinguish between different types of requests.

  - ❌ Bad:
    ```
    {"name": "math", "description": "anything closely related to mathematics"}
    ```
  - ✅ Good:
    ```
    {"name": "math", "description": "solving, explaining math problems, concepts"}
    ```

- **💡Nouns Descriptor:** Preference-based routers perform better with noun-centric descriptors, as they offer more stable and semantically rich signals for matching.

- **💡Domain Inclusion:** for best user experience, you should always include a domain route. This helps the router fall back to domain when action is not confidently inferred.

Unsupported Features
--------------------

The following features are **not supported** by the Arch-Router model:

- **Multi-modality**: The model is not trained to process raw image or audio inputs. It can handle textual queries *about* these modalities (e.g., "generate an image of a cat"), but cannot interpret encoded multimedia data directly.

- **Function calling**: Arch-Router is designed for **semantic preference matching**, not exact intent classification or tool execution. For structured function invocation, use models in the Plano Function Calling collection instead.

- **System prompt dependency**: Arch-Router routes based solely on the user’s conversation history. It does not use or rely on system prompts for routing decisions.
