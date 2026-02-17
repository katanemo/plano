.. _prompt_target:

Prompt Target
=============
A Prompt Target is a deterministic, task-specific backend function or API endpoint that your application calls via Plano.
Unlike agents (which handle wide-ranging, open-ended tasks), prompt targets are designed for focused, specific workloads where Plano can add value through input clarification and validation.

Plano helps by:

* **Clarifying and validating input**: Plano enriches incoming prompts with metadata (e.g., detecting follow-ups or clarifying requests) and can extract structured parameters from natural language before passing them to your backend.
* **Enabling high determinism**: Since the task is specific and well-defined, Plano can reliably extract the information your backend needs without ambiguity.
* **Reducing backend work**: Your backend receives clean, validated, structured inputs—so you can focus on business logic instead of parsing and validation.

For example, a prompt target might be "schedule a meeting" (specific task, deterministic inputs like date, time, attendees) or "retrieve documents" (well-defined RAG query with clear intent). Prompt targets are typically called from your application code via Plano's internal listener.


.. table::
    :width: 100%

    ====================    ============================================
    **Capability**          **Description**
    ====================    ============================================
    Intent Recognition      Identify the purpose of a user prompt.
    Parameter Extraction    Extract necessary data from the prompt.
    Invocation              Call relevant backend agents or tools (APIs).
    Response Handling       Process and return responses to the user.
    ====================    ============================================

Key Features
~~~~~~~~~~~~

Below are the key features of prompt targets that empower developers to build efficient, scalable, and personalized GenAI solutions:

- **Design Scenarios**: Define prompt targets to effectively handle specific agentic scenarios.
- **Input Management**: Specify required and optional parameters for each target.
- **Tools Integration**: Seamlessly connect prompts to backend APIs or functions.
- **Error Handling**: Direct errors to designated handlers for streamlined troubleshooting.
- **Multi-Turn Support**: Manage follow-up prompts and clarifications in conversational flows.

Basic Configuration
~~~~~~~~~~~~~~~~~~~
Configuring prompt targets involves defining them in Plano's configuration file. Each Prompt target specifies how a particular type of prompt should be handled, including the endpoint to invoke and any parameters required. A prompt target configuration includes the following elements:

.. vale Vale.Spelling = NO

- ``name``: A unique identifier for the prompt target.
- ``description``: A brief explanation of what the prompt target does.
- ``endpoint``: Required if you want to call a tool or specific API. ``name`` and ``path`` ``http_method`` are the three attributes of the endpoint.
- ``parameters`` (Optional): A list of parameters to extract from the prompt.

.. _defining_prompt_target_parameters:

Defining Parameters
~~~~~~~~~~~~~~~~~~~
Parameters are the pieces of information that Plano needs to extract from the user's prompt to perform the desired action.
Each parameter can be marked as required or optional. Here is a full list of parameter attributes that Plano can support:

.. table::
    :width: 100%

    ========================  ============================================================================
    **Attribute**             **Description**
    ========================  ============================================================================
    ``name (req.)``           Specifies name of the parameter.
    ``description (req.)``    Provides a human-readable explanation of the parameter's purpose.
    ``type (req.)``           Specifies the data type. Supported types include: **int**, **str**, **float**, **bool**, **list**, **set**, **dict**, **tuple**
    ``in_path``               Indicates whether the parameter is part of the path in the endpoint url. Valid values: **true** or **false**
    ``default``               Specifies a default value for the parameter if not provided by the user.
    ``format``                Specifies a format for the parameter value. For example: `2019-12-31` for a date value.
    ``enum``                  Lists of allowable values for the parameter with data type matching the ``type`` attribute. **Usage Example**: ``enum: ["celsius`", "fahrenheit"]``
    ``items``                 Specifies the attribute of the elements when type equals **list**, **set**, **dict**, **tuple**. **Usage Example**: ``items: {"type": "str"}``
    ``required``              Indicates whether the parameter is mandatory or optional. Valid values: **true** or **false**
    ========================  ============================================================================

Example Configuration For Tools
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: yaml
    :caption: Tools and Function Calling Configuration Example

    prompt_targets:
      - name: get_weather
        description: Get the current weather for a location
        parameters:
          - name: location
            description: The city and state, e.g. San Francisco, New York
            type: str
            required: true
          - name: unit
            description: The unit of temperature
            type: str
            default: fahrenheit
            enum: [celsius, fahrenheit]
        endpoint:
          name: api_server
          path: /weather

