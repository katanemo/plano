.. _function_calling:

Function Calling
================

**Function Calling** is a powerful feature in Plano that allows your application to dynamically execute backend functions or services based on user prompts.
This enables seamless integration between natural language interactions and backend operations, turning user inputs into actionable results.

.. deprecated:: v0.4.22
   The prompt-target based workflow shown below (see :ref:`Step 2 <function_calling>`)
   is deprecated. :ref:`Prompt Targets <prompt_target>` are no longer actively
   maintained and may be removed in a future release. For new function-calling
   workloads, prefer :ref:`Agents <agents>` with tool definitions.


What is Function Calling?
-------------------------

Function Calling refers to the mechanism where the user's prompt is parsed, relevant parameters are extracted, and a designated backend function (or API) is triggered to execute a particular task.
This feature bridges the gap between generative AI systems and functional business logic, allowing users to interact with the system through natural language while the backend performs the necessary operations.

Function Calling Workflow
-------------------------

#. **Prompt Parsing**

    When a user submits a prompt, Plano analyzes it to determine the intent. Based on this intent, the system identifies whether a function needs to be invoked and which parameters should be extracted.

#. **Parameter Extraction**

    Plano’s advanced natural language processing capabilities automatically extract parameters from the prompt that are necessary for executing the function. These parameters can include text, numbers, dates, locations, or other relevant data points.

#. **Function Invocation**

    Once the necessary parameters have been extracted, Plano invokes the relevant backend function. This function could be an API, a database query, or any other form of backend logic. The function is executed with the extracted parameters to produce the desired output.

#. **Response Handling**

    After the function has been called and executed, the result is processed and a response is generated. This response is typically delivered in a user-friendly format, which can include text explanations, data summaries, or even a confirmation message for critical actions.


Arch-Function
-------------
The `Arch-Function <https://huggingface.co/collections/katanemo/arch-function-66f209a693ea8df14317ad68>`_ collection of large language models (LLMs) is a collection state-of-the-art (SOTA) LLMs specifically designed for **function calling** tasks.
The models are designed to understand complex function signatures, identify required parameters, and produce accurate function call outputs based on natural language prompts.
Achieving performance on par with GPT-4, these models set a new benchmark in the domain of function-oriented tasks, making them suitable for scenarios where automated API interaction and function execution is crucial.

In summary, the Arch-Function collection demonstrates:

- **State-of-the-art performance** in function calling
- **Accurate parameter identification and suggestion**, even in ambiguous or incomplete inputs
- **High generalization** across multiple function calling use cases, from API interactions to automated backend tasks.
- Optimized **low-latency, high-throughput performance**, making it suitable for real-time, production environments.


Key Features
~~~~~~~~~~~~
.. table::
    :width: 100%

    =========================   ===============================================================
    **Functionality**	        **Definition**
    =========================   ===============================================================
    Single Function Calling	    Call only one function per user prompt
    Parallel Function Calling	Call the same function multiple times but with parameter values
    Multiple Function Calling	Call different functions per user prompt
    Parallel & Multiple	        Perform both parallel and multiple function calling
    =========================   ===============================================================

Implementing Function Calling
-----------------------------

Here’s a step-by-step guide to configuring function calling within your Plano setup:

Step 1: Define the Function
~~~~~~~~~~~~~~~~~~~~~~~~~~~
First, create or identify the backend function you want Plano to call. This could be an API endpoint, a script, or any other executable backend logic.

.. code-block:: python

    import requests

    def get_weather(location: str, unit: str = "fahrenheit"):
        if unit not in ["celsius", "fahrenheit"]:
            raise ValueError("Invalid unit. Choose either 'celsius' or 'fahrenheit'.")

        api_server = "https://api.yourweatherapp.com"
        endpoint = f"{api_server}/weather"

        params = {
            "location": location,
            "unit": unit
        }

        response = requests.get(endpoint, params=params)
        return response.json()

    # Example usage
    weather_info = get_weather("Seattle, WA", "celsius")
    print(weather_info)


Step 2: Configure Prompt Targets
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
Next, map the function to a prompt target, defining the intent and parameters that Plano will extract from the user’s prompt.
Specify the parameters your function needs and how Plano should interpret these.

.. code-block:: yaml
    :caption: Prompt Target Example Configuration

    prompt_targets:
      - name: get_weather
        description: Get the current weather for a location
        parameters:
          - name: location
            description: The city and state, e.g. San Francisco, New York
            type: str
            required: true
          - name: unit
            description: The unit of temperature to return
            type: str
            enum: ["celsius", "fahrenheit"]
        endpoint:
          name: api_server
          path: /weather

.. Note::
    For a complete refernce of attributes that you can configure in a prompt target, see :ref:`here <defining_prompt_target_parameters>`.

Step 3: Plano Takes Over
~~~~~~~~~~~~~~~~~~~~~~~~
Once you have defined the functions and configured the prompt targets, Plano takes care of the remaining work.
It will automatically validate parameters, and ensure that the required parameters (e.g., location) are present in the prompt, and add validation rules if necessary.

.. figure:: /_static/img/plano_network_diagram_high_level.png
   :width: 100%
   :align: center

   High-level network flow of where Plano sits in your agentic stack. Managing incoming and outgoing prompt traffic


Once a downstream function (API) is called, Plano  takes the response and sends it an upstream LLM to complete the request (for summarization, Q/A, text generation tasks).
For more details on how Plano  enables you to centralize usage of LLMs, please read :ref:`LLM providers <llm_providers>`.

By completing these steps, you enable Plano to manage the process from validation to response, ensuring users receive consistent, reliable results - and that you are focused
on the stuff that matters most.

Example Use Cases
-----------------

Here are some common use cases where Function Calling can be highly beneficial:

- **Data Retrieval**: Extracting information from databases or APIs based on user inputs (e.g., checking account balances, retrieving order status).
- **Transactional Operations**: Executing business logic such as placing an order, processing payments, or updating user profiles.
- **Information Aggregation**: Fetching and combining data from multiple sources (e.g., displaying travel itineraries or combining analytics from various dashboards).
- **Task Automation**: Automating routine tasks like setting reminders, scheduling meetings, or sending emails.
- **User Personalization**: Tailoring responses based on user history, preferences, or ongoing interactions.

Best Practices and Tips
-----------------------
When integrating function calling into your generative AI applications, keep these tips in mind to get the most out of our Plano-Function models:

- **Keep it clear and simple**: Your function names and parameters should be straightforward and easy to understand. Think of it like explaining a task to a smart colleague - the clearer you are, the better the results.

- **Context is king**: Don't skimp on the descriptions for your functions and parameters. The more context you provide, the better the LLM can understand when and how to use each function.

- **Be specific with your parameters**: Instead of using generic types, get specific. If you're asking for a date, say it's a date. If you need a number between 1 and 10, spell that out. The more precise you are, the more accurate the LLM's responses will be.

- **Expect the unexpected**: Test your functions thoroughly, including edge cases. LLMs can be creative in their interpretations, so it's crucial to ensure your setup is robust and can handle unexpected inputs.

- **Watch and learn**: Pay attention to how the LLM uses your functions. Which ones does it call often? In what contexts? This information can help you optimize your setup over time.

Remember, working with LLMs is part science, part art. Don't be afraid to experiment and iterate to find what works best for your specific use case.
