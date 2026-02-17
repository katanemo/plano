.. _agent_routing:

Orchestration
==============

Building multi-agent systems allow you to route requests across multiple specialized agents, each designed to handle specific types of tasks.
Plano makes it easy to build and scale these systems by managing the orchestration layer—deciding which agent(s) should handle each request—while you focus on implementing individual agent logic.

This guide shows you how to configure and implement multi-agent orchestration in Plano using a real-world example: a **Travel Booking Assistant** that routes queries to specialized agents for weather and flights.

How It Works
------------

Plano's orchestration layer analyzes incoming prompts and routes them to the most appropriate agent based on user intent and conversation context. The workflow is:

1. **User submits a prompt**: The request arrives at Plano's agent listener.
2. **Agent selection**: Plano uses an LLM to analyze the prompt and determine user intent and complexity. By default, this uses `Plano-Orchestrator-30B-A3B <https://huggingface.co/collections/katanemo/plano-orchestrator>`_, which offers performance of foundation models at 1/10th the cost. The LLM routes the request to the most suitable agent configured in your system—such as a weather agent or flight agent.
3. **Agent handles request**: Once the selected agent receives the request object from Plano, it manages its own :ref:`inner loop <agents>` until the task is complete. This means the agent autonomously calls models, invokes tools, processes data, and reasons about next steps—all within its specialized domain—before returning the final response.
4. **Seamless handoffs**: For multi-turn conversations, Plano repeats the intent analysis for each follow-up query, enabling smooth handoffs between agents as the conversation evolves.

Example: Travel Booking Assistant
----------------------------------

Let's walk through a complete multi-agent system: a Travel Booking Assistant that helps users plan trips by providing weather forecasts and flight information. This system uses two specialized agents:

* **Weather Agent**: Provides real-time weather conditions and multi-day forecasts
* **Flight Agent**: Searches for flights between airports with real-time tracking

Configuration
-------------

Configure your agents in the ``listeners`` section of your ``plano_config.yaml``:

.. literalinclude:: ../resources/includes/agents/agents_config.yaml
    :language: yaml
    :linenos:
    :caption: Travel Booking Multi-Agent Configuration

**Key Configuration Elements:**

* **agent listener**: A listener of ``type: agent`` tells Plano to perform intent analysis and routing for incoming requests.
* **agents list**: Define each agent with an ``id``, ``description`` (used for routing decisions)
* **router**: The ``plano_orchestrator_v1`` router uses Plano-Orchestrator to analyze user intent and select the appropriate agent.
* **filter_chain**: Optionally attach :ref:`filter chains <filter_chain>` to agents for guardrails, query rewriting, or context enrichment.

**Writing Effective Agent Descriptions**

Agent descriptions are critical—they're used by Plano-Orchestrator to make routing decisions. Effective descriptions should include:

* **Clear introduction**: A concise statement explaining what the agent is and its primary purpose
* **Capabilities section**: A bulleted list of specific capabilities, including:

  * What APIs or data sources it uses (e.g., "Open-Meteo API", "FlightAware AeroAPI")
  * What information it provides (e.g., "current temperature", "multi-day forecasts", "gate information")
  * How it handles context (e.g., "Understands conversation context to resolve location references")
  * What question patterns it handles (e.g., "What's the weather in [city]?")
  * How it handles multi-part queries (e.g., "When queries include both weather and flights, this agent answers ONLY the weather part")

Here's an example of a well-structured agent description:

.. code-block:: yaml

    - id: weather_agent
      description: |

        WeatherAgent is a specialized AI assistant for real-time weather information
        and forecasts. It provides accurate weather data for any city worldwide using
        the Open-Meteo API, helping travelers plan their trips with up-to-date weather
        conditions.

        Capabilities:
          * Get real-time weather conditions and multi-day forecasts for any city worldwide
          * Provides current temperature, weather conditions, sunrise/sunset times
          * Provides detailed weather information including multi-day forecasts
          * Understands conversation context to resolve location references from previous messages
          * Handles weather-related questions including "What's the weather in [city]?"
          * When queries include both weather and other travel questions (e.g., flights),
            this agent answers ONLY the weather part

