.. _supported_providers:

Supported Providers & Configuration
===================================

Plano provides first-class support for multiple LLM providers through native integrations and OpenAI-compatible interfaces. This comprehensive guide covers all supported providers, their available chat models, and detailed configuration instructions.

.. note::
   **Model Support:** Plano supports all chat models from each provider, not just the examples shown in this guide. The configurations below demonstrate common models for reference, but you can use any chat model available from your chosen provider.

   Please refer to the quuickstart guide :ref:`here <llm_routing_quickstart>` to configure and use LLM providers via common client libraries like OpenAI and Anthropic Python SDKs, or via direct HTTP/cURL requests.


Configuration Structure
-----------------------

All providers are configured in the ``llm_providers`` section of your ``plano_config.yaml`` file:

.. code-block:: yaml

    llm_providers:
      # Provider configurations go here
      - model: provider/model-name
        access_key: $API_KEY
        # Additional provider-specific options

**Common Configuration Fields:**

- ``model``: Provider prefix and model name (format: ``provider/model-name`` or ``provider/*`` for wildcard expansion)
- ``access_key``: API key for authentication (supports environment variables)
- ``default``: Mark a model as the default (optional, boolean)
- ``name``: Custom name for the provider instance (optional)
- ``base_url``: Custom endpoint URL (required for some providers, optional for others - see :ref:`base_url_details`)

Provider Categories
-------------------

**First-Class Providers**
Native integrations with built-in support for provider-specific features and authentication.

**OpenAI-Compatible Providers**
Any provider that implements the OpenAI API interface can be configured using custom endpoints.

Supported API Endpoints
------------------------

Plano supports the following standardized endpoints across providers:

.. list-table::
   :header-rows: 1
   :widths: 30 30 40

   * - Endpoint
     - Purpose
     - Supported Clients
   * - ``/v1/chat/completions``
     - OpenAI-style chat completions
     - OpenAI SDK, cURL, custom clients
   * - ``/v1/messages``
     - Anthropic-style messages
     - Anthropic SDK, cURL, custom clients
   * - ``/v1/responses``
     - Unified response endpoint for agentic apps
     - All SDKs, cURL, custom clients

First-Class Providers
---------------------

OpenAI
~~~~~~

**Provider Prefix:** ``openai/``

**API Endpoint:** ``/v1/chat/completions``

**Authentication:** API Key - Get your OpenAI API key from `OpenAI Platform <https://platform.openai.com/api-keys>`_.

**Supported Chat Models:** All OpenAI chat models including GPT-5.2, GPT-5, GPT-4o, and all future releases.

.. list-table::
   :header-rows: 1
   :widths: 30 20 50

   * - Model Name
     - Model ID for Config
     - Description
   * - GPT-5.2
     - ``openai/gpt-5.2``
     - Next-generation model (use any model name from OpenAI's API)
   * - GPT-5
     - ``openai/gpt-5``
     - Latest multimodal model
   * - GPT-4o mini
     - ``openai/gpt-4o-mini``
     - Fast, cost-effective model
   * - GPT-4o
     - ``openai/gpt-4o``
     - High-capability reasoning model
   * - o3-mini
     - ``openai/o3-mini``
     - Reasoning-focused model (preview)
   * - o3
     - ``openai/o3``
     - Advanced reasoning model (preview)

**Configuration Examples:**

.. code-block:: yaml

    llm_providers:
      # Configure all OpenAI models with wildcard
      - model: openai/*
        access_key: $OPENAI_API_KEY

      # Or configure specific models
      - model: openai/gpt-5.2
        access_key: $OPENAI_API_KEY
        default: true

      - model: openai/gpt-5
        access_key: $OPENAI_API_KEY

      - model: openai/gpt-4o
        access_key: $OPENAI_API_KEY

Anthropic
~~~~~~~~~

**Provider Prefix:** ``anthropic/``

**API Endpoint:** ``/v1/messages``

**Authentication:** API Key - Get your Anthropic API key from `Anthropic Console <https://console.anthropic.com/settings/keys>`_.

**Supported Chat Models:** All Anthropic Claude models including Claude Sonnet 4.5, Claude Opus 4.5, Claude Haiku 4.5, and all future releases.

.. list-table::
   :header-rows: 1
   :widths: 30 20 50

   * - Model Name
     - Model ID for Config
     - Description
   * - Claude Opus 4.5
     - ``anthropic/claude-opus-4-5``
     - Most capable model for complex tasks
   * - Claude Sonnet 4.5
     - ``anthropic/claude-sonnet-4-5``
     - Balanced performance model
   * - Claude Haiku 4.5
     - ``anthropic/claude-haiku-4-5``
     - Fast and efficient model
   * - Claude Sonnet 3.5
     - ``anthropic/claude-sonnet-3-5``
     - Complex agents and coding

**Configuration Examples:**

.. code-block:: yaml

    llm_providers:
      # Configure all Anthropic models with wildcard
      - model: anthropic/*
        access_key: $ANTHROPIC_API_KEY

      # Or configure specific models
      - model: anthropic/claude-opus-4-5
        access_key: $ANTHROPIC_API_KEY

      - model: anthropic/claude-sonnet-4-5
        access_key: $ANTHROPIC_API_KEY

      - model: anthropic/claude-haiku-4-5
        access_key: $ANTHROPIC_API_KEY

      # Override specific model with custom routing
      - model: anthropic/*
        access_key: $ANTHROPIC_API_KEY

      - model: anthropic/claude-sonnet-4-20250514
        access_key: $ANTHROPIC_PROD_API_KEY
        routing_preferences:
          - name: code_generation

DeepSeek
~~~~~~~~

**Provider Prefix:** ``deepseek/``

**API Endpoint:** ``/v1/chat/completions``

**Authentication:** API Key - Get your DeepSeek API key from `DeepSeek Platform <https://platform.deepseek.com/api_keys>`_.

**Supported Chat Models:** All DeepSeek chat models including DeepSeek-Chat, DeepSeek-Coder, and all future releases.

.. list-table::
   :header-rows: 1
   :widths: 30 20 50

   * - Model Name
     - Model ID for Config
     - Description
   * - DeepSeek Chat
     - ``deepseek/deepseek-chat``
     - General purpose chat model
   * - DeepSeek Coder
     - ``deepseek/deepseek-coder``
     - Code-specialized model

**Configuration Examples:**

.. code-block:: yaml

    llm_providers:
      - model: deepseek/deepseek-chat
        access_key: $DEEPSEEK_API_KEY

      - model: deepseek/deepseek-coder
        access_key: $DEEPSEEK_API_KEY

Mistral AI
~~~~~~~~~~

**Provider Prefix:** ``mistral/``

**API Endpoint:** ``/v1/chat/completions``

**Authentication:** API Key - Get your Mistral API key from `Mistral AI Console <https://console.mistral.ai/api-keys/>`_.

**Supported Chat Models:** All Mistral chat models including Mistral Large, Mistral Small, Ministral, and all future releases.

.. list-table::
   :header-rows: 1
   :widths: 30 20 50

   * - Model Name
     - Model ID for Config
     - Description
   * - Mistral Large
     - ``mistral/mistral-large-latest``
     - Most capable model
   * - Mistral Medium
     - ``mistral/mistral-medium-latest``
     - Balanced performance
   * - Mistral Small
     - ``mistral/mistral-small-latest``
     - Fast and efficient
   * - Ministral 3B
     - ``mistral/ministral-3b-latest``
     - Compact model

**Configuration Examples:**
**Configuration Examples:**

.. code-block:: yaml

    llm_providers:
      - model: mistral/mistral-large-latest
        access_key: $MISTRAL_API_KEY

      - model: mistral/mistral-small-latest
        access_key: $MISTRAL_API_KEY

Groq
~~~~

**Provider Prefix:** ``groq/``

**API Endpoint:** ``/openai/v1/chat/completions`` (transformed internally)

**Authentication:** API Key - Get your Groq API key from `Groq Console <https://console.groq.com/keys>`_.

**Supported Chat Models:** All Groq chat models including Llama 4, GPT OSS, Mixtral, Gemma, and all future releases.

.. list-table::
   :header-rows: 1
   :widths: 30 20 50

   * - Model Name
     - Model ID for Config
     - Description
   * - Llama 4 Maverick 17B
     - ``groq/llama-4-maverick-17b-128e-instruct``
     - Fast inference Llama model
   * - Llama 4 Scout 8B
     - ``groq/llama-4-scout-8b-128e-instruct``
     - Smaller Llama model
   * - GPT OSS 20B
     - ``groq/gpt-oss-20b``
     - Open source GPT model

**Configuration Examples:**

.. code-block:: yaml

    llm_providers:
      - model: groq/llama-4-maverick-17b-128e-instruct
        access_key: $GROQ_API_KEY

      - model: groq/llama-4-scout-8b-128e-instruct
        access_key: $GROQ_API_KEY

      - model: groq/gpt-oss-20b
        access_key: $GROQ_API_KEY

Google Gemini
~~~~~~~~~~~~~

**Provider Prefix:** ``gemini/``

**API Endpoint:** ``/v1beta/openai/chat/completions`` (transformed internally)

**Authentication:** API Key - Get your Google AI API key from `Google AI Studio <https://aistudio.google.com/app/apikey>`_.

**Supported Chat Models:** All Google Gemini chat models including Gemini 3 Pro, Gemini 3 Flash, and all future releases.

.. list-table::
   :header-rows: 1
   :widths: 30 20 50

   * - Model Name
     - Model ID for Config
     - Description
   * - Gemini 3 Pro
     - ``gemini/gemini-3-pro``
     - Advanced reasoning and creativity
   * - Gemini 3 Flash
     - ``gemini/gemini-3-flash``
     - Fast and efficient model

**Configuration Examples:**

.. code-block:: yaml

    llm_providers:
      - model: gemini/gemini-3-pro
        access_key: $GOOGLE_API_KEY

      - model: gemini/gemini-3-flash
        access_key: $GOOGLE_API_KEY

Together AI
~~~~~~~~~~~

**Provider Prefix:** ``together_ai/``

**API Endpoint:** ``/v1/chat/completions``

**Authentication:** API Key - Get your Together AI API key from `Together AI Settings <https://api.together.xyz/settings/api-keys>`_.

**Supported Chat Models:** All Together AI chat models including Llama, CodeLlama, Mixtral, Qwen, and hundreds of other open-source models.

.. list-table::
   :header-rows: 1
   :widths: 30 20 50

   * - Model Name
     - Model ID for Config
     - Description
   * - Meta Llama 2 7B
     - ``together_ai/meta-llama/Llama-2-7b-chat-hf``
     - Open source chat model
   * - Meta Llama 2 13B
     - ``together_ai/meta-llama/Llama-2-13b-chat-hf``
     - Larger open source model
   * - Code Llama 34B
     - ``together_ai/codellama/CodeLlama-34b-Instruct-hf``
     - Code-specialized model

**Configuration Examples:**

.. code-block:: yaml

    llm_providers:
      - model: together_ai/meta-llama/Llama-2-7b-chat-hf
        access_key: $TOGETHER_API_KEY

      - model: together_ai/codellama/CodeLlama-34b-Instruct-hf
        access_key: $TOGETHER_API_KEY

xAI
~~~

**Provider Prefix:** ``xai/``

**API Endpoint:** ``/v1/chat/completions``

**Authentication:** API Key - Get your xAI API key from `xAI Console <https://console.x.ai/>`_.

**Supported Chat Models:** All xAI chat models including Grok Beta and all future releases.

.. list-table::
   :header-rows: 1
   :widths: 30 20 50

   * - Model Name
     - Model ID for Config
     - Description
   * - Grok Beta
     - ``xai/grok-beta``
     - Conversational AI model

**Configuration Examples:**

.. code-block:: yaml

    llm_providers:
      - model: xai/grok-beta
        access_key: $XAI_API_KEY

Moonshot AI
~~~~~~~~~~~

**Provider Prefix:** ``moonshotai/``

**API Endpoint:** ``/v1/chat/completions``

**Authentication:** API Key - Get your Moonshot AI API key from `Moonshot AI Platform <https://platform.moonshot.ai/>`_.

**Supported Chat Models:** All Moonshot AI chat models including Kimi K2, Moonshot v1, and all future releases.

.. list-table::
   :header-rows: 1
   :widths: 30 20 50

   * - Model Name
     - Model ID for Config
     - Description
   * - Kimi K2 Preview
     - ``moonshotai/kimi-k2-0905-preview``
     - Foundation model optimized for agentic tasks with 32B activated parameters
   * - Moonshot v1 32K
     - ``moonshotai/moonshot-v1-32k``
     - Extended context model with 32K tokens
   * - Moonshot v1 128K
     - ``moonshotai/moonshot-v1-128k``
     - Long context model with 128K tokens

**Configuration Examples:**

.. code-block:: yaml

    llm_providers:
      # Latest K2 models for agentic tasks
      - model: moonshotai/kimi-k2-0905-preview
        access_key: $MOONSHOTAI_API_KEY

      # V1 models with different context lengths
      - model: moonshotai/moonshot-v1-32k
        access_key: $MOONSHOTAI_API_KEY

      - model: moonshotai/moonshot-v1-128k
        access_key: $MOONSHOTAI_API_KEY


Zhipu AI
~~~~~~~~

**Provider Prefix:** ``zhipu/``

**API Endpoint:** ``/api/paas/v4/chat/completions``

**Authentication:** API Key - Get your Zhipu AI API key from `Zhipu AI Platform <https://open.bigmodel.cn/console/overview/>`_.

**Supported Chat Models:** All Zhipu AI GLM models including GLM-4, GLM-4 Flash, and all future releases.

.. list-table::
   :header-rows: 1
   :widths: 30 20 50

   * - Model Name
     - Model ID for Config
     - Description
   * - GLM-4.6
     - ``zhipu/glm-4.6``
     - Latest and most capable GLM model with enhanced reasoning abilities
   * - GLM-4.5
     - ``zhipu/glm-4.5``
     - High-performance model with multimodal capabilities
   * - GLM-4.5 Air
     - ``zhipu/glm-4.5-air``
     - Lightweight and fast model optimized for efficiency

**Configuration Examples:**

.. code-block:: yaml

    llm_providers:
      # Latest GLM models
      - model: zhipu/glm-4.6
        access_key: $ZHIPU_API_KEY

      - model: zhipu/glm-4.5
        access_key: $ZHIPU_API_KEY

      - model: zhipu/glm-4.5-air
        access_key: $ZHIPU_API_KEY

Providers Requiring Base URL
----------------------------

The following providers require a ``base_url`` parameter to be configured. For detailed information on base URL configuration including path prefix behavior and examples, see :ref:`base_url_details`.

Azure OpenAI
~~~~~~~~~~~~

**Provider Prefix:** ``azure_openai/``

**API Endpoint:** ``/openai/deployments/{deployment-name}/chat/completions`` (constructed automatically)

**Authentication:** API Key + Base URL - Get your Azure OpenAI API key from `Azure Portal <https://portal.azure.com/>`_ → Your OpenAI Resource → Keys and Endpoint.

**Supported Chat Models:** All Azure OpenAI chat models including GPT-4o, GPT-4, GPT-3.5-turbo deployed in your Azure subscription.

.. code-block:: yaml

    llm_providers:
      # Single deployment
      - model: azure_openai/gpt-4o
        access_key: $AZURE_OPENAI_API_KEY
        base_url: https://your-resource.openai.azure.com

      # Multiple deployments
      - model: azure_openai/gpt-4o-mini
        access_key: $AZURE_OPENAI_API_KEY
        base_url: https://your-resource.openai.azure.com

Amazon Bedrock
~~~~~~~~~~~~~~

**Provider Prefix:** ``amazon_bedrock/``

**API Endpoint:** Plano automatically constructs the endpoint as:
  - Non-streaming: ``/model/{model-id}/converse``
  - Streaming: ``/model/{model-id}/converse-stream``

**Authentication:** AWS Bearer Token + Base URL - Get your API Keys from `AWS Bedrock Console <https://console.aws.amazon.com/bedrock/>`_ → Discover → API Keys.

**Supported Chat Models:** All Amazon Bedrock foundation models including Claude (Anthropic), Nova (Amazon), Llama (Meta), Mistral AI, and Cohere Command models.

.. code-block:: yaml

    llm_providers:
      # Amazon Nova models
      - model: amazon_bedrock/us.amazon.nova-premier-v1:0
        access_key: $AWS_BEARER_TOKEN_BEDROCK
        base_url: https://bedrock-runtime.us-west-2.amazonaws.com
        default: true

      - model: amazon_bedrock/us.amazon.nova-pro-v1:0
        access_key: $AWS_BEARER_TOKEN_BEDROCK
        base_url: https://bedrock-runtime.us-west-2.amazonaws.com

      # Claude on Bedrock
      - model: amazon_bedrock/us.anthropic.claude-3-5-sonnet-20241022-v2:0
        access_key: $AWS_BEARER_TOKEN_BEDROCK
        base_url: https://bedrock-runtime.us-west-2.amazonaws.com

Qwen (Alibaba)
~~~~~~~~~~~~~~

**Provider Prefix:** ``qwen/``

**API Endpoint:** ``/v1/chat/completions``

**Authentication:** API Key + Base URL - Get your Qwen API key from `Qwen Portal <https://modelstudio.console.alibabacloud.com/>`_ → Your Qwen Resource → Keys and Endpoint.

**Supported Chat Models:** All Qwen chat models including Qwen3, Qwen3-Coder and all future releases.

.. code-block:: yaml

    llm_providers:
      # Single deployment
      - model: qwen/qwen3
        access_key: $DASHSCOPE_API_KEY
        base_url: https://dashscope.aliyuncs.com

      # Multiple deployments
      - model: qwen/qwen3-coder
        access_key: $DASHSCOPE_API_KEY
        base_url: "https://dashscope-intl.aliyuncs.com"

Ollama
~~~~~~

**Provider Prefix:** ``ollama/``

**API Endpoint:** ``/v1/chat/completions`` (Ollama's OpenAI-compatible endpoint)

**Authentication:** None (Base URL only) - Install Ollama from `Ollama.com <https://ollama.com/>`_ and pull your desired models.

**Supported Chat Models:** All chat models available in your local Ollama installation. Use ``ollama list`` to see installed models.

.. code-block:: yaml

    llm_providers:
      # Local Ollama installation
      - model: ollama/llama3.1
        base_url: http://localhost:11434

      # Ollama in Docker (from host)
      - model: ollama/codellama
        base_url: http://host.docker.internal:11434


OpenAI-Compatible Providers
~~~~~~~~~~~~~~~~~~~~~~~~~~~

**Supported Models:** Any chat models from providers that implement the OpenAI Chat Completions API standard.

For providers that implement the OpenAI API but aren't natively supported:

.. code-block:: yaml

    llm_providers:
      # Generic OpenAI-compatible provider
      - model: custom-provider/custom-model
        base_url: https://api.customprovider.com
        provider_interface: openai
        access_key: $CUSTOM_API_KEY

      # Local deployment
      - model: local/llama2-7b
        base_url: http://localhost:8000
        provider_interface: openai

.. _base_url_details:

Base URL Configuration
----------------------

The ``base_url`` parameter allows you to specify custom endpoints for model providers. It supports both hostname and path components, enabling flexible routing to different API endpoints.

**Format:** ``<scheme>://<hostname>[:<port>][/<path>]``

**Components:**

- ``scheme``: ``http`` or ``https``
- ``hostname``: API server hostname or IP address
- ``port``: Optional, defaults to 80 for http, 443 for https
- ``path``: Optional path prefix that **replaces** the provider's default API path

**How Path Prefixes Work:**

When you include a path in ``base_url``, it replaces the provider's default path prefix while preserving the endpoint suffix:

- **Without path prefix**: Uses the provider's default path structure
- **With path prefix**: Your custom path replaces the provider's default prefix, then the endpoint suffix is appended

**Configuration Examples:**

.. code-block:: yaml

    llm_providers:
      # Simple hostname only - uses provider's default path
      - model: zhipu/glm-4.6
        access_key: $ZHIPU_API_KEY
        base_url: https://api.z.ai
        # Results in: https://api.z.ai/api/paas/v4/chat/completions

      # With custom path prefix - replaces provider's default path
      - model: zhipu/glm-4.6
        access_key: $ZHIPU_API_KEY
        base_url: https://api.z.ai/api/coding/paas/v4
        # Results in: https://api.z.ai/api/coding/paas/v4/chat/completions

      # Azure with custom path
      - model: azure_openai/gpt-4
        access_key: $AZURE_API_KEY
        base_url: https://mycompany.openai.azure.com/custom/deployment/path
        # Results in: https://mycompany.openai.azure.com/custom/deployment/path/chat/completions

      # Behind a proxy or API gateway
      - model: openai/gpt-4o
        access_key: $OPENAI_API_KEY
        base_url: https://proxy.company.com/ai-gateway/openai
        # Results in: https://proxy.company.com/ai-gateway/openai/chat/completions

      # Local endpoint with custom port
      - model: ollama/llama3.1
        base_url: http://localhost:8080
        # Results in: http://localhost:8080/v1/chat/completions

      # Custom provider with path prefix
      - model: vllm/custom-model
        access_key: $VLLM_API_KEY
        base_url: https://vllm.example.com/models/v2
        provider_interface: openai
        # Results in: https://vllm.example.com/models/v2/chat/completions

Advanced Configuration
----------------------

Multiple Provider Instances
~~~~~~~~~~~~~~~~~~~~~~~~~~~

Configure multiple instances of the same provider:

.. code-block:: yaml

    llm_providers:
      # Production OpenAI
      - model: openai/gpt-4o
        access_key: $OPENAI_PROD_KEY
        name: openai-prod

      # Development OpenAI (different key/quota)
      - model: openai/gpt-4o-mini
        access_key: $OPENAI_DEV_KEY
        name: openai-dev

Wildcard Model Configuration
~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Automatically configure all available models from a provider using wildcard patterns. Plano expands wildcards at configuration load time to include all known models from the provider's registry.

**Basic Wildcard Usage:**

.. code-block:: yaml

    llm_providers:
      # Expand to all OpenAI models
      - model: openai/*
        access_key: $OPENAI_API_KEY

      # Expand to all Anthropic Claude models
      - model: anthropic/*
        access_key: $ANTHROPIC_API_KEY

      # Expand to all Mistral models
      - model: mistral/*
        access_key: $MISTRAL_API_KEY

**How Wildcards Work:**

1. **Known Providers** (OpenAI, Anthropic, DeepSeek, Mistral, Groq, Gemini, Together AI, xAI, Moonshot, Zhipu):

   - Expands at config load time to all models in Plano's provider registry
   - Creates entries for both canonical (``openai/gpt-4``) and short names (``gpt-4``)
   - Enables the ``/models/list`` endpoint to list all available models
   - **View complete model list**: `provider_models.yaml <../../includes/provider_models.yaml>`_

2. **Unknown/Custom Providers** (e.g., ``custom-provider/*``):

   - Stores as a wildcard pattern for runtime matching
   - Requires ``base_url`` and ``provider_interface`` configuration
   - Matches model requests dynamically (e.g., ``custom-provider/any-model-name``)
   - Does not appear in ``/models/list`` endpoint

**Overriding Wildcard Models:**

You can configure specific models with custom settings even when using wildcards. Specific configurations take precedence and are excluded from wildcard expansion:

.. code-block:: yaml

    llm_providers:
      # Expand to all Anthropic models
      - model: anthropic/*
        access_key: $ANTHROPIC_API_KEY

      # Override specific model with custom settings
      # This model will NOT be included in the wildcard expansion above
      - model: anthropic/claude-sonnet-4-20250514
        access_key: $ANTHROPIC_PROD_API_KEY
        routing_preferences:
          - name: code_generation
            priority: 1

      # Another specific override
      - model: anthropic/claude-3-haiku-20240307
        access_key: $ANTHROPIC_DEV_API_KEY

**Custom Provider Wildcards:**

For providers not in Plano's registry, wildcards enable dynamic model routing:

.. code-block:: yaml

    llm_providers:
      # Custom LiteLLM deployment
      - model: litellm/*
        base_url: https://litellm.example.com
        provider_interface: openai
        passthrough_auth: true

      # Custom provider with all models
      - model: custom-provider/*
        access_key: $CUSTOM_API_KEY
        base_url: https://api.custom-provider.com
        provider_interface: openai

**Benefits:**

- **Simplified Configuration**: One line instead of listing dozens of models
- **Future-Proof**: Automatically includes new models as they're released
- **Flexible Overrides**: Customize specific models while using wildcards for others
- **Selective Expansion**: Control which models get custom configurations

Default Model Configuration
~~~~~~~~~~~~~~~~~~~~~~~~~~~

Mark one model as the default for fallback scenarios:

.. code-block:: yaml

    llm_providers:
      - model: openai/gpt-4o-mini
        access_key: $OPENAI_API_KEY
        default: true  # Used when no specific model is requested

Routing Preferences
~~~~~~~~~~~~~~~~~~~

Configure routing preferences for dynamic model selection:

.. code-block:: yaml

    llm_providers:
      - model: openai/gpt-5.2
        access_key: $OPENAI_API_KEY
        routing_preferences:
          - name: complex_reasoning
            description: deep analysis, mathematical problem solving, and logical reasoning
          - name: code_review
            description: reviewing and analyzing existing code for bugs and improvements

      - model: anthropic/claude-sonnet-4-5
        access_key: $ANTHROPIC_API_KEY
        routing_preferences:
          - name: creative_writing
            description: creative content generation, storytelling, and writing assistance

.. _passthrough_auth:

Passthrough Authentication
~~~~~~~~~~~~~~~~~~~~~~~~~~

When deploying Plano in front of LLM proxy services that manage their own API key validation (such as LiteLLM, OpenRouter, or custom gateways), you may want to forward the client's original ``Authorization`` header instead of replacing it with a configured ``access_key``.

The ``passthrough_auth`` option enables this behavior:

.. code-block:: yaml

    llm_providers:
      # Forward client's Authorization header to LiteLLM
      - model: openai/gpt-4o-litellm
        base_url: https://litellm.example.com
        passthrough_auth: true
        default: true

      # Forward to OpenRouter
      - model: openai/claude-3-opus
        base_url: https://openrouter.ai/api/v1
        passthrough_auth: true

**How it works:**

1. Client sends a request with ``Authorization: Bearer <virtual-key>``
2. Plano preserves this header instead of replacing it with ``access_key``
3. The upstream service (e.g., LiteLLM) validates the virtual key
4. Response flows back through Plano to the client

**Use Cases:**

- **LiteLLM Integration**: Route requests to LiteLLM which manages virtual keys and rate limits
- **OpenRouter**: Forward requests to OpenRouter with per-user API keys
- **Custom API Gateways**: Integrate with internal gateways that have their own authentication
- **Multi-tenant Deployments**: Allow different clients to use their own credentials

**Important Notes:**

- When ``passthrough_auth: true`` is set, the ``access_key`` field is ignored (a warning is logged if both are configured)
- If the client doesn't provide an ``Authorization`` header, the request is forwarded without authentication (upstream will likely return 401)
- The ``base_url`` is typically required when using ``passthrough_auth``

**Configuration with LiteLLM example:**

.. code-block:: yaml

    # plano_config.yaml
    version: v0.3.0

    listeners:
      - name: llm
        type: model
        port: 10000

    model_providers:
      - model: openai/gpt-4o
        base_url: https://litellm.example.com
        passthrough_auth: true
        default: true

.. code-block:: bash

    # Client request - virtual key is forwarded to upstream
    curl http://localhost:10000/v1/chat/completions \
      -H "Authorization: Bearer sk-litellm-virtual-key-abc123" \
      -H "Content-Type: application/json" \
      -d '{"model": "gpt-4o", "messages": [{"role": "user", "content": "Hello"}]}'

Model Selection Guidelines
--------------------------

**For Production Applications:**
- **High Performance**: OpenAI GPT-5.2, Anthropic Claude Sonnet 4.5
- **Cost-Effective**: OpenAI GPT-5, Anthropic Claude Haiku 4.5
- **Code Tasks**: DeepSeek Coder, Together AI Code Llama
- **Local Deployment**: Ollama with Llama 3.1 or Code Llama

**For Development/Testing:**
- **Fast Iteration**: Groq models (optimized inference)
- **Local Testing**: Ollama models
- **Cost Control**: Smaller models like GPT-4o or Mistral Small

See Also
--------

- :ref:`client_libraries` - Using different client libraries with providers
- :ref:`model_aliases` - Creating semantic model names
- :ref:`llm_router` - Setting up intelligent routing
- :ref:`client_libraries` - Using different client libraries
- :ref:`model_aliases` - Creating semantic model names
