<div align="center">
  <img src="docs/source/_static/img/PlanoTagline.svg" alt="Plano Logo" width="75%" height=auto>
</div>
<div align="center">

 _Plano is a models-native proxy server and data plane for agents._<br><br>
 Plano pulls out the rote plumbing work and decouples you from brittle framework abstractions, centralizing what shouldn’t be bespoke in every codebase - like agent routing and orchestration, rich agentic signals and traces for continuous improvement, guardrail filters for safety and moderation, and smart LLM routing APIs for UX and DX agility. Use any language or AI framework, and deliver agents faster to production.


[Quickstart](#Quickstart) •
[Route LLMs](#use-plano-as-a-llm-router) •
[Build Agentic Apps with Plano](#Build-Agentic-Apps-with-Plano) •
[Documentation](https://docs.planoai.dev) •
[Contact](#Contact)

[![pre-commit](https://github.com/katanemo/plano/actions/workflows/pre-commit.yml/badge.svg)](https://github.com/katanemo/plano/actions/workflows/pre-commit.yml)
[![rust tests (prompt and llm gateway)](https://github.com/katanemo/plano/actions/workflows/rust_tests.yml/badge.svg)](https://github.com/katanemo/plano/actions/workflows/rust_tests.yml)
[![e2e tests](https://github.com/katanemo/plano/actions/workflows/e2e_tests.yml/badge.svg)](https://github.com/katanemo/plano/actions/workflows/e2e_tests.yml)
[![Build and Deploy Documentation](https://github.com/katanemo/plano/actions/workflows/static.yml/badge.svg)](https://github.com/katanemo/plano/actions/workflows/static.yml)

</div>

# Overview
Building agentic demos is easy. Shipping agentic applications safely, reliably, and repeatably to production is hard. After the thrill of a quick hack, you end up building the “hidden middleware” to reach production: routing logic to reach the right agent, guardrail hooks for safety and moderation, evaluation and observability glue for continuous learning, and model/provider quirks scattered across frameworks and application code.

Plano solves this by moving core delivery concerns into a unified, out-of-process dataplane.

- **🚦 Orchestration:** Low-latency orchestration between agents; add new agents without modifying app code.
- **🔗 Model Agility:** Route [by model name, alias (semantic names) or automatically via preferences](#use-plano-as-a-llm-router).
- **🕵 Agentic Signals&trade;:** Zero-code capture of [behavior signals](#observability) plus OTEL traces/metrics across every agent.
- **🛡️ Moderation & Memory Hooks:** Build jailbreak protection, add moderation policies and memory consistently via [Filter Chains](https://docs.planoai.dev/concepts/filter_chain.html).

Plano pulls rote plumbing out of your framework so you can stay focused on what matters most: the core product logic of your agentic applications. Plano is backed by [industry-leading LLM research](https://planoai.dev/research) and built on [Envoy](https://envoyproxy.io) by its core contributors, who built critical infrastructure at scale for modern worklaods.

**High-Level Network Sequence Diagram**:
![high-level network plano arcitecture for Plano](docs/source/_static/img/plano_network_diagram_high_level.png)

**Jump to our [docs](https://docs.planoai.dev)** to learn how you can use Plano to improve the speed, safety and obervability of your agentic applications.

> [!IMPORTANT]
> Plano and the Arch family of LLMs (like Plano-Orchestrator-4B, Arch-Router, etc) are hosted free of charge in the US-central region to give you a great first-run developer experience of Plano. To scale and run in production, you can either run these LLMs locally or contact us on [Discord](https://discord.gg/pGZf2gcwEc) for API keys.

## Contact
To get in touch with us, please join our [discord server](https://discord.gg/pGZf2gcwEc). We actively monitor that and offer support there.

## Getting Started

Ready to try Plano? Check out our comprehensive documentation:

- **[Quickstart Guide](https://docs.planoai.dev/get_started/quickstart.html)** - Get up and running in minutes
- **[LLM Routing](https://docs.planoai.dev/guides/llm_router.html)** - Route by model name, alias, or intelligent preferences
- **[Agent Orchestration](https://docs.planoai.dev/guides/orchestration.html)** - Build multi-agent workflows
- **[Prompt Targets](https://docs.planoai.dev/concepts/prompt_target.html)** - Turn prompts into deterministic API calls
- **[Observability](https://docs.planoai.dev/guides/observability/observability.html)** - Traces, metrics, and logs

## Contribution
We would love feedback on our [Roadmap](https://github.com/orgs/katanemo/projects/1) and we welcome contributions to **Plano**!
Whether you're fixing bugs, adding new features, improving documentation, or creating tutorials, your help is much appreciated.
Please visit our [Contribution Guide](CONTRIBUTING.md) for more details
