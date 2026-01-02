.. _quickstart:

Quickstart
==========

Follow this guide to learn how to quickly set up Plano and integrate it into your generative AI applications. You can:

- :ref:`Use Plano as a model proxy (Gateway) <llm_routing_quickstart>` to standardize access to multiple LLM providers.
- :ref:`Build agents <quickstart_agents>` for multi-step workflows (e.g., travel assistants with flights and hotels).
- :ref:`Call deterministic APIs via prompt targets <quickstart_prompt_targets>` to turn instructions directly into function calls.

.. note::
  This quickstart assumes basic familiarity with agents and prompt targets from the Concepts section. For background, see :ref:`Agents <agents>` and :ref:`Prompt Target <prompt_target>`.

  The full agent and backend API implementations used here are available in the `plano-quickstart repository <https://github.com/plano-ai/plano-quickstart>`_. This guide focuses on wiring and configuring Plano (orchestration, prompt targets, and the model proxy), not application code.

Prerequisites
-------------

Before you begin, ensure you have the following:

1. `Docker System <https://docs.docker.com/get-started/get-docker/>`_ (v24)
2. `Docker Compose <https://docs.docker.com/compose/install/>`_ (v2.29)
3. `Python <https://www.python.org/downloads/>`_ (v3.10+)

Plano's CLI allows you to manage and interact with the Plano efficiently. To install the CLI, simply run the following command:

.. tip::

   We recommend using **uv** for fast, reliable Python package management. Install uv if you haven't already:

   .. code-block:: console

      $ curl -LsSf https://astral.sh/uv/install.sh | sh

**Option 1: Install planoai with uv (Recommended)**

.. code-block:: console

   $ uv tool install planoai==0.4.1

**Option 2: Install with pip (Traditional)**

.. code-block:: console

   $ python -m venv venv
   $ source venv/bin/activate   # On Windows, use: venv\Scripts\activate
   $ pip install planoai==0.4.1


.. _llm_routing_quickstart:

Use Plano as a Model Proxy (Gateway)
------------------------------------

Step 1. Create plano config file
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Plano operates based on a configuration file where you can define LLM providers, prompt targets, guardrails, etc. Below is an example configuration that defines OpenAI and Anthropic LLM providers.

Create ``plano_config.yaml`` file with the following content:

.. code-block:: yaml

  version: v0.3.0

  listeners:
    - type: model
      name: model_1
      address: 0.0.0.0
      port: 12000

  model_providers:

    - access_key: $OPENAI_API_KEY
      model: openai/gpt-4o
      default: true

    - access_key: $ANTHROPIC_API_KEY
      model: anthropic/claude-sonnet-4-5

Step 2. Start plano
~~~~~~~~~~~~~~~~~~~

Once the config file is created, ensure that you have environment variables set up for ``ANTHROPIC_API_KEY`` and ``OPENAI_API_KEY`` (or these are defined in a ``.env`` file).

Start Plano:

.. code-block:: console

   $ planoai up plano_config.yaml
   # Or if installed with uv tool: uvx planoai up plano_config.yaml
   2024-12-05 11:24:51,288 - planoai.main - INFO - Starting plano cli version: 0.4.1
   2024-12-05 11:24:51,825 - planoai.utils - INFO - Schema validation successful!
   2024-12-05 11:24:51,825 - planoai.main - INFO - Starting plano
   ...
   2024-12-05 11:25:16,131 - planoai.core - INFO - Container is healthy!

Step 3: Interact with LLM
~~~~~~~~~~~~~~~~~~~~~~~~~

Step 3.1: Using curl command
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

.. code-block:: bash

   $ curl --header 'Content-Type: application/json' \
     --data '{"messages": [{"role": "user","content": "What is the capital of France?"}], "model": "none"}' \
     http://localhost:12000/v1/chat/completions

   {
     ...
     "model": "gpt-4o-2024-08-06",
     "choices": [
       {
         ...
         "messages": {
           "role": "assistant",
           "content": "The capital of France is Paris.",
         },
       }
     ],
   }

.. note::
   When the requested model is not found in the configuration, Plano will randomly select an available model from the configured providers. In this example, we use ``"model": "none"`` and Plano selects the default model ``openai/gpt-4o``.

Step 3.2: Using OpenAI Python client
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