.. _plano_multi_turn_guide:

Multi-Turn
~~~~~~~~~~
Developers often `struggle <https://www.reddit.com/r/LocalLLaMA/comments/18mqwg6/best_practice_for_rag_with_followup_chat/>`_ to efficiently handle
``follow-up`` or ``clarification`` questions. Specifically, when users ask for changes or additions to previous responses, it requires developers to
re-write prompts using LLMs with precise prompt engineering techniques. This process is slow, manual, error prone and adds latency and token cost for
common scenarios that can be managed more efficiently.

Plano is highly capable of accurately detecting and processing prompts in multi-turn scenarios so that you can buil fast and accurate agents in minutes.
Below are some cnversational examples that you can build via Plano. Each example is enriched with annotations (via ** [Plano] ** ) that illustrates how Plano
processess conversational messages on your behalf.

Example 1: Adjusting Retrieval

.. code-block:: text

    User: What are the benefits of renewable energy?
    **[Plano]**: Check if there is an available <prompt_target> that can handle this user query.
    **[Plano]**: Found "get_info_for_energy_source" prompt_target in plano_config.yaml. Forward prompt to the endpoint configured in "get_info_for_energy_source"
    ...
    Assistant: Renewable energy reduces greenhouse gas emissions, lowers air pollution, and provides sustainable power sources like solar and wind.

    User: Include cost considerations in the response.
    **[Plano]**: Follow-up detected. Forward prompt history to the "get_info_for_energy_source" prompt_target and post the following parameters consideration="cost"
    ...
    Assistant: Renewable energy reduces greenhouse gas emissions, lowers air pollution, and provides sustainable power sources like solar and wind. While the initial setup costs can be high, long-term savings from reduced fuel expenses and government incentives make it cost-effective.


Example 2: Switching Intent
---------------------------
.. code-block:: text

    User: What are the symptoms of diabetes?
    **[Plano]**: Check if there is an available <prompt_target> that can handle this user query.
    **[Plano]**: Found "diseases_symptoms" prompt_target in plano_config.yaml. Forward disease=diabeteres to "diseases_symptoms" prompt target
    ...
    Assistant: Common symptoms include frequent urination, excessive thirst, fatigue, and blurry vision.

    User: How is it diagnosed?
    **[Plano]**: New intent detected.
    **[Plano]**: Found "disease_diagnoses" prompt_target in plano_config.yaml. Forward disease=diabeteres to "disease_diagnoses" prompt target
    ...
    Assistant: Diabetes is diagnosed through blood tests like fasting blood sugar, A1C, or an oral glucose tolerance test.


Build Multi-Turn RAG Apps
-------------------------
The following section describes how you can easilly add support for multi-turn scenarios via Plano. You process and manage multi-turn prompts
just like you manage single-turn ones. Plano handles the conpleixity of detecting the correct intent based on the last user prompt and
the covnersational history, extracts relevant parameters needed by downstream APIs, and dipatches calls to any upstream LLMs to summarize the
response from your APIs.


.. _multi_turn_subsection_prompt_target:

Step 1: Define Plano Config
---------------------------

.. literalinclude:: ../build_with_plano/includes/multi_turn/prompt_targets_multi_turn.yaml
    :language: yaml
    :caption: Plano Config
    :linenos:

Step 2: Process Request in Flask
--------------------------------

Once the prompt targets are configured as above, handle parameters across multi-turn as if its a single-turn request

.. literalinclude:: ../build_with_plano/includes/multi_turn/multi_turn_rag.py
    :language: python
    :caption: Parameter handling with Flask
    :linenos:

Demo App
--------

For your convenience, we've built a `demo app <https://github.com/katanemo/plano/tree/main/demos/advanced/multi_turn_rag>`_
that you can test and modify locally for multi-turn RAG scenarios.

.. figure:: ../build_with_plano/includes/multi_turn/mutli-turn-example.png
   :width: 100%
   :align: center

   Example multi-turn user conversation showing adjusting retrieval

Summary
~~~~~~~
By carefully designing prompt targets as deterministic, task-specific entry points, you ensure that prompts are routed to the right workload, necessary parameters are cleanly extracted and validated, and backend services are invoked with structured inputs. This clear separation between prompt handling and business logic simplifies your architecture, makes behavior more predictable and testable, and improves the scalability and maintainability of your agentic applications.
