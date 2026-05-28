.. _configuration_reference:

Configuration Reference
=======================

The following is a complete reference of the ``plano_config.yml`` that controls the behavior of a single instance of
the Plano gateway. This where you enable capabilities like routing to upstream LLm providers, defining prompt_targets
where prompts get routed to, apply guardrails, and enable critical agent observability features.

``model_providers`` entries support an optional ``headers`` map for extra string HTTP headers sent to the
upstream provider. The deprecated ``llm_providers`` key accepts the same provider fields for legacy configs.

.. literalinclude:: includes/plano_config_full_reference.yaml
    :language: yaml
    :linenos:
    :caption: :download:`Plano Configuration - Full Reference <includes/plano_config_full_reference.yaml>`
