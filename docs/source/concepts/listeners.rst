.. _plano_overview_listeners:

Listeners
---------
**Listeners** are a top-level primitive in Plano that bind network traffic to the dataplane. They simplify the
configuration required to accept incoming connections from downstream clients (edge) and to expose a unified egress
endpoint for calls from your applications to upstream LLMs.

Plano builds on Envoy's Listener subsystem to streamline connection management for developers. It hides most of
Envoy's complexity behind sensible defaults and a focused configuration surface, so you can bind listeners without
deep knowledge of Envoy’s configuration model while still getting secure, reliable, and performant connections.

Listeners are modular building blocks: you can configure only inbound listeners (for edge proxying and guardrails),
only outbound/model-proxy listeners (for LLM routing from your services), or both together. This lets you fit Plano
cleanly into existing architectures, whether you need it at the edge, behind the firewall, or across the full
request path.


Network Topology
^^^^^^^^^^^^^^^^

The diagram below shows how inbound and outbound traffic flow through Plano and how listeners relate to agents,
prompt targets, and upstream LLMs:

.. image:: /_static/img/network-topology-ingress-egress.png
   :width: 100%
   :align: center


Inbound (Agent & Prompt Target)
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
Developers configure **inbound listeners** to accept connections from clients such as web frontends, backend
services, or other gateways. An inbound listener acts as the primary entry point for prompt traffic, handling
initial connection setup, TLS termination, guardrails, and forwarding incoming traffic to the appropriate prompt
targets or agents.

There are two primary types of inbound connections exposed via listeners:

* **Agent Inbound (Edge)**: Clients (web/mobile apps or other services) connect to Plano, send prompts, and receive
  responses. This is typically your public/edge listener where Plano applies guardrails, routing, and orchestration
  before returning results to the caller.

* **Prompt Target Inbound (Edge)**: Your application server calls Plano's internal listener targeting
  :ref:`prompt targets <prompt_target>` that can invoke tools and LLMs directly on its behalf.

Inbound listeners are where you attach :ref:`Filter Chains <filter_chain>` so that safety and context-building happen
consistently at the edge.

Outbound (Model Proxy & Egress)
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
Plano also exposes an **egress listener** that your applications call when sending requests to upstream LLM providers
or self-hosted models. From your application's perspective this looks like a single OpenAI-compatible HTTP endpoint
(for example, ``http://127.0.0.1:12000/v1``), while Plano handles provider selection, retries, and failover behind
the scenes.

Under the hood, Plano opens outbound HTTP(S) connections to upstream LLM providers using its unified API surface and
smart model routing. For more details on how Plano talks to models and how providers are configured, see
:ref:`LLM providers <llm_providers>`.

Configure Listeners
^^^^^^^^^^^^^^^^^^^

Listeners are configured via the ``listeners`` block in your Plano configuration. You can define one or more inbound
listeners (for example, ``type:edge``) or one or more outbound/model listeners (for example, ``type:model``), or both
in the same deployment.

To configure an inbound (edge) listener, add a ``listeners`` block to your configuration file and define at least one
listener with address, port, and protocol details:

.. literalinclude:: ./includes/plano_config.yaml
    :language: yaml
    :linenos:
    :lines: 1-13
    :emphasize-lines: 3-7
    :caption: Example Configuration

When you start Plano, you specify a listener address/port that you want to bind downstream. Plano also exposes a
predefined internal listener (``127.0.0.1:12000``) that you can use to proxy egress calls originating from your
application to LLMs (API-based or hosted) via prompt targets.

Internal Service Routes
^^^^^^^^^^^^^^^^^^^^^^^

Listeners support an optional ``routes`` block that lets you forward path-prefix traffic to co-located internal
services (for example, a Jaeger tracing UI or a Prometheus dashboard) through the same listener port:

.. code-block:: yaml

   listeners:
     - type: agent
       name: agent_1
       port: 8001
       routes:
         - path_prefix: /traces
           upstream: http://jaeger:16686
         - path_prefix: /metrics
           upstream: http://localhost:9090
       agents:
         - id: my_agent
           description: my agent

With the configuration above, requests to ``http://localhost:8001/traces`` are proxied to the Jaeger UI, while
all other requests are routed to the agent as usual. Each route entry requires:

- ``path_prefix`` — the URL prefix to match (e.g., ``/traces``).
- ``upstream`` — the full URL of the internal service to forward to.

Routes are evaluated before the default agent catch-all, so specific prefixes always take priority. A listener
can also define only ``routes`` (without ``agents``) to act as a lightweight reverse proxy for internal services.