Make outbound calls via the Plano gateway:

.. code-block:: python

   from openai import OpenAI

   # Use the OpenAI client as usual
   client = OpenAI(
     # No need to set a specific openai.api_key since it's configured in Plano's gateway
     api_key='--',
     # Set the OpenAI API base URL to the Plano gateway endpoint
     base_url="http://127.0.0.1:12000/v1"
   )

   response = client.chat.completions.create(
       # we select model from plano_config file
       model="--",
       messages=[{"role": "user", "content": "What is the capital of France?"}],
   )

   print("OpenAI Response:", response.choices[0].message.content)


Build Agentic Apps with Plano
-----------------------------

Plano helps you build agentic applications in two complementary ways:

* **Orchestrate agents**: Let Plano decide which agent or LLM should handle each request and in what sequence.
* **Call deterministic backends**: Use prompt targets to turn natural-language prompts into structured, validated API calls.

.. _quickstart_agents:

Building agents with Plano orchestration
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Agents are where your business logic lives (the "inner loop"). Plano takes care of the "outer loop"—routing, sequencing, and managing calls across agents and LLMs.

At a high level, building agents with Plano looks like this:

1. **Implement your agent** in your framework of choice (Python, JS/TS, etc.), exposing it as an HTTP service.
2. **Route LLM calls through Plano's Model Proxy**, so all models share a consistent interface and observability.
3. **Configure Plano to orchestrate**: define which agent(s) can handle which kinds of prompts, and let Plano decide when to call an agent vs. an LLM.

This quickstart uses a simplified version of the Travel Booking Assistant; for the full multi-agent walkthrough, see :ref:`Orchestration <agent_routing>`.

Step 1. Minimal orchestration config
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

Here is a minimal configuration that wires Plano-Orchestrator to two HTTP services: one for flights and one for hotels.

.. code-block:: yaml

  version: v0.1.0

  agents:
    - id: flight_agent
      url: http://host.docker.internal:10520  # your flights service
    - id: hotel_agent
      url: http://host.docker.internal:10530  # your hotels service

  model_providers:
    - model: openai/gpt-4o
      access_key: $OPENAI_API_KEY

  listeners:
    - type: agent
      name: travel_assistant
      port: 8001
      router: plano_orchestrator_v1
      agents:
        - id: flight_agent
          description: Search for flights and provide flight status.
        - id: hotel_agent
          description: Find hotels and check availability.

  tracing:
    random_sampling: 100

Step 2. Start your agents and Plano
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

Run your ``flight_agent`` and ``hotel_agent`` services (see :ref:`Orchestration <agent_routing>` for a full Travel Booking example), then start Plano with the config above:

.. code-block:: console

  $ planoai up plano_config.yaml
  # Or if installed with uv tool:
  $ uvx planoai up plano_config.yaml

Plano will start the orchestrator and expose an agent listener on port ``8001``.

Step 3. Send a prompt and let Plano route
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

Now send a request to Plano using the OpenAI-compatible chat completions API—the orchestrator will analyze the prompt and route it to the right agent based on intent:

.. code-block:: bash

  $ curl --header 'Content-Type: application/json' \
    --data '{"messages": [{"role": "user","content": "Find me flights from SFO to JFK tomorrow"}], "model": "openai/gpt-4o"}' \
    http://localhost:8001/v1/chat/completions

You can then ask a follow-up like "Also book me a hotel near JFK" and Plano-Orchestrator will route to ``hotel_agent``—your agents stay focused on business logic while Plano handles routing.

.. _quickstart_prompt_targets:

Deterministic API calls with prompt targets
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Next, we'll show Plano's deterministic API calling using a single prompt target. We'll build a currency exchange backend powered by `https://api.frankfurter.dev/`, assuming USD as the base currency.

Step 1. Create plano config file
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

Create ``plano_config.yaml`` file with the following content:

