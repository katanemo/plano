.. _configuration_reference:

Configuration Reference
=======================

The following is a complete reference of the ``plano_config.yml`` that controls the behavior of a single instance of
the Plano gateway. This where you enable capabilities like routing to upstream LLm providers, defining prompt_targets
where prompts get routed to, apply guardrails, and enable critical agent observability features.

Agent headers
-------------

Each entry under ``agents`` may include an optional ``headers`` map of strings. Plano adds these headers only to
requests for that selected agent, including calls made when it is used in a filter chain. Configured values override
matching incoming headers (header names are case-insensitive). Other agents keep their own configuration, and omitting
``headers`` preserves existing header forwarding. Plano's routing, tracing, retry and content-length handling still apply.

Use the usual ``$VAR`` or ``${VAR}`` environment variable substitution to supply credentials at startup:

.. code-block:: yaml

    agents:
      - id: billing_agent
        url: http://billing-agent.team-x:8080
        headers:
          Authorization: "Bearer ${BILLING_AGENT_TOKEN}"

Invalid HTTP header names or values cause the agent request to fail without including the configured value in the error.
This uses the existing startup environment substitution; it does not introduce a secret-manager integration or a new
``passthrough_auth`` option for agents.

Model provider headers
----------------------

Each entry under ``model_providers`` (or the legacy ``llm_providers`` alias) may include a ``headers`` map of extra
HTTP headers that Plano adds to upstream LLM requests. Plano applies these headers after it sets authentication from
``access_key`` or ``passthrough_auth``, so you can supply provider-specific metadata without replacing the configured
credentials.

- **Type:** map of strings (header name → value)
- **Optional:** yes
- **Common uses:** required ``User-Agent`` values, organization or account identifiers, or other headers some APIs expect

.. code-block:: yaml

    model_providers:
      - model: moonshotai/kimi-for-coding
        access_key: $MOONSHOTAI_API_KEY
        base_url: https://api.kimi.com/coding/v1
        headers:
          User-Agent: "KimiCLI/1.3"

The example below includes this and other provider options in context.

.. literalinclude:: includes/plano_config_full_reference.yaml
    :language: yaml
    :linenos:
    :caption: :download:`Plano Configuration - Full Reference <includes/plano_config_full_reference.yaml>`
