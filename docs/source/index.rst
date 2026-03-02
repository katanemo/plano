Welcome to Plano!
=================

.. image:: /_static/img/PlanoTagline.svg
   :width: 100%
   :align: center

`Plano <https://github.com/katanemo/plano>`_ is delivery infrastructure for agentic apps. An AI-native proxy server and data plane designed to help you build agents faster, and deliver them reliably to production.

Plano pulls out the rote plumbing work (aka “hidden AI middleware”) and decouples you from brittle, ever‑changing framework abstractions. It centralizes what shouldn’t be bespoke in every codebase like agent routing and orchestration, rich agentic signals and traces for continuous improvement, guardrail filters for safety and moderation, and smart LLM routing APIs for UX and DX agility. Use any language or AI framework, and ship agents to production faster with Plano.

Built by contributors to the widely adopted `Envoy Proxy <https://www.envoyproxy.io/>`_, Plano **helps developers** focus more on the core product logic of agents, **product teams** accelerate feedback loops for reinforcement learning, and **engineering teams** standardize policies and access controls across every agent and LLM for safer, more reliable scaling.

.. tab-set::

  .. tab-item:: Get Started

    .. toctree::
      :caption: Get Started
      :titlesonly:
      :maxdepth: 2

      get_started/overview
      get_started/intro_to_plano
      get_started/quickstart

  .. tab-item:: Concepts

    .. toctree::
      :caption: Concepts
      :titlesonly:
      :maxdepth: 2

      concepts/listeners
      concepts/agents
      concepts/filter_chain
      concepts/llm_providers/llm_providers
      concepts/prompt_target
      concepts/signals

  .. tab-item:: Guides

    .. toctree::
      :caption: Guides
      :titlesonly:
      :maxdepth: 2

      guides/orchestration
      guides/llm_router
      guides/function_calling
      guides/observability/observability
      guides/prompt_guard
      guides/state

  .. tab-item:: Resources

    .. toctree::
      :caption: Resources
      :titlesonly:
      :maxdepth: 2

      resources/tech_overview/tech_overview
      resources/deployment
      resources/configuration_reference
      resources/cli_reference
      resources/llms_txt
