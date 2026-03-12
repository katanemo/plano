.. _filter_chain:

Filter Chains
==============

Filter chains are Plano's way of capturing **reusable workflow steps** in the dataplane, without duplication and coupling logic into application code. A filter chain is an ordered list of **mutations** that a request flows through before reaching its final destination —such as an agent, an LLM, or a tool backend. Each filter is a network-addressable service/path that can:

1. Inspect the incoming prompt, metadata, and conversation state.
2. Mutate or enrich the request (for example, rewrite queries or build context).
3. Short-circuit the flow and return a response early (for example, block a request on a compliance failure).
4. Emit structured logs and traces so you can debug and continuously improve your agents.

In other words, filter chains provide a lightweight programming model over HTTP for building reusable steps
in your agent architectures.

Typical Use Cases
-----------------

Without a dataplane programming model, teams tend to spread logic like query rewriting, compliance checks,
context building, and routing decisions across many agents and frameworks. This quickly becomes hard to reason
about and even harder to evolve.

Filter chains show up most often in patterns like:

* **Guardrails and Compliance**: Enforcing content policies, stripping or masking sensitive data, and blocking obviously unsafe or off-topic requests before they reach an agent.
* **Query rewriting, RAG, and Memory**: Rewriting user queries for retrieval, normalizing entities, and assembling RAG context envelopes while pulling in relevant memory (for example, conversation history, user profiles, or prior tool results) before calling a model or tool.
* **Cross-cutting Observability**: Injecting correlation IDs, sampling traces, or logging enriched request metadata at consistent points in the request path.

Because these behaviors live in the dataplane rather than inside individual agents, you define them once, attach them to many agents and prompt targets, and can add, remove, or reorder them without changing application code.

Configuration example
---------------------

Agent listener filter chain
^^^^^^^^^^^^^^^^^^^^^^^^^^^

The example below shows a configuration where an agent uses a filter chain with two filters: a query rewriter,
and a context builder that prepares retrieval context before the agent runs.

.. literalinclude:: ../../source/resources/includes/plano_config_agents_filters.yaml
    :language: yaml
    :linenos:
    :emphasize-lines: 7-14, 37-39
    :caption: Example Configuration

In this setup:

* The ``filters`` section defines the reusable filters, each running as its own HTTP/MCP service.
* The ``listeners`` section wires the ``rag_agent`` behind an ``agent`` listener and attaches a ``filter_chain`` with ``query_rewriter`` followed by ``context_builder``.
* When a request arrives at ``agent_1``, Plano executes the filters in order before handing control to ``rag_agent``.

Model listener filter chain
^^^^^^^^^^^^^^^^^^^^^^^^^^^^

Filter chains can also be attached directly to a **model listener**. This lets you run input guardrails on
direct LLM proxy requests (``/v1/chat/completions``, ``/v1/responses``, etc.) without an agent layer in between.

.. code-block:: yaml
    :caption: Model listener with a content-safety filter chain

    filters:
      - id: content_guard
        url: http://content-guard:10500
        type: http

    model_providers:
      - model: openai/gpt-4o-mini
        access_key: $OPENAI_API_KEY
        default: true

    listeners:
      - type: model
        name: llm_gateway
        port: 12000
        filter_chain:
          - content_guard

In this setup:

* The ``filter_chain`` is declared at the listener level (not per-agent).
* When a request arrives at the model listener, Plano executes the filters in order before forwarding the request to the upstream LLM provider.
* If a filter rejects the request (HTTP 4xx), the error is returned to the caller and the LLM is never called.


Filter Chain Programming Model (HTTP and MCP)
---------------------------------------------

Filters are implemented as simple RESTful endpoints reachable via HTTP. If you want to use the `Model Context Protocol (MCP) <https://modelcontextprotocol.io/>`_, you can configure that as well, which makes it easy to write filters in any language. However, you can also write a filter as a plain HTTP service.


When defining a filter in Plano configuration, the following fields are optional:

* ``type``: Controls the filter runtime. Use ``mcp`` for Model Context Protocol filters, or ``http`` for plain HTTP filters. Defaults to ``mcp``.
* ``transport``: Controls how Plano talks to the filter (defaults to ``streamable-http`` for efficient streaming interactions over HTTP). You can omit this for standard HTTP transport.
* ``tool``: Names the MCP tool Plano will invoke (by default, the filter ``id``). You can omit this if the tool name matches your filter id.

In practice, you typically only need to specify ``id`` and ``url`` to get started. Plano's sensible defaults mean a filter can be as simple as an HTTP endpoint. If you want to customize the runtime or protocol, those fields are there, but they're optional.

Filters communicate the outcome of their work via HTTP status codes:

* **HTTP 200 (Success)**: The filter successfully processed the request. If the filter mutated the request (e.g., rewrote a query or enriched context), those mutations are passed downstream.
* **HTTP 4xx (User Error)**: The request violates a filter's rules or constraints—for example, content moderation policies or compliance checks. The request is terminated, and the error is returned to the caller. This is *not* a fatal error; it represents expected user-facing policy enforcement.
* **HTTP 5xx (Fatal Error)**: An unexpected failure in the filter itself (for example, a crash or misconfiguration). Plano will surface the error back to the caller and record it in logs and traces.

This semantics allows filters to enforce guardrails and policies (4xx) without blocking the entire system, while still surfacing critical failures (5xx) for investigation.

If any filter fails or decides to terminate the request early (for example, after a policy violation), Plano will
surface that outcome back to the caller and record it in logs and traces. This makes filter chains a safe and
powerful abstraction for evolving your agent workflows over time.
