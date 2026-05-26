.. _configuration_reference:

Configuration Reference
=======================

The following is a complete reference of the ``plano_config.yml`` that controls the behavior of a single instance of
the Plano gateway. This where you enable capabilities like routing to upstream LLm providers, defining prompt_targets
where prompts get routed to, apply guardrails, and enable critical agent observability features.

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
