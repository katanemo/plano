.. _model_aliases:

Model Aliases
=============

Model aliases provide semantic, version-controlled names for your models, enabling cleaner client code, easier model management, and advanced routing capabilities. Instead of using provider-specific model names like ``gpt-4o-mini`` or ``claude-3-5-sonnet-20241022``, you can create meaningful aliases like ``fast-model`` or ``arch.summarize.v1``.

**Benefits of Model Aliases:**

- **Semantic Naming**: Use descriptive names that reflect the model's purpose
- **Version Control**: Implement versioning schemes (e.g., ``v1``, ``v2``) for model upgrades
- **Environment Management**: Different aliases can point to different models across environments
- **Client Simplification**: Clients use consistent, meaningful names regardless of underlying provider
- **Advanced Routing (Coming Soon)**: Enable guardrails, fallbacks, and traffic splitting at the alias level

Basic Configuration
-------------------

**Simple Alias Mapping**

.. code-block:: yaml
    :caption: Basic Model Aliases

    llm_providers:
      - model: openai/gpt-4o-mini
        access_key: $OPENAI_API_KEY

      - model: openai/gpt-4o
        access_key: $OPENAI_API_KEY

      - model: anthropic/claude-3-5-sonnet-20241022
        access_key: $ANTHROPIC_API_KEY

      - model: ollama/llama3.1
        base_url: http://localhost:11434

    # Define aliases that map to the models above
    model_aliases:
      # Semantic versioning approach
      arch.summarize.v1:
        target: gpt-4o-mini

      arch.reasoning.v1:
        target: gpt-4o

      arch.creative.v1:
        target: claude-3-5-sonnet-20241022

      # Functional aliases
      fast-model:
        target: gpt-4o-mini

      smart-model:
        target: gpt-4o

      creative-model:
        target: claude-3-5-sonnet-20241022

      # Local model alias
      local-chat:
        target: llama3.1

Using Aliases
-------------

**Client Code Examples**

Once aliases are configured, clients can use semantic names instead of provider-specific model names:

.. code-block:: python
    :caption: Python Client Usage

    from openai import OpenAI

    client = OpenAI(base_url="http://127.0.0.1:12000/")

    # Use semantic alias instead of provider model name
    response = client.chat.completions.create(
        model="arch.summarize.v1",  # Points to gpt-4o-mini
        messages=[{"role": "user", "content": "Summarize this document..."}]
    )

    # Switch to a different capability
    response = client.chat.completions.create(
        model="arch.reasoning.v1",  # Points to gpt-4o
        messages=[{"role": "user", "content": "Solve this complex problem..."}]
    )

.. code-block:: bash
    :caption: cURL Example

    curl -X POST http://127.0.0.1:12000/v1/chat/completions \
      -H "Content-Type: application/json" \
      -d '{
        "model": "fast-model",
        "messages": [{"role": "user", "content": "Hello!"}]
      }'

Naming Best Practices
---------------------

**Semantic Versioning**

Use version numbers for backward compatibility and gradual model upgrades:

.. code-block:: yaml

    model_aliases:
      # Current production version
      arch.summarize.v1:
        target: gpt-4o-mini

      # Beta version for testing
      arch.summarize.v2:
        target: gpt-4o

      # Stable alias that always points to latest
      arch.summarize.latest:
        target: gpt-4o-mini

**Purpose-Based Naming**

Create aliases that reflect the intended use case:

.. code-block:: yaml

    model_aliases:
      # Task-specific
      code-reviewer:
        target: gpt-4o

      document-summarizer:
        target: gpt-4o-mini

      creative-writer:
        target: claude-3-5-sonnet-20241022

      data-analyst:
        target: gpt-4o

**Environment-Specific Aliases**

Different environments can use different underlying models:

.. code-block:: yaml

    model_aliases:
      # Development environment - use faster/cheaper models
      dev.chat.v1:
        target: gpt-4o-mini

      # Production environment - use more capable models
      prod.chat.v1:
        target: gpt-4o

      # Staging environment - test new models
      staging.chat.v1:
        target: claude-3-5-sonnet-20241022

Advanced Features (Coming Soon)
--------------------------------

The following features are planned for future releases of model aliases:

**Guardrails Integration**

Apply safety, cost, or latency rules at the alias level:

.. code-block:: yaml
    :caption: Future Feature - Guardrails

    model_aliases:
      arch.reasoning.v1:
        target: gpt-oss-120b
        guardrails:
          max_latency: 5s
          max_cost_per_request: 0.10
          block_categories: ["jailbreak", "PII"]
          content_filters:
            - type: "profanity"
            - type: "sensitive_data"

**Fallback Chains**

Provide a chain of models if the primary target fails or hits quota limits:

.. code-block:: yaml
    :caption: Future Feature - Fallbacks

    model_aliases:
      arch.summarize.v1:
        target: gpt-4o-mini
        fallbacks:
          - target: llama3.1
            conditions: ["quota_exceeded", "timeout"]
          - target: claude-3-haiku-20240307
            conditions: ["primary_and_first_fallback_failed"]

**Traffic Splitting & Canary Deployments**

Distribute traffic across multiple models for A/B testing or gradual rollouts:

.. code-block:: yaml
    :caption: Future Feature - Traffic Splitting

    model_aliases:
      arch.v1:
        targets:
          - model: llama3.1
            weight: 80
          - model: gpt-4o-mini
            weight: 20

      # Canary deployment
      arch.experimental.v1:
        targets:
          - model: gpt-4o      # Current stable
            weight: 95
          - model: o1-preview  # New model being tested
            weight: 5

**Load Balancing**

Distribute requests across multiple instances of the same model:

.. code-block:: yaml
    :caption: Future Feature - Load Balancing

    model_aliases:
      high-throughput-chat:
        load_balance:
          algorithm: "round_robin"  # or "least_connections", "weighted"
        targets:
          - model: gpt-4o-mini
            endpoint: "https://api-1.example.com"
          - model: gpt-4o-mini
            endpoint: "https://api-2.example.com"
          - model: gpt-4o-mini
            endpoint: "https://api-3.example.com"


Validation Rules
----------------

- Alias names must be valid identifiers (alphanumeric, dots, hyphens, underscores)
- Target models must be defined in the ``llm_providers`` section
- Circular references between aliases are not allowed
- Weights in traffic splitting must sum to 100

See Also
--------

- :ref:`llm_providers` - Learn about configuring LLM providers
- :ref:`llm_router` - Understand how aliases work with intelligent routing
