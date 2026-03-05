.. _deployment:

Deployment
==========

Plano can be deployed in two ways: **natively** on the host (default) or inside a **Docker container**.

Native Deployment (Default)
---------------------------

Plano runs natively by default. Pre-compiled binaries (Envoy, WASM plugins, brightstaff) are automatically downloaded on the first run and cached at ``~/.plano/``.

Supported platforms: Linux (x86_64, aarch64), macOS (Apple Silicon).

Start Plano
~~~~~~~~~~~~

.. code-block:: bash

   planoai up plano_config.yaml

Options:

- ``--foreground`` — stay attached and stream logs (Ctrl+C to stop)
- ``--with-tracing`` — start a local OTLP trace collector

Runtime files (rendered configs, logs, PID file) are stored in ``~/.plano/run/``.

Stop Plano
~~~~~~~~~~

.. code-block:: bash

   planoai down

Build from Source (Developer)
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

If you want to build from source instead of using pre-compiled binaries, you need:

- `Rust <https://rustup.rs>`_ with the ``wasm32-wasip1`` target
- OpenSSL dev headers (``libssl-dev`` on Debian/Ubuntu, ``openssl`` on macOS)

.. code-block:: bash

   planoai build --native

Docker Deployment
-----------------

Below is a minimal, production-ready example showing how to deploy the Plano  Docker image directly and run basic runtime checks. Adjust image names, tags, and the ``plano_config.yaml`` path to match your environment.

.. note::
   You will need to pass all required environment variables that are referenced in your ``plano_config.yaml`` file.

For ``plano_config.yaml``, you can use any sample configuration defined earlier in the documentation. For example, you can try the :ref:`LLM Routing <llm_router>` sample config.

Docker Compose Setup
~~~~~~~~~~~~~~~~~~~~

Create a ``docker-compose.yml`` file with the following configuration:

.. code-block:: yaml

   # docker-compose.yml
   services:
     plano:
       image: katanemo/plano:0.4.11
       container_name: plano
       ports:
         - "10000:10000" # ingress (client -> plano)
         - "12000:12000" # egress (plano -> upstream/llm proxy)
       volumes:
         - ./plano_config.yaml:/app/plano_config.yaml:ro
       environment:
         - OPENAI_API_KEY=${OPENAI_API_KEY:?error}
         - ANTHROPIC_API_KEY=${ANTHROPIC_API_KEY:?error}

Starting the Stack
~~~~~~~~~~~~~~~~~~

Start the services from the directory containing ``docker-compose.yml`` and ``plano_config.yaml``:

.. code-block:: bash

   # Set required environment variables and start services
   OPENAI_API_KEY=xxx ANTHROPIC_API_KEY=yyy docker compose up -d

Check container health and logs:

.. code-block:: bash

   docker compose ps
   docker compose logs -f plano

You can also use the CLI with Docker mode:

.. code-block:: bash

   planoai up plano_config.yaml --docker
   planoai down --docker

Runtime Tests
-------------

Perform basic runtime tests to verify routing and functionality.

Gateway Smoke Test
~~~~~~~~~~~~~~~~~~

Test the chat completion endpoint with automatic routing:

.. code-block:: bash

   # Request handled by the gateway. 'model: "none"' lets Plano  decide routing
   curl --header 'Content-Type: application/json' \
     --data '{"messages":[{"role":"user","content":"tell me a joke"}], "model":"none"}' \
     http://localhost:12000/v1/chat/completions | jq .model

Expected output:

.. code-block:: json

   "gpt-5.2"

Model-Based Routing
~~~~~~~~~~~~~~~~~~~

Test explicit provider and model routing:

.. code-block:: bash

   curl -s -H "Content-Type: application/json" \
     -d '{"messages":[{"role":"user","content":"Explain quantum computing"}], "model":"anthropic/claude-sonnet-4-5"}' \
     http://localhost:12000/v1/chat/completions | jq .model

Expected output:

.. code-block:: json

   "claude-sonnet-4-5"

Troubleshooting
---------------

Common Issues and Solutions
~~~~~~~~~~~~~~~~~~~~~~~~~~~

**Environment Variables**
   Ensure all environment variables (``OPENAI_API_KEY``, ``ANTHROPIC_API_KEY``, etc.) used by ``plano_config.yaml`` are set before starting services.

**TLS/Connection Errors**
   If you encounter TLS or connection errors to upstream providers:

   - Check DNS resolution
   - Verify proxy settings
   - Confirm correct protocol and port in your ``plano_config`` endpoints

**Verbose Logging**
   To enable more detailed logs for debugging:

   - Run plano with a higher component log level
   - See the :ref:`Observability <observability>` guide for logging and monitoring details
   - Rebuild the image if required with updated log configuration

**CI/Automated Checks**
   For continuous integration or automated testing, you can use the curl commands above as health checks in your deployment pipeline.
