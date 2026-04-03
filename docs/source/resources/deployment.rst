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
       image: katanemo/plano:0.4.16
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

Kubernetes Deployment
---------------------

Plano runs as a single container in Kubernetes. The container bundles Envoy, WASM plugins, and brightstaff, managed by supervisord internally. Deploy it as a standard Kubernetes Deployment with your ``plano_config.yaml`` mounted via a ConfigMap and API keys injected via a Secret.

.. note::
   All environment variables referenced in your ``plano_config.yaml`` (e.g. ``$OPENAI_API_KEY``) must be set in the container environment. Use Kubernetes Secrets for API keys.

Step 1: Create the Config
~~~~~~~~~~~~~~~~~~~~~~~~~

Store your ``plano_config.yaml`` in a ConfigMap:

.. code-block:: bash

   kubectl create configmap plano-config --from-file=plano_config.yaml=./plano_config.yaml

Step 2: Create API Key Secrets
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Store your LLM provider API keys in a Secret:

.. code-block:: bash

   kubectl create secret generic plano-secrets \
     --from-literal=OPENAI_API_KEY=sk-... \
     --from-literal=ANTHROPIC_API_KEY=sk-ant-...

Step 3: Deploy Plano
~~~~~~~~~~~~~~~~~~~~

Create a ``plano-deployment.yaml``:

.. code-block:: yaml

   apiVersion: apps/v1
   kind: Deployment
   metadata:
     name: plano
     labels:
       app: plano
   spec:
     replicas: 1
     selector:
       matchLabels:
         app: plano
     template:
       metadata:
         labels:
           app: plano
       spec:
         containers:
           - name: plano
             image: katanemo/plano:0.4.16
             ports:
               - containerPort: 12000  # LLM gateway (chat completions, model routing)
                 name: llm-gateway
             envFrom:
               - secretRef:
                   name: plano-secrets
             env:
               - name: LOG_LEVEL
                 value: "info"
             volumeMounts:
               - name: plano-config
                 mountPath: /app/plano_config.yaml
                 subPath: plano_config.yaml
                 readOnly: true
             readinessProbe:
               httpGet:
                 path: /healthz
                 port: 12000
               initialDelaySeconds: 5
               periodSeconds: 10
             livenessProbe:
               httpGet:
                 path: /healthz
                 port: 12000
               initialDelaySeconds: 10
               periodSeconds: 30
             resources:
               requests:
                 memory: "256Mi"
                 cpu: "250m"
               limits:
                 memory: "512Mi"
                 cpu: "1000m"
         volumes:
           - name: plano-config
             configMap:
               name: plano-config
   ---
   apiVersion: v1
   kind: Service
   metadata:
     name: plano
   spec:
     selector:
       app: plano
     ports:
       - name: llm-gateway
         port: 12000
         targetPort: 12000

Apply it:

.. code-block:: bash

   kubectl apply -f plano-deployment.yaml

Step 4: Verify
~~~~~~~~~~~~~~

.. code-block:: bash

   # Check pod status
   kubectl get pods -l app=plano

   # Check logs
   kubectl logs -l app=plano -f

   # Test routing (port-forward for local testing)
   kubectl port-forward svc/plano 12000:12000

   curl -s -H "Content-Type: application/json" \
     -d '{"messages":[{"role":"user","content":"tell me a joke"}], "model":"none"}' \
     http://localhost:12000/v1/chat/completions | jq .model

Updating Configuration
~~~~~~~~~~~~~~~~~~~~~~

To update ``plano_config.yaml``, replace the ConfigMap and restart the pod:

.. code-block:: bash

   kubectl create configmap plano-config \
     --from-file=plano_config.yaml=./plano_config.yaml \
     --dry-run=client -o yaml | kubectl apply -f -

   kubectl rollout restart deployment/plano

Enabling OTEL Tracing
~~~~~~~~~~~~~~~~~~~~~

Plano emits OpenTelemetry traces for every request — including routing decisions, model selection, and upstream latency. To export traces to an OTEL collector in your cluster, add the ``tracing`` section to your ``plano_config.yaml``:

.. code-block:: yaml

   tracing:
     opentracing_grpc_endpoint: "http://otel-collector.monitoring:4317"
     random_sampling: 100       # percentage of requests to trace (1-100)
     trace_arch_internal: true  # include internal Plano spans
     span_attributes:
       header_prefixes:         # capture request headers as span attributes
         - "x-"
       static:                  # add static attributes to all spans
         environment: "production"
         service: "plano"

Set the ``OTEL_TRACING_GRPC_ENDPOINT`` environment variable or configure it directly in the config. Plano propagates the ``traceparent`` header end-to-end, so traces correlate across your upstream and downstream services.

Environment Variables Reference
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

The following environment variables can be set on the container:

.. list-table::
   :header-rows: 1
   :widths: 30 50 20

   * - Variable
     - Description
     - Default
   * - ``LOG_LEVEL``
     - Log verbosity (``debug``, ``info``, ``warn``, ``error``)
     - ``info``
   * - ``OPENAI_API_KEY``
     - OpenAI API key (if referenced in config)
     -
   * - ``ANTHROPIC_API_KEY``
     - Anthropic API key (if referenced in config)
     -
   * - ``OTEL_TRACING_GRPC_ENDPOINT``
     - OTEL collector endpoint for trace export
     - ``http://localhost:4317``

Any environment variable referenced in ``plano_config.yaml`` with ``$VAR_NAME`` syntax will be substituted at startup. Use Kubernetes Secrets for sensitive values and ConfigMaps or ``env`` entries for non-sensitive configuration.

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
