.. _managing_conversational_state:

Conversational State
=====================

The OpenAI Responses API (``v1/responses``) is designed for multi-turn conversations where context needs to persist across requests. Plano provides a unified ``v1/responses`` API that works with **any LLM provider**—OpenAI, Anthropic, Azure OpenAI, DeepSeek, or any OpenAI-compatible provider—while automatically managing conversational state for you.

Unlike the traditional Chat Completions API where you manually manage conversation history by including all previous messages in each request, Plano handles state management behind the scenes. This means you can use the Responses API with any model provider, and Plano will persist conversation context across requests—making it ideal for building conversational agents that remember context without bloating every request with full message history.

How It Works
------------

When a client calls the Responses API:

1. **First request**: Plano generates a unique ``resp_id`` and stores the conversation state (messages, model, provider, timestamp).
2. **Subsequent requests**: The client includes the ``previous_resp_id`` from the previous response. Plano retrieves the stored conversation state, merges it with the new input, and sends the combined context to the LLM.
3. **Response**: The LLM sees the full conversation history without the client needing to resend all previous messages.

This pattern dramatically reduces bandwidth and makes it easier to build multi-turn agents—Plano handles the state plumbing so you can focus on agent logic.

**Example Using OpenAI Python SDK:**

.. code-block:: python

    from openai import OpenAI

    # Point to Plano's Model Proxy endpoint
    client = OpenAI(
        api_key="test-key",
        base_url="http://127.0.0.1:12000/v1"
    )

    # First turn - Plano creates a new conversation state
    response = client.responses.create(
        model="claude-sonnet-4-5",  # Works with any configured provider
        input="My name is Alice and I like Python"
    )

    # Save the response_id for conversation continuity
    resp_id = response.id
    print(f"Assistant: {response.output_text}")

    # Second turn - Plano automatically retrieves previous context
    resp2 = client.responses.create(
        model="claude-sonnet-4-5", # Make sure its configured in plano_config.yaml
        input="Please list all the messages you have received in our conversation, numbering each one.",
        previous_response_id=resp_id,
    )

    print(f"Assistant: {resp2.output_text}")
    # Output: "Your name is Alice and your favorite language is Python"

Notice how the second request only includes the new user message—Plano automatically merges it with the stored conversation history before sending to the LLM.

Configuration Overview
----------------------

State storage is configured in the ``state_storage`` section of your ``plano_config.yaml``:

.. literalinclude:: ../resources/includes/plano_config_state_storage_example.yaml
    :language: yaml
    :lines: 21-30
    :linenos:
    :emphasize-lines: 3,6-10

Plano supports two storage backends:

* **Memory**: Fast, ephemeral storage for development and testing. State is lost when Plano restarts.
* **PostgreSQL**: Durable, production-ready storage with support for Supabase and self-hosted PostgreSQL instances.

.. note::
   If you don't configure ``state_storage``, conversation state management is **disabled**. The Responses API will still work, but clients must manually include full conversation history in each request (similar to the Chat Completions API behavior).

Memory Storage (Development)
----------------------------

Memory storage keeps conversation state in-memory using a thread-safe ``HashMap``. It's perfect for local development, demos, and testing, but all state is lost when Plano restarts.

**Configuration**

Add this to your ``plano_config.yaml``:

.. code-block:: yaml

   state_storage:
     type: memory

That's it. No additional setup required.

**When to Use Memory Storage**

* Local development and debugging
* Demos and proof-of-concepts
* Automated testing environments
* Single-instance deployments where persistence isn't critical

**Limitations**

* State is lost on restart
* Not suitable for production workloads
* Cannot scale across multiple Plano instances

PostgreSQL Storage (Production)
--------------------------------

PostgreSQL storage provides durable, production-grade conversation state management. It works with both self-hosted PostgreSQL and Supabase (PostgreSQL-as-a-service), making it ideal for scaling multi-agent systems in production.

Prerequisites
^^^^^^^^^^^^^

Before configuring PostgreSQL storage, you need:

1. A PostgreSQL database (version 12 or later)
2. Database credentials (host, user, password)
3. The ``conversation_states`` table created in your database

**Setting Up the Database**

