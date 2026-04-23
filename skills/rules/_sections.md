# Section Definitions

This file defines the sections used to organize Plano agent skills rules.
Files are assigned to sections based on their filename prefix.


| Prefix      | Section # | Title                      | Impact      | Description                                                                                                             |
| ----------- | --------- | -------------------------- | ----------- | ----------------------------------------------------------------------------------------------------------------------- |
| `config-`   | 1         | Configuration Fundamentals | CRITICAL    | Core config.yaml structure, versioning, listener types, and provider setup — the entry point for every Plano deployment |
| `routing-`  | 2         | Routing & Model Selection  | HIGH        | Intelligent LLM routing using preferences, aliases, and defaults to match tasks to the best model                       |
| `agent-`    | 3         | Agent Orchestration        | HIGH        | Multi-agent patterns, agent descriptions, and orchestration strategies for building agentic applications                |
| `filter-`   | 4         | Filter Chains & Guardrails | HIGH        | Request/response processing pipelines — ordering, MCP integration, and safety guardrails                                |
| `observe-`  | 5         | Observability & Debugging  | MEDIUM-HIGH | OpenTelemetry tracing, log levels, span attributes, and sampling for production visibility                              |
| `cli-`      | 6         | CLI Operations             | MEDIUM      | Using the planoai CLI for startup, tracing, CLI agents, project init, and code generation                               |
| `deploy-`   | 7         | Deployment & Security      | HIGH        | Docker deployment, environment variable management, health checks, and state storage for production                     |
| `advanced-` | 8         | Advanced Patterns          | MEDIUM      | Prompt targets, external API integration, and multi-listener architectures                                              |