.. code-block:: yaml

  version: v0.1.0

  listeners:
    ingress_traffic:
      address: 0.0.0.0
      port: 10000
      message_format: openai
      timeout: 30s

   model_providers:
     - access_key: $OPENAI_API_KEY
       model: openai/gpt-4o

   system_prompt: |
     You are a helpful assistant.

   prompt_targets:
     - name: currency_exchange
       description: Get currency exchange rate from USD to other currencies
       parameters:
         - name: currency_symbol
           description: the currency that needs conversion
           required: true
           type: str
           in_path: true
       endpoint:
         name: frankfurther_api
         path: /v1/latest?base=USD&symbols={currency_symbol}
       system_prompt: |
         You are a helpful assistant. Show me the currency symbol you want to convert from USD.

     - name: get_supported_currencies
       description: Get list of supported currencies for conversion
       endpoint:
         name: frankfurther_api
         path: /v1/currencies

   endpoints:
     frankfurther_api:
       endpoint: api.frankfurter.dev:443
       protocol: https

Step 2. Start plano with currency conversion config
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

.. code-block:: sh

   $ planoai up plano_config.yaml
   # Or if installed with uv tool: uvx planoai up plano_config.yaml
   2024-12-05 16:56:27,979 - planoai.main - INFO - Starting plano cli version: 0.1.5
   ...
   2024-12-05 16:56:28,485 - planoai.utils - INFO - Schema validation successful!
   2024-12-05 16:56:28,485 - planoai.main - INFO - Starting plano model server and plano gateway
   ...
   2024-12-05 16:56:51,647 - planoai.core - INFO - Container is healthy!

Once the gateway is up, you can start interacting with it at port 10000 using the OpenAI chat completion API.

Some sample queries you can ask include: ``what is currency rate for gbp?`` or ``show me list of currencies for conversion``.

Step 3. Interacting with gateway using curl command
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

Here is a sample curl command you can use to interact:

.. code-block:: bash

   $ curl --header 'Content-Type: application/json' \
     --data '{"messages": [{"role": "user","content": "what is exchange rate for gbp"}], "model": "none"}' \
     http://localhost:10000/v1/chat/completions | jq ".choices[0].message.content"

   "As of the date provided in your context, December 5, 2024, the exchange rate for GBP (British Pound) from USD (United States Dollar) is 0.78558. This means that 1 USD is equivalent to 0.78558 GBP."

And to get the list of supported currencies:

.. code-block:: bash

   $ curl --header 'Content-Type: application/json' \
     --data '{"messages": [{"role": "user","content": "show me list of currencies that are supported for conversion"}], "model": "none"}' \
     http://localhost:10000/v1/chat/completions | jq ".choices[0].message.content"

   "Here is a list of the currencies that are supported for conversion from USD, along with their symbols:\n\n1. AUD - Australian Dollar\n2. BGN - Bulgarian Lev\n3. BRL - Brazilian Real\n4. CAD - Canadian Dollar\n5. CHF - Swiss Franc\n6. CNY - Chinese Renminbi Yuan\n7. CZK - Czech Koruna\n8. DKK - Danish Krone\n9. EUR - Euro\n10. GBP - British Pound\n11. HKD - Hong Kong Dollar\n12. HUF - Hungarian Forint\n13. IDR - Indonesian Rupiah\n14. ILS - Israeli New Sheqel\n15. INR - Indian Rupee\n16. ISK - Icelandic Króna\n17. JPY - Japanese Yen\n18. KRW - South Korean Won\n19. MXN - Mexican Peso\n20. MYR - Malaysian Ringgit\n21. NOK - Norwegian Krone\n22. NZD - New Zealand Dollar\n23. PHP - Philippine Peso\n24. PLN - Polish Złoty\n25. RON - Romanian Leu\n26. SEK - Swedish Krona\n27. SGD - Singapore Dollar\n28. THB - Thai Baht\n29. TRY - Turkish Lira\n30. USD - United States Dollar\n31. ZAR - South African Rand\n\nIf you want to convert USD to any of these currencies, you can select the one you are interested in."


Next Steps
==========

Congratulations! You've successfully set up Plano and made your first prompt-based request. To further enhance your GenAI applications, explore the following resources:

- :ref:`Full Documentation <overview>`: Comprehensive guides and references.
- `GitHub Repository <https://github.com/katanemo/plano>`_: Access the source code, contribute, and track updates.
- `Support <https://github.com/katanemo/plano#contact>`_: Get help and connect with the Plano community .

With Plano, building scalable, fast, and personalized GenAI applications has never been easier. Dive deeper into Plano's capabilities and start creating innovative AI-driven experiences today!