Run the SQL schema to create the required table:

.. literalinclude:: ../resources/db_setup/conversation_states.sql
    :language: sql
    :linenos:

**Using psql:**

.. code-block:: bash

   psql $DATABASE_URL -f docs/db_setup/conversation_states.sql

**Using Supabase Dashboard:**

1. Log in to your Supabase project
2. Navigate to the SQL Editor
3. Copy and paste the SQL from ``docs/db_setup/conversation_states.sql``
4. Run the query

Configuration
^^^^^^^^^^^^^

Once the database table is created, configure Plano to use PostgreSQL storage:

.. code-block:: yaml

   state_storage:
     type: postgres
     connection_string: "postgresql://user:password@host:5432/database"

**Using Environment Variables**

You should **never** hardcode credentials. Use environment variables instead:

.. code-block:: yaml

   state_storage:
     type: postgres
     connection_string: "postgresql://myuser:$DB_PASSWORD@db.example.com:5432/postgres"

Then set the environment variable before running Plano:

.. code-block:: bash

   export DB_PASSWORD="your-secure-password"
   # Run Plano or config validation
   ./plano

.. warning::
   **Special Characters in Passwords**: If your password contains special characters like ``#``, ``@``, or ``&``, you must URL-encode them in the connection string. For example, ``MyPass#123`` becomes ``MyPass%23123``.

Supabase Connection Strings
^^^^^^^^^^^^^^^^^^^^^^^^^^^^

Supabase requires different connection strings depending on your network setup. Most users should use the **Session Pooler** connection string.

**IPv4 Networks (Most Common)**

Use the Session Pooler connection string (port 5432):

.. code-block:: text

   postgresql://postgres.[PROJECT-REF]:[PASSWORD]@aws-0-[REGION].pooler.supabase.com:5432/postgres

**IPv6 Networks**

Use the direct connection (port 5432):

.. code-block:: text

   postgresql://postgres:[PASSWORD]@db.[PROJECT-REF].supabase.co:5432/postgres

**Finding Your Connection String**

1. Go to your Supabase project dashboard
2. Navigate to **Settings → Database → Connection Pooling**
3. Copy the **Session mode** connection string
4. Replace ``[YOUR-PASSWORD]`` with your actual database password
5. URL-encode special characters in the password

**Example Configuration**

.. code-block:: yaml

   state_storage:
     type: postgres
     connection_string: "postgresql://postgres.myproject:$DB_PASSWORD@aws-0-us-west-2.pooler.supabase.com:5432/postgres"

Then set the environment variable:

.. code-block:: bash

   # If your password is "MyPass#123", encode it as "MyPass%23123"
   export DB_PASSWORD="MyPass%23123"

Troubleshooting
---------------

**"Table 'conversation_states' does not exist"**

Run the SQL schema from ``docs/db_setup/conversation_states.sql`` against your database.

**Connection errors with Supabase**

* Verify you're using the correct connection string format (Session Pooler for IPv4)
* Check that your password is URL-encoded if it contains special characters
* Ensure your Supabase project hasn't paused due to inactivity (free tier)

**Permission errors**

Ensure your database user has the following permissions:

.. code-block:: sql

   GRANT SELECT, INSERT, UPDATE, DELETE ON conversation_states TO your_user;

**State not persisting across requests**

* Verify ``state_storage`` is configured in your ``plano_config.yaml``
* Check Plano logs for state storage initialization messages
* Ensure the client is sending the ``prev_response_id={$response_id}`` from previous responses

Best Practices
--------------

1. **Use environment variables for credentials**: Never hardcode database passwords in configuration files.
2. **Start with memory storage for development**: Switch to PostgreSQL when moving to production.
3. **Implement cleanup policies**: Prevent unbounded growth by regularly archiving or deleting old conversations.
4. **Monitor storage usage**: Track conversation state table size and query performance in production.
5. **Test failover scenarios**: Ensure your application handles storage backend failures gracefully.

Next Steps
----------

* Learn more about building :ref:`agents <agents>` that leverage conversational state
* Explore :ref:`filter chains <filter_chain>` for enriching conversation context
* See the :ref:`LLM Providers <llm_providers>` guide for configuring model routing