.. note::
   We will soon support "Agents as Tools" via Model Context Protocol (MCP), enabling agents to dynamically discover and invoke other agents as tools. Track progress on `GitHub Issue #646 <https://github.com/katanemo/archgw/issues/646>`_.

Implementation
--------------

Agents are HTTP services that receive routed requests from Plano. Each agent implements the OpenAI Chat Completions API format, making them compatible with standard LLM clients.

Agent Structure
^^^^^^^^^^^^^^^

Let's examine the Weather Agent implementation:

.. literalinclude:: ../resources/includes/agents/weather.py
    :language: python
    :linenos:
    :lines: 262-283
    :caption: Weather Agent - Core Structure

**Key Points:**

* Agents expose a ``/v1/chat/completions`` endpoint that matches OpenAI's API format
* They use Plano's LLM gateway (via ``LLM_GATEWAY_ENDPOINT``) for all LLM calls
* They receive the full conversation history in ``request_body.messages``

Information Extraction with LLMs
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

Agents use LLMs to extract structured information from natural language queries. This enables them to understand user intent and extract parameters needed for API calls.

The Weather Agent extracts location information:

.. literalinclude:: ../resources/includes/agents/weather.py
    :language: python
    :linenos:
    :lines: 73-119
    :caption: Weather Agent - Location Extraction

The Flight Agent extracts more complex information—origin, destination, and dates:

.. literalinclude:: ../resources/includes/agents/flights.py
    :language: python
    :linenos:
    :lines: 69-120
    :caption: Flight Agent - Flight Information Extraction

**Key Points:**

* Use smaller, faster models (like ``gpt-4o-mini``) for extraction tasks
* Include conversation context to handle follow-up questions and pronouns
* Use structured prompts with clear output formats (JSON)
* Handle edge cases with fallback values

Calling External APIs
^^^^^^^^^^^^^^^^^^^^^^

After extracting information, agents call external APIs to fetch real-time data:

.. literalinclude:: ../resources/includes/agents/weather.py
    :language: python
    :linenos:
    :lines: 136-197
    :caption: Weather Agent - External API Call

The Flight Agent calls FlightAware's AeroAPI:

.. literalinclude:: ../resources/includes/agents/flights.py
    :language: python
    :linenos:
    :lines: 156-260
    :caption: Flight Agent - External API Call

**Key Points:**

* Use async HTTP clients (like ``httpx.AsyncClient``) for non-blocking API calls
* Transform external API responses into consistent, structured formats
* Handle errors gracefully with fallback values
* Cache or validate data when appropriate (e.g., airport code validation)

Preparing Context and Generating Responses
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

Agents combine extracted information, API data, and conversation history to generate responses:

.. literalinclude:: ../resources/includes/agents/weather.py
    :language: python
    :linenos:
    :lines: 290-370
    :caption: Weather Agent - Context Preparation and Response Generation

**Key Points:**

* Use system messages to provide structured data to the LLM
* Include full conversation history for context-aware responses
* Stream responses for better user experience
* Route all LLM calls through Plano's gateway for consistent behavior and observability

Best Practices
--------------

**Write Clear Agent Descriptions**

Agent descriptions are used by Plano-Orchestrator to make routing decisions. Be specific about what each agent handles:

.. code-block:: yaml

    # Good - specific and actionable
    - id: flight_agent
      description: Get live flight information between airports using FlightAware AeroAPI. Shows real-time flight status, scheduled/estimated/actual departure and arrival times, gate and terminal information, delays, aircraft type, and flight status. Automatically resolves city names to airport codes (IATA/ICAO). Understands conversation context to infer origin/destination from follow-up questions.

    # Less ideal - too vague
    - id: flight_agent
      description: Handles flight queries

**Use Conversation Context Effectively**

Include conversation history in your extraction and response generation:

.. code-block:: python

    # Include conversation context for extraction
    conversation_context = []
    for msg in messages:
        conversation_context.append({"role": msg.role, "content": msg.content})

    # Use recent context (last 10 messages)
    context_messages = conversation_context[-10:] if len(conversation_context) > 10 else conversation_context

**Route LLM Calls Through Plano's Model Proxy**

