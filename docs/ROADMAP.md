# Plano Roadmap

This document describes the roadmap for the Plano project — its current focus areas, how features are planned, and how you can participate in shaping its direction.

Plano's roadmap is a **living plan** maintained through [GitHub Project Board](https://github.com/orgs/katanemo/projects/1), [GitHub Milestones](https://github.com/katanemo/plano/milestones), and the [Plano Enhancement Proposal (PEP)](peps/PEP-0000-process.md) process. This document provides the high-level context; the detailed, up-to-date tracking lives in those tools.

## Contributing to the Roadmap

Anyone can propose a feature or improvement:

1. **Small changes** (bug fixes, docs, minor enhancements) — open a [GitHub issue](https://github.com/katanemo/plano/issues) directly.
2. **Significant features** (new capabilities, architectural changes, new providers) — write a [Plano Enhancement Proposal (PEP)](peps/PEP-0000-process.md) and submit it as a PR to `docs/peps/`.
3. **Discussion first** — if you're unsure whether something warrants a PEP, start a [GitHub Discussion](https://github.com/katanemo/plano/discussions) or bring it to a [community meeting](#community-meetings).

If your proposal is accepted, a maintainer will assign it to a release milestone and link it on the project board.

### How to Help with Existing Items

- Browse the [project board](https://github.com/orgs/katanemo/projects/1) for items that interest you
- Look for issues labeled [`help wanted`](https://github.com/katanemo/plano/labels/help%20wanted) or [`good first issue`](https://github.com/katanemo/plano/labels/good%20first%20issue)
- Comment on any roadmap issue to volunteer or ask questions
- Attend a [community meeting](#community-meetings) to discuss design or get unblocked

## Current Focus Areas

### Actively Working On

These items are being implemented now. PRs are in flight or imminent.

- **Content guard models via filter chains** — use off-the-shelf SLMs (e.g., Llama Guard, ShieldGemma, WildGuard) as content moderation filters for jailbreak detection, toxicity screening, and content safety. The legacy `prompt_guards` config is being deprecated in favor of this composable filter-chain approach.
- **Gemini native protocol** — full support for Google's native Gemini API (generateContent, streamGenerateContent) as both a client-facing and upstream protocol, unlocking Gemini-specific features lost in translation
- **Model fallback & retry** — automatic failover to the next ranked model on provider errors
- **`prompt_guards` deprecation** — removing the legacy config path in favor of the filter-chain approach

### Next Up

Scoped and ready for contributors. If you want to help, these are the best places to start.

- **Circuit breaking** — per-provider/model circuit breakers to prevent cascading failures
- **PII detection & redaction** — configurable entity detection as a reference filter implementation
- **Accurate token counting** — provider-specific tokenizers for correct rate limiting and cost attribution
- **Response caching** — exact-match cache with configurable TTL, opt-out headers
- **Full Responses API support** — complete coverage of OpenAI's Responses API tool types

### Future

Planned but not yet scoped in detail. These are good candidates for [PEPs](peps/PEP-0000-process.md).

**Routing Intelligence**
- Embedding-based semantic routing for high-throughput use cases
- A/B testing with weighted traffic splitting and automatic metric collection
- Latency SLO routing based on historical P99 data

**Agentic Protocols**
- MCP server mode — expose Plano routing and orchestration as MCP tools
- A2A protocol — agent discovery and communication across platforms
- Streaming request passthrough for large-context workloads

**Observability & Evaluation**
- Pre-built Grafana dashboards for Agentic Signals
- Regression detection when signal quality degrades after changes
- Evaluation dataset capture for offline eval
- Prompt versioning correlated with signal quality

**Developer Experience**
- Client SDKs — typed Python, JavaScript, and Go clients
- Authentication — built-in API key and JWT validation for multi-tenant deployments
- Framework integrations for LangChain, CrewAI, Vercel AI SDK, and others

**Extensibility**
- WASM plugin SDK with stable ABI contract
- Community plugin registry for guardrails, routers, and provider adapters
- Python/JS filter runtime to lower the barrier beyond Rust/WASM

## Release Process

Plano follows a **time-based release cadence**, targeting a new release approximately every two weeks. Each release:

- Is tagged and published to [GitHub Releases](https://github.com/katanemo/plano/releases) with notes
- Publishes Docker images to Docker Hub, GHCR, and DigitalOcean Container Registry
- Publishes the `planoai` CLI to [PyPI](https://pypi.org/project/planoai/)
- Publishes pre-built `brightstaff` binaries

Features land in whichever release they're ready for. Large features that span multiple releases use the PEP process to track progress.

## Community Meetings

We hold regular community meetings open to all contributors:

- **When:** Schedule posted on [Discord](https://discord.gg/pGZf2gcwEc) and GitHub Discussions
- **Where:** Video link shared in Discord `#community-meetings` channel
- **What:** Demo new features, discuss active PEPs, triage roadmap items, answer questions
- **Notes:** Published to GitHub Discussions after each meeting

## Roadmap History

| Version | Theme | Key Deliverables |
|---|---|---|
| v0.4.x | Foundation | Agent orchestration, filter chains, cost/latency routing, 17+ providers, Agentic Signals |
| v0.5.x | _Planned_ | Gemini native protocol, content guard model demos, model fallback, `prompt_guards` deprecation, caching, PEP process |

## Feedback

Roadmap features and timelines may change based on community feedback, contributor capacity, and ecosystem shifts. If you depend on a specific item, you're encouraged to:

- Comment on the relevant GitHub issue to register interest
- Attend a community meeting to discuss timeline
- Contribute directly — the fastest way to get a feature shipped

Questions? Join our [Discord](https://discord.gg/pGZf2gcwEc) or open a [Discussion](https://github.com/katanemo/plano/discussions).