Always route LLM calls through Plano's :ref:`Model Proxy <llm_providers>` for consistent responses, smart routing, and rich observability:

.. code-block:: python

    openai_client_via_plano = AsyncOpenAI(
        base_url=LLM_GATEWAY_ENDPOINT,  # Plano's LLM gateway
        api_key="EMPTY",
    )

    response = await openai_client_via_plano.chat.completions.create(
        model="openai/gpt-4o",
        messages=messages,
        stream=True,
    )

**Handle Errors Gracefully**

Provide fallback values and clear error messages:

.. code-block:: python

    async def get_weather_data(request: Request, messages: list, days: int = 1):
        try:
            # ... extraction and API logic ...
            location = response.choices[0].message.content.strip().strip("\"'`.,!?")
            if not location or location.upper() == "NOT_FOUND":
                location = "New York"  # Fallback to default
            return weather_data
        except Exception as e:
            logger.error(f"Error getting weather data: {e}")
            return {"location": "New York", "weather": {"error": "Could not retrieve weather data"}}

**Use Appropriate Models for Tasks**

Use smaller, faster models for extraction tasks and larger models for final responses:

.. code-block:: python

    # Extraction: Use smaller, faster model
    LOCATION_MODEL = "openai/gpt-4o-mini"

    # Final response: Use larger, more capable model
    WEATHER_MODEL = "openai/gpt-4o"

**Stream Responses**

Stream responses for better user experience:

.. code-block:: python

    async def invoke_weather_agent(request: Request, request_body: dict, traceparent_header: str = None):
        # ... prepare messages with weather data ...

        stream = await openai_client_via_plano.chat.completions.create(
            model=WEATHER_MODEL,
            messages=response_messages,
            temperature=request_body.get("temperature", 0.7),
            max_tokens=request_body.get("max_tokens", 1000),
            stream=True,
            extra_headers=extra_headers,
        )

        async for chunk in stream:
            if chunk.choices:
                yield f"data: {chunk.model_dump_json()}\n\n"

        yield "data: [DONE]\n\n"

Common Use Cases
----------------

Multi-agent orchestration is particularly powerful for:

**Travel and Booking Systems**

Route queries to specialized agents for weather and flights:

.. code-block:: yaml

    agents:
      - id: weather_agent
        description: Get real-time weather conditions and forecasts
      - id: flight_agent
        description: Search for flights and provide flight status

**Customer Support**

Route common queries to automated support agents while escalating complex issues:

.. code-block:: yaml

    agents:
      - id: tier1_support
        description: Handles common FAQs, password resets, and basic troubleshooting
      - id: tier2_support
        description: Handles complex technical issues requiring deep product knowledge
      - id: human_escalation
        description: Escalates sensitive issues or unresolved problems to human agents

**Sales and Marketing**

Direct leads and inquiries to specialized sales agents:

.. code-block:: yaml

    agents:
      - id: product_recommendation
        description: Recommends products based on user needs and preferences
      - id: pricing_agent
        description: Provides pricing information and quotes
      - id: sales_closer
        description: Handles final negotiations and closes deals

**Technical Documentation and Support**

Combine RAG agents for documentation lookup with specialized troubleshooting agents:

.. code-block:: yaml

    agents:
      - id: docs_agent
        description: Retrieves relevant documentation and guides
        filter_chain:
          - query_rewriter
          - context_builder
      - id: troubleshoot_agent
        description: Diagnoses and resolves technical issues step by step

Next Steps
----------

* Learn more about :ref:`agents <agents>` and the inner vs. outer loop model
* Explore :ref:`filter chains <filter_chain>` for adding guardrails and context enrichment
* See :ref:`observability <observability>` for monitoring multi-agent workflows
* Review the :ref:`LLM Providers <llm_providers>` guide for model routing within agents
* Check out the complete `Travel Booking demo <https://github.com/katanemo/plano/tree/main/demos/agent_orchestration/travel_agents>`_ on GitHub

.. note::
    To observe traffic to and from agents, please read more about :ref:`observability <observability>` in Plano.

By carefully configuring and managing your Agent routing and hand off, you can significantly improve your application's responsiveness, performance, and overall user satisfaction.
