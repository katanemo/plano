# Plano Agent Skills

> Best practices for building agents and agentic applications with Plano — the AI-native proxy and dataplane. Covers configuration, routing, agent orchestration, filter chains, observability, CLI operations, and deployment patterns.

**Version:** 1.0.0 | **Organization:** Plano

---

## Table of Contents

- [Section 1: Configuration Fundamentals](#section-1)
  - [1.1 Always Specify a Supported Config Version](#always-specify-a-supported-config-version)
  - [1.2 Choose the Right Listener Type for Your Use Case](#choose-the-right-listener-type-for-your-use-case)
  - [1.3 Register Model Providers with Correct Format Identifiers](#register-model-providers-with-correct-format-identifiers)
  - [1.4 Use Environment Variable Substitution for All Secrets](#use-environment-variable-substitution-for-all-secrets)
- [Section 2: Routing & Model Selection](#section-2)
  - [2.1 Always Set Exactly One Default Model Provider](#always-set-exactly-one-default-model-provider)
  - [2.2 Use Model Aliases for Semantic, Stable Model References](#use-model-aliases-for-semantic-stable-model-references)
  - [2.3 Use Passthrough Auth for Proxy and Multi-Tenant Setups](#use-passthrough-auth-for-proxy-and-multi-tenant-setups)
  - [2.4 Write Task-Specific Routing Preference Descriptions](#write-task-specific-routing-preference-descriptions)
- [Section 3: Agent Orchestration](#section-3)
  - [3.1 Register All Sub-Agents in Both `agents` and `listeners.agents`](#register-all-sub-agents-in-both-agents-and-listenersagents)
  - [3.2 Write Capability-Focused Agent Descriptions for Accurate Routing](#write-capability-focused-agent-descriptions-for-accurate-routing)
- [Section 4: Filter Chains & Guardrails](#section-4)
  - [4.1 Configure MCP Filters with Explicit Type and Transport](#configure-mcp-filters-with-explicit-type-and-transport)
  - [4.2 Configure Prompt Guards with Actionable Rejection Messages](#configure-prompt-guards-with-actionable-rejection-messages)
  - [4.3 Order Filter Chains with Guards First, Enrichment Last](#order-filter-chains-with-guards-first-enrichment-last)
- [Section 5: Observability & Debugging](#section-5)
  - [5.1 Add Custom Span Attributes for Correlation and Filtering](#add-custom-span-attributes-for-correlation-and-filtering)
  - [5.2 Enable Tracing with Appropriate Sampling for Your Environment](#enable-tracing-with-appropriate-sampling-for-your-environment)
  - [5.3 Use `planoai trace` to Inspect Routing Decisions](#use-planoai-trace-to-inspect-routing-decisions)
- [Section 6: CLI Operations](#section-6)
  - [6.1 Follow the `planoai up` Validation Workflow Before Debugging Runtime Issues](#follow-the-planoai-up-validation-workflow-before-debugging-runtime-issues)
  - [6.2 Generate Prompt Targets from Python Functions with `planoai generate_prompt_targets`](#generate-prompt-targets-from-python-functions-with-planoai-generateprompttargets)
  - [6.3 Use `planoai cli_agent` to Connect Claude Code Through Plano](#use-planoai-cliagent-to-connect-claude-code-through-plano)
  - [6.4 Use `planoai init` Templates to Bootstrap New Projects Correctly](#use-planoai-init-templates-to-bootstrap-new-projects-correctly)
- [Section 7: Deployment & Security](#section-7)
  - [7.1 Understand Plano's Docker Network Topology for Agent URL Configuration](#understand-planos-docker-network-topology-for-agent-url-configuration)
  - [7.2 Use PostgreSQL State Storage for Multi-Turn Conversations in Production](#use-postgresql-state-storage-for-multi-turn-conversations-in-production)
  - [7.3 Verify Listener Health Before Sending Requests](#verify-listener-health-before-sending-requests)
- [Section 8: Advanced Patterns](#section-8)
  - [8.1 Combine Multiple Listener Types for Layered Agent Architectures](#combine-multiple-listener-types-for-layered-agent-architectures)
  - [8.2 Design Prompt Targets with Precise Parameter Schemas](#design-prompt-targets-with-precise-parameter-schemas)

---

## Section 1: Configuration Fundamentals

*Core config.yaml structure, versioning, listener types, and provider setup — the entry point for every Plano deployment.*

### 1.1 Always Specify a Supported Config Version

**Impact:** `CRITICAL` — Plano rejects configs with missing or unsupported version fields — the version field gates all other validation
**Tags:** `config`, `versioning`, `validation`

## Always Specify a Supported Config Version

Every Plano `config.yaml` must include a `version` field at the top level. Plano validates configs against a versioned JSON schema — an unrecognized or missing version will cause `planoai up` to fail immediately with a schema validation error before the container starts.

**Incorrect (missing or invalid version):**

```yaml
# No version field — fails schema validation
listeners:
  - type: model
    name: model_listener
    port: 12000

model_providers:
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
```

**Correct (explicit supported version):**

```yaml
version: v0.3.0

listeners:
  - type: model
    name: model_listener
    port: 12000

model_providers:
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    default: true
```

Use the latest supported version unless you are targeting a specific deployed Plano image. Current supported versions: `v0.1`, `v0.1.0`, `0.1-beta`, `v0.2.0`, `v0.3.0`. Prefer `v0.3.0` for all new projects.

Reference: https://github.com/katanemo/archgw/blob/main/config/plano_config_schema.yaml

---

### 1.2 Choose the Right Listener Type for Your Use Case

**Impact:** `CRITICAL` — The listener type determines the entire request processing pipeline — choosing the wrong type means features like prompt functions or agent routing are unavailable
**Tags:** `config`, `listeners`, `architecture`, `routing`

## Choose the Right Listener Type for Your Use Case

Plano supports three listener types, each serving a distinct purpose. `listeners` is the only required top-level array in a Plano config. Every listener needs at minimum a `type`, `name`, and `port`.

| Type | Use When | Key Feature |
|------|----------|-------------|
| `model` | You want an OpenAI-compatible LLM gateway | Routes to multiple LLM providers, supports model aliases and routing preferences |
| `prompt` | You want LLM-callable custom functions | Define `prompt_targets` that the LLM dispatches as function calls |
| `agent` | You want multi-agent orchestration | Routes user requests to specialized sub-agents by matching agent descriptions |

**Incorrect (using `model` when agents need orchestration):**

```yaml
version: v0.3.0

# Wrong: a model listener cannot route to backend agent services
listeners:
  - type: model
    name: main
    port: 12000

agents:
  - id: weather_agent
    url: http://host.docker.internal:8001
```

**Correct (use `agent` listener for multi-agent systems):**

```yaml
version: v0.3.0

agents:
  - id: weather_agent
    url: http://host.docker.internal:8001
  - id: travel_agent
    url: http://host.docker.internal:8002

listeners:
  - type: agent
    name: orchestrator
    port: 8000
    router: plano_orchestrator_v1
    agents:
      - id: weather_agent
        description: Provides real-time weather, forecasts, and conditions for any city.
      - id: travel_agent
        description: Books flights, hotels, and travel itineraries.

model_providers:
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    default: true
```

A single Plano instance can expose multiple listeners on different ports, each with a different type, to serve different clients simultaneously.

Reference: https://github.com/katanemo/archgw

---

### 1.3 Register Model Providers with Correct Format Identifiers

**Impact:** `CRITICAL` — Incorrect provider format causes request translation failures — Plano must know the wire format each provider expects
**Tags:** `config`, `model-providers`, `llm`, `api-format`

## Register Model Providers with Correct Format Identifiers

Plano translates requests between its internal format and each provider's API. The `model` field uses `provider/model-name` syntax which determines both the upstream endpoint and the request/response translation layer. Some providers require an explicit `provider_interface` override.

**Provider format reference:**

| Model prefix | Wire format | Example |
|---|---|---|
| `openai/*` | OpenAI | `openai/gpt-4o` |
| `anthropic/*` | Anthropic | `anthropic/claude-sonnet-4-20250514` |
| `gemini/*` | Google Gemini | `gemini/gemini-2.0-flash` |
| `mistral/*` | Mistral | `mistral/mistral-large-latest` |
| `groq/*` | Groq | `groq/llama-3.3-70b-versatile` |
| `deepseek/*` | DeepSeek | `deepseek/deepseek-chat` |
| `xai/*` | Grok (OpenAI-compat) | `xai/grok-2` |
| `together_ai/*` | Together.ai | `together_ai/meta-llama/Llama-3` |
| `custom/*` | Requires `provider_interface` | `custom/my-local-model` |

**Incorrect (missing provider prefix, ambiguous format):**

```yaml
model_providers:
  - model: gpt-4o            # Missing openai/ prefix — Plano cannot route this
    access_key: $OPENAI_API_KEY

  - model: claude-3-5-sonnet # Missing anthropic/ prefix
    access_key: $ANTHROPIC_API_KEY
```

**Correct (explicit provider prefixes):**

```yaml
model_providers:
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    default: true

  - model: anthropic/claude-sonnet-4-20250514
    access_key: $ANTHROPIC_API_KEY

  - model: gemini/gemini-2.0-flash
    access_key: $GOOGLE_API_KEY
```

**For local or self-hosted models (Ollama, LiteLLM, vLLM):**

```yaml
model_providers:
  - model: custom/llama3
    base_url: http://host.docker.internal:11434/v1   # Ollama endpoint
    provider_interface: openai                        # Ollama speaks OpenAI format
    default: true
```

Always set `default: true` on exactly one provider per listener so Plano has a fallback when routing preferences do not match.

Reference: https://github.com/katanemo/archgw

---

### 1.4 Use Environment Variable Substitution for All Secrets

**Impact:** `CRITICAL` — Hardcoded API keys in config.yaml will be committed to version control and exposed in Docker container inspect output
**Tags:** `config`, `security`, `secrets`, `api-keys`, `environment-variables`

## Use Environment Variable Substitution for All Secrets

Plano supports `$VAR_NAME` substitution in config values. This applies to `access_key` fields, `connection_string` for state storage, and `http_headers` in prompt targets and endpoints. Never hardcode credentials — Plano reads them from environment variables or a `.env` file at startup via `planoai up`.

**Incorrect (hardcoded secrets):**

```yaml
version: v0.3.0

model_providers:
  - model: openai/gpt-4o
    access_key: abcdefghijklmnopqrstuvwxyz...   # Hardcoded — never do this

state_storage:
  type: postgres
  connection_string: "postgresql://admin:mysecretpassword@prod-db:5432/plano"

prompt_targets:
  - name: get_data
    endpoint:
      name: my_api
      http_headers:
        Authorization: "Bearer abcdefghijklmnopqrstuvwxyz"   # Hardcoded token
```

**Correct (environment variable substitution):**

```yaml
version: v0.3.0

model_providers:
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    default: true

  - model: anthropic/claude-sonnet-4-20250514
    access_key: $ANTHROPIC_API_KEY

state_storage:
  type: postgres
  connection_string: "postgresql://${DB_USER}:${DB_PASS}@${DB_HOST}:5432/${DB_NAME}"

prompt_targets:
  - name: get_data
    endpoint:
      name: my_api
      http_headers:
        Authorization: "Bearer $MY_API_TOKEN"
```

**`.env` file pattern (loaded automatically by `planoai up`):**

```bash
# .env — add to .gitignore
OPENAI_API_KEY=abcdefghijklmnopqrstuvwxyz...
ANTHROPIC_API_KEY=abcdefghijklmnopqrstuvwxyz...
DB_USER=plano
DB_PASS=secure-password
DB_HOST=localhost
MY_API_TOKEN=abcdefghijklmnopqrstuvwxyz...
```

Plano also accepts keys set directly in the shell environment. Variables referenced in config but not found at startup cause `planoai up` to fail with a clear error listing the missing keys.

Reference: https://github.com/katanemo/archgw

---

## Section 2: Routing & Model Selection

*Intelligent LLM routing using preferences, aliases, and defaults to match tasks to the best model.*

### 2.1 Always Set Exactly One Default Model Provider

**Impact:** `HIGH` — Without a default provider, Plano has no fallback when routing preferences do not match — requests with unclassified intent will fail
**Tags:** `routing`, `defaults`, `model-providers`, `reliability`

## Always Set Exactly One Default Model Provider

When a request does not match any routing preference, Plano forwards it to the `default: true` provider. Without a default, unmatched requests fail. If multiple providers are marked `default: true`, Plano uses the first one — which can produce unexpected behavior.

**Incorrect (no default provider set):**

```yaml
version: v0.3.0

model_providers:
  - model: openai/gpt-4o-mini     # No default: true anywhere
    access_key: $OPENAI_API_KEY
    routing_preferences:
      - name: summarization
        description: Summarizing documents and extracting key points

  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    routing_preferences:
      - name: code_generation
        description: Writing new functions and implementing algorithms
```

**Incorrect (multiple defaults — ambiguous):**

```yaml
model_providers:
  - model: openai/gpt-4o-mini
    default: true               # First default
    access_key: $OPENAI_API_KEY

  - model: openai/gpt-4o
    default: true               # Second default — confusing
    access_key: $OPENAI_API_KEY
```

**Correct (exactly one default, covering unmatched requests):**

```yaml
version: v0.3.0

model_providers:
  - model: openai/gpt-4o-mini
    access_key: $OPENAI_API_KEY
    default: true               # Handles general/unclassified requests
    routing_preferences:
      - name: summarization
        description: Summarizing documents, articles, and meeting notes
      - name: classification
        description: Categorizing inputs, labeling, and intent detection

  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    routing_preferences:
      - name: code_generation
        description: Writing, debugging, and reviewing code
      - name: complex_reasoning
        description: Multi-step math, logical analysis, research synthesis
```

Choose your most cost-effective capable model as the default — it handles all traffic that doesn't match specialized preferences.

Reference: [https://github.com/katanemo/archgw](https://github.com/katanemo/archgw)

---

### 2.2 Use Model Aliases for Semantic, Stable Model References

**Impact:** `MEDIUM` — Hardcoded model names in client code require code changes when you swap providers; aliases let you update routing in config.yaml alone
**Tags:** `routing`, `model-aliases`, `maintainability`, `client-integration`

## Use Model Aliases for Semantic, Stable Model References

`model_aliases` map human-readable names to specific model identifiers. Client applications reference the alias, not the underlying model. When you want to upgrade from `gpt-4o` to a new model, you change one line in `config.yaml` — not every client calling the API.

**Incorrect (clients hardcode specific model names):**

```yaml
# config.yaml — no aliases defined
version: v0.3.0

listeners:
  - type: model
    name: model_listener
    port: 12000

model_providers:
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    default: true
```

```python
# Client code — brittle, must be updated when model changes
client.chat.completions.create(model="gpt-4o", ...)
```

**Correct (semantic aliases, stable client contracts):**

```yaml
version: v0.3.0

listeners:
  - type: model
    name: model_listener
    port: 12000

model_providers:
  - model: openai/gpt-4o-mini
    access_key: $OPENAI_API_KEY
    default: true
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
  - model: anthropic/claude-sonnet-4-20250514
    access_key: $ANTHROPIC_API_KEY

model_aliases:
  plano.fast.v1:
    target: gpt-4o-mini          # Cheap, fast — for high-volume tasks

  plano.smart.v1:
    target: gpt-4o               # High capability — for complex reasoning

  plano.creative.v1:
    target: claude-sonnet-4-20250514  # Strong creative writing and analysis

  plano.v1:
    target: gpt-4o               # Default production alias
```

```python
# Client code — stable, alias is the contract
client.chat.completions.create(model="plano.smart.v1", ...)
```

**Alias naming conventions:**
- `<org>.<purpose>.<version>` — e.g., `plano.fast.v1`, `acme.code.v2`
- Bumping `.v2` → `.v3` lets you run old and new aliases simultaneously during rollouts
- `plano.v1` as a canonical default gives clients a single stable entry point

Reference: https://github.com/katanemo/archgw

---

### 2.3 Use Passthrough Auth for Proxy and Multi-Tenant Setups

**Impact:** `MEDIUM` — Without passthrough auth, self-hosted proxy services (LiteLLM, vLLM, etc.) reject Plano's requests because the wrong Authorization header is sent
**Tags:** `routing`, `authentication`, `proxy`, `litellm`, `multi-tenant`

## Use Passthrough Auth for Proxy and Multi-Tenant Setups

When routing to a self-hosted LLM proxy (LiteLLM, vLLM, OpenRouter, Azure APIM) or in multi-tenant setups where clients supply their own keys, set `passthrough_auth: true`. This forwards the client's `Authorization` header rather than Plano's configured `access_key`. Combine with a `base_url` pointing to the proxy.

**Incorrect (Plano sends its own key to a proxy that expects the client's key):**

```yaml
model_providers:
  - model: custom/proxy
    base_url: http://host.docker.internal:8000
    access_key: $SOME_KEY    # Plano overwrites the client's auth — proxy rejects it
```

**Correct (forward client Authorization header to the proxy):**

```yaml
version: v0.3.0

listeners:
  - type: model
    name: model_listener
    port: 12000

model_providers:
  - model: custom/litellm-proxy
    base_url: http://host.docker.internal:4000    # LiteLLM server
    provider_interface: openai                    # LiteLLM uses OpenAI format
    passthrough_auth: true                        # Forward client's Bearer token
    default: true
```

**Multi-tenant pattern (client supplies their own API key):**

```yaml
model_providers:
  # Plano acts as a passthrough gateway; each client has their own OpenAI key
  - model: openai/gpt-4o
    passthrough_auth: true    # No access_key here — client's key is forwarded
    default: true
```

**Combined: proxy for some models, Plano-managed for others:**

```yaml
model_providers:
  - model: openai/gpt-4o-mini
    access_key: $OPENAI_API_KEY    # Plano manages this key
    default: true
    routing_preferences:
      - name: quick tasks
        description: Short answers, simple lookups, fast completions

  - model: custom/vllm-llama
    base_url: http://gpu-server:8000
    provider_interface: openai
    passthrough_auth: true         # vLLM cluster handles its own auth
    routing_preferences:
      - name: long context
        description: Processing very long documents, multi-document analysis
```

Reference: https://github.com/katanemo/archgw

---

### 2.4 Write Task-Specific Routing Preference Descriptions

**Impact:** `HIGH` — Vague preference descriptions cause Plano's internal router LLM to misclassify requests, routing expensive tasks to cheap models and vice versa
**Tags:** `routing`, `model-selection`, `preferences`, `llm-routing`

## Write Task-Specific Routing Preference Descriptions

Plano's `plano_orchestrator_v1` router uses a 1.5B preference-aligned LLM to classify incoming requests against your `routing_preferences` descriptions. It routes the request to the first provider whose preferences match. Description quality directly determines routing accuracy.

**Incorrect (vague, overlapping descriptions):**

```yaml
model_providers:
  - model: openai/gpt-4o-mini
    access_key: $OPENAI_API_KEY
    default: true
    routing_preferences:
      - name: simple
        description: easy tasks      # Too vague — what is "easy"?

  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    routing_preferences:
      - name: hard
        description: hard tasks      # Too vague — overlaps with "easy"
```

**Correct (specific, distinct task descriptions):**

```yaml
model_providers:
  - model: openai/gpt-4o-mini
    access_key: $OPENAI_API_KEY
    default: true
    routing_preferences:
      - name: summarization
        description: >
          Summarizing documents, articles, emails, or meeting transcripts.
          Extracting key points, generating TL;DR sections, condensing long text.
      - name: classification
        description: >
          Categorizing inputs, sentiment analysis, spam detection,
          intent classification, labeling structured data fields.
      - name: translation
        description: >
          Translating text between languages, localization tasks.

  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    routing_preferences:
      - name: code_generation
        description: >
          Writing new functions, classes, or modules from scratch.
          Implementing algorithms, boilerplate generation, API integrations.
      - name: code_review
        description: >
          Reviewing code for bugs, security vulnerabilities, performance issues.
          Suggesting refactors, explaining complex code, debugging errors.
      - name: complex_reasoning
        description: >
          Multi-step math problems, logical deduction, strategic planning,
          research synthesis requiring chain-of-thought reasoning.
```

**Key principles for good preference descriptions:**
- Use concrete action verbs: "writing", "reviewing", "translating", "summarizing"
- List 3–5 specific sub-tasks or synonyms for each preference
- Ensure preferences across providers are mutually exclusive in scope
- Test with representative queries using `planoai trace` and `--where` filters to verify routing decisions

Reference: https://github.com/katanemo/archgw

---

## Section 3: Agent Orchestration

*Multi-agent patterns, agent descriptions, and orchestration strategies for building agentic applications.*

### 3.1 Register All Sub-Agents in Both `agents` and `listeners.agents`

**Impact:** `CRITICAL` — An agent registered only in `agents` but not referenced in a listener's agent list is unreachable; an agent listed in a listener but missing from `agents` causes a startup error
**Tags:** `agent`, `orchestration`, `config`, `multi-agent`

## Register All Sub-Agents in Both `agents` and `listeners.agents`

Plano's agent system has two separate concepts: the global `agents` array (defines the agent's ID and backend URL) and the `listeners[].agents` array (controls which agents are available to an orchestrator and provides their routing descriptions). Both must reference the same agent ID.

**Incorrect (agent defined globally but not referenced in listener):**

```yaml
version: v0.3.0

agents:
  - id: weather_agent
    url: http://host.docker.internal:8001
  - id: news_agent              # Defined but never referenced in any listener
    url: http://host.docker.internal:8002

listeners:
  - type: agent
    name: orchestrator
    port: 8000
    router: plano_orchestrator_v1
    agents:
      - id: weather_agent
        description: Provides weather forecasts and current conditions.
      # news_agent is missing here — the orchestrator cannot route to it
```

**Incorrect (listener references an agent ID not in the global agents list):**

```yaml
agents:
  - id: weather_agent
    url: http://host.docker.internal:8001

listeners:
  - type: agent
    name: orchestrator
    port: 8000
    router: plano_orchestrator_v1
    agents:
      - id: weather_agent
        description: Provides weather forecasts.
      - id: flights_agent        # ID not in global agents[] — startup error
        description: Provides flight status information.
```

**Correct (every agent ID appears in both places):**

```yaml
version: v0.3.0

agents:
  - id: weather_agent
    url: http://host.docker.internal:8001
  - id: flights_agent
    url: http://host.docker.internal:8002
  - id: hotels_agent
    url: http://host.docker.internal:8003

model_providers:
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    default: true

listeners:
  - type: agent
    name: travel_orchestrator
    port: 8000
    router: plano_orchestrator_v1
    agents:
      - id: weather_agent
        description: Real-time weather, forecasts, and climate data for any city.
      - id: flights_agent
        description: Live flight status, schedules, gates, and delays.
      - id: hotels_agent
        description: Hotel search, availability, pricing, and booking.
        default: true    # Fallback if no other agent matches
```

Set `default: true` on one agent in each listener's agents list to handle unmatched requests. The agent's URL in the global `agents` array is the HTTP endpoint Plano forwards matching requests to — it must be reachable from within the Docker container (use `host.docker.internal` for services on the host).

Reference: https://github.com/katanemo/archgw

---

### 3.2 Write Capability-Focused Agent Descriptions for Accurate Routing

**Impact:** `HIGH` — The orchestrator LLM routes requests purely by reading agent descriptions — poor descriptions cause misroutes to the wrong specialized agent
**Tags:** `agent`, `orchestration`, `descriptions`, `routing`, `multi-agent`

## Write Capability-Focused Agent Descriptions for Accurate Routing

In an `agent` listener, Plano's orchestrator reads each agent's `description` and routes user requests to the best-matching agent. This is LLM-based intent matching — the description is the entire specification the router sees. Write it as a capability manifest: what can this agent do, what data does it have access to, and what types of requests should it handle?

**Incorrect (generic, overlapping descriptions):**

```yaml
listeners:
  - type: agent
    name: orchestrator
    port: 8000
    router: plano_orchestrator_v1
    agents:
      - id: agent_1
        description: Helps users with information    # Too generic — matches everything

      - id: agent_2
        description: Also helps users               # Indistinguishable from agent_1
```

**Correct (specific capabilities, distinct domains, concrete examples):**

```yaml
version: v0.3.0

agents:
  - id: weather_agent
    url: http://host.docker.internal:8001
  - id: flight_agent
    url: http://host.docker.internal:8002
  - id: hotel_agent
    url: http://host.docker.internal:8003

listeners:
  - type: agent
    name: travel_orchestrator
    port: 8000
    router: plano_orchestrator_v1
    agents:
      - id: weather_agent
        description: >
          Provides real-time weather conditions and multi-day forecasts for any city
          worldwide. Handles questions about temperature, precipitation, wind, humidity,
          sunrise/sunset times, and severe weather alerts. Examples: "What's the weather
          in Tokyo?", "Will it rain in London this weekend?", "Sunrise time in New York."

      - id: flight_agent
        description: >
          Provides live flight status, schedules, gate information, delays, and
          aircraft details for any flight number or route between airports.
          Handles questions about departures, arrivals, and airline information.
          Examples: "Is AA123 on time?", "Flights from JFK to LAX tomorrow."

      - id: hotel_agent
        description: >
          Searches and books hotel accommodations, compares room types, pricing,
          and availability. Handles check-in/check-out dates, amenities, and
          cancellation policies. Examples: "Hotels near Times Square for next Friday."
```

**Description writing checklist:**
- State the primary domain in the first sentence
- List 3–5 specific data types or question categories this agent handles
- Include 2–3 concrete example user queries in quotes
- Avoid capability overlap between agents — if they overlap, the router will split traffic unpredictably
- Keep descriptions under 150 words — the orchestrator reads all descriptions per request

Reference: https://github.com/katanemo/archgw

---

## Section 4: Filter Chains & Guardrails

*Request/response processing pipelines — ordering, MCP integration, and safety guardrails.*

### 4.1 Configure MCP Filters with Explicit Type and Transport

**Impact:** `MEDIUM` — Omitting type and transport fields relies on defaults that may not match your MCP server's protocol implementation
**Tags:** `filter`, `mcp`, `integration`, `configuration`

## Configure MCP Filters with Explicit Type and Transport

Plano filters integrate with external services via MCP (Model Context Protocol) or plain HTTP. MCP filters call a specific tool on a remote MCP server. Always specify `type`, `transport`, and optionally `tool` (defaults to the filter `id`) to ensure Plano connects correctly to your filter implementation.

**Incorrect (minimal filter definition relying on all defaults):**

```yaml
filters:
  - id: my_guard          # Plano infers type=mcp, transport=streamable-http, tool=my_guard
    url: http://localhost:10500
    # If your MCP server uses a different tool name or transport, this silently misroutes
```

**Correct (explicit configuration for each filter):**

```yaml
version: v0.3.0

filters:
  - id: input_guards
    url: http://host.docker.internal:10500
    type: mcp                        # Explicitly MCP protocol
    transport: streamable-http       # Streamable HTTP transport
    tool: input_guards               # MCP tool name (matches MCP server registration)

  - id: query_rewriter
    url: http://host.docker.internal:10501
    type: mcp
    transport: streamable-http
    tool: rewrite_query              # Tool name differs from filter ID — explicit is safer

  - id: custom_validator
    url: http://host.docker.internal:10503
    type: http                       # Plain HTTP filter (not MCP)
    # No tool field for HTTP filters
```

**MCP filter implementation contract:**
Your MCP server must expose a tool matching the `tool` name. The tool receives the request payload and must return either:
- A modified request (to pass through with changes)
- A rejection response (to short-circuit the pipeline)

**HTTP filter alternative** — use `type: http` for simpler request/response interceptors that don't need the MCP protocol:

```yaml
filters:
  - id: auth_validator
    url: http://host.docker.internal:9000/validate
    type: http    # Plano POSTs the request, expects the modified request back
```

Reference: https://github.com/katanemo/archgw

---

### 4.2 Configure Prompt Guards with Actionable Rejection Messages

**Impact:** `MEDIUM` — A generic or empty rejection message leaves users confused about why their request was blocked and unable to rephrase appropriately
**Tags:** `filter`, `guardrails`, `jailbreak`, `security`, `ux`

## Configure Prompt Guards with Actionable Rejection Messages

Plano has built-in `prompt_guards` for detecting jailbreak attempts. When triggered, Plano returns the `on_exception.message` instead of forwarding the request. Write messages that explain the restriction and suggest what the user can do instead — both for user experience and to reduce support burden.

**Incorrect (no message configured — returns a generic error):**

```yaml
version: v0.3.0

prompt_guards:
  input_guards:
    jailbreak:
      on_exception: {}    # Empty — returns unhelpful generic error
```

**Incorrect (cryptic technical message):**

```yaml
prompt_guards:
  input_guards:
    jailbreak:
      on_exception:
        message: "Error code 403: guard triggered"    # Unhelpful to the user
```

**Correct (clear, actionable, brand-appropriate message):**

```yaml
version: v0.3.0

prompt_guards:
  input_guards:
    jailbreak:
      on_exception:
        message: >
          I'm not able to help with that request. This assistant is designed
          to help with [your use case, e.g., customer support, coding questions].
          Please rephrase your question or contact support@yourdomain.com
          if you believe this is an error.
```

**Combining prompt_guards with MCP filter guardrails:**

```yaml
# Built-in jailbreak detection (fast, no external service needed)
prompt_guards:
  input_guards:
    jailbreak:
      on_exception:
        message: "This request cannot be processed. Please ask about our products and services."

# MCP-based custom guards for additional policy enforcement
filters:
  - id: topic_restriction
    url: http://host.docker.internal:10500
    type: mcp
    transport: streamable-http
    tool: topic_restriction    # Custom filter for domain-specific restrictions

listeners:
  - type: agent
    name: customer_support
    port: 8000
    router: plano_orchestrator_v1
    agents:
      - id: support_agent
        description: Customer support assistant for product questions and order issues.
        filter_chain:
          - topic_restriction    # Additional custom topic filtering
```

`prompt_guards` applies globally to all listeners. Use `filter_chain` on individual agents for per-agent policies.

Reference: https://github.com/katanemo/archgw

---

### 4.3 Order Filter Chains with Guards First, Enrichment Last

**Impact:** `HIGH` — Running context builders before input guards means jailbreak attempts get RAG-enriched context before being blocked — wasting compute and risking data exposure
**Tags:** `filter`, `guardrails`, `security`, `pipeline`, `ordering`

## Order Filter Chains with Guards First, Enrichment Last

A `filter_chain` is an ordered list of filter IDs applied sequentially to each request. The order is semantically meaningful: each filter receives the output of the previous one. Safety and validation filters must run first to short-circuit bad requests before expensive enrichment filters process them.

**Recommended filter chain order:**

1. **Input guards** — jailbreak detection, PII detection, topic restrictions (reject early)
2. **Query rewriting** — normalize or enhance the user query
3. **Context building** — RAG retrieval, tool lookup, knowledge injection (expensive)
4. **Output guards** — validate or sanitize LLM response before returning

**Incorrect (context built before guards — wasteful and potentially unsafe):**

```yaml
filters:
  - id: context_builder
    url: http://host.docker.internal:10502    # Runs expensive RAG retrieval first
  - id: query_rewriter
    url: http://host.docker.internal:10501
  - id: input_guards
    url: http://host.docker.internal:10500    # Guards run last — jailbreak gets context

listeners:
  - type: agent
    name: rag_orchestrator
    port: 8000
    router: plano_orchestrator_v1
    agents:
      - id: rag_agent
        filter_chain:
          - context_builder   # Wrong: expensive enrichment before safety check
          - query_rewriter
          - input_guards
```

**Correct (guards block bad requests before any enrichment):**

```yaml
version: v0.3.0

filters:
  - id: input_guards
    url: http://host.docker.internal:10500
    type: mcp
    transport: streamable-http
  - id: query_rewriter
    url: http://host.docker.internal:10501
    type: mcp
    transport: streamable-http
  - id: context_builder
    url: http://host.docker.internal:10502
    type: mcp
    transport: streamable-http

listeners:
  - type: agent
    name: rag_orchestrator
    port: 8000
    router: plano_orchestrator_v1
    agents:
      - id: rag_agent
        description: Answers questions using internal knowledge base documents.
        filter_chain:
          - input_guards      # 1. Block jailbreaks and policy violations
          - query_rewriter    # 2. Normalize the safe query
          - context_builder   # 3. Retrieve relevant context for the clean query
```

Different agents within the same listener can have different filter chains — a public-facing agent may need all guards while an internal admin agent may skip them.

Reference: https://github.com/katanemo/archgw

---

## Section 5: Observability & Debugging

*OpenTelemetry tracing, log levels, span attributes, and sampling for production visibility.*

### 5.1 Add Custom Span Attributes for Correlation and Filtering

**Impact:** `MEDIUM` — Without custom span attributes, traces cannot be filtered by user, session, or environment — making production debugging significantly harder
**Tags:** `observability`, `tracing`, `span-attributes`, `correlation`

## Add Custom Span Attributes for Correlation and Filtering

Plano can automatically extract HTTP request headers and attach them as span attributes, plus attach static key-value pairs to every span. This enables filtering traces by user, session, tenant, environment, or any other dimension that matters to your application.

**Incorrect (no span attributes — traces are unfiltered blobs):**

```yaml
tracing:
  random_sampling: 20
  # No span_attributes — cannot filter by user, session, or environment
```

**Correct (rich span attributes for production correlation):**

```yaml
version: v0.3.0

tracing:
  random_sampling: 20
  trace_arch_internal: true

  span_attributes:
    # Match all headers with this prefix, then map to span attributes by:
    # 1) stripping the prefix and 2) converting hyphens to dots
    header_prefixes:
      - x-katanemo-

    # Static attributes added to every span from this Plano instance
    static:
      environment: production
      service.name: plano-gateway
      deployment.region: us-east-1
      service.version: "2.1.0"
      team: platform-engineering
```

**Sending correlation headers from client code:**

```python
import httpx

response = httpx.post(
    "http://localhost:12000/v1/chat/completions",
    headers={
        "x-katanemo-request-id": "req_abc123",
        "x-katanemo-user-id": "usr_12",
        "x-katanemo-session-id": "sess_xyz456",
        "x-katanemo-tenant-id": "acme-corp",
    },
    json={"model": "plano.v1", "messages": [...]}
)
```

**Querying by custom attribute:**

```bash
# Find all requests from a specific user
planoai trace --where user.id=usr_12

# Find all traces from production environment
planoai trace --where environment=production

# Find traces from a specific tenant
planoai trace --where tenant.id=acme-corp
```

Header prefix matching is a prefix match. With `x-katanemo-`, these mappings apply:

- `x-katanemo-user-id` -> `user.id`
- `x-katanemo-tenant-id` -> `tenant.id`
- `x-katanemo-request-id` -> `request.id`

Reference: [https://github.com/katanemo/archgw](https://github.com/katanemo/archgw)

---

### 5.2 Enable Tracing with Appropriate Sampling for Your Environment

**Impact:** `HIGH` — Without tracing enabled, debugging routing decisions, latency issues, and model selection is guesswork — traces are the primary observability primitive in Plano
**Tags:** `observability`, `tracing`, `opentelemetry`, `otel`, `debugging`

## Enable Tracing with Appropriate Sampling for Your Environment

Plano emits OpenTelemetry (OTEL) traces for every request, capturing routing decisions, LLM provider selection, filter chain execution, and response latency. Traces are the best tool for understanding why a request was routed to a particular model and debugging unexpected behavior.

**Incorrect (no tracing configured — flying blind in production):**

```yaml
version: v0.3.0

listeners:
  - type: model
    name: model_listener
    port: 12000

model_providers:
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    default: true

# No tracing block — no visibility into routing, latency, or errors
```

**Correct (tracing enabled with environment-appropriate sampling):**

```yaml
version: v0.3.0

listeners:
  - type: model
    name: model_listener
    port: 12000

model_providers:
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    default: true

tracing:
  random_sampling: 100              # 100% for development/debugging
  trace_arch_internal: true         # Include Plano's internal routing spans
```

**Production configuration (sampled to control volume):**

```yaml
tracing:
  random_sampling: 10               # Sample 10% of requests in production
  trace_arch_internal: false        # Skip internal spans to reduce noise
  span_attributes:
    header_prefixes:
      - x-katanemo-               # Match all x-katanemo-* headers
    static:
      environment: production
      service.name: my-plano-service
      version: "1.0.0"
```

With `x-katanemo-` configured, Plano maps headers to attributes by stripping the prefix and converting hyphens to dots:

- `x-katanemo-user-id` -> `user.id`
- `x-katanemo-session-id` -> `session.id`
- `x-katanemo-request-id` -> `request.id`

**Starting the trace collector:**

```bash
# Start Plano with built-in OTEL collector
planoai up config.yaml --with-tracing
```

Sampling rates: 100% for dev/staging, 5–20% for high-traffic production, 100% for low-traffic production. `trace_arch_internal: true` adds spans showing which routing preference matched — essential for debugging preference configuration.

Reference: [https://github.com/katanemo/archgw](https://github.com/katanemo/archgw)

---

### 5.3 Use `planoai trace` to Inspect Routing Decisions

**Impact:** `MEDIUM-HIGH` — The trace CLI lets you verify which model was selected, why, and how long each step took — without setting up a full OTEL backend
**Tags:** `observability`, `tracing`, `cli`, `debugging`, `routing`

## Use `planoai trace` to Inspect Routing Decisions

`planoai trace` provides a built-in trace viewer backed by an in-memory OTEL collector. Use it to inspect routing decisions, verify preference matching, measure filter latency, and debug failed requests — all from the CLI without configuring Jaeger, Zipkin, or another backend.

**Workflow: start collector, run requests, then inspect traces:**

```bash
# 1. Start Plano with the built-in trace collector (recommended)
planoai up config.yaml --with-tracing

# 2. Send test requests through Plano
curl http://localhost:12000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "plano.v1", "messages": [{"role": "user", "content": "Write a Python function to sort a list"}]}'

# 3. Show the latest trace
planoai trace
```

You can also run the trace listener directly:

```bash
planoai trace listen # available on a process ID running OTEL collector
```

Stop the background trace listener:

```bash
planoai trace down
```

**Useful trace viewer patterns:**

```bash
# Show latest trace (default target is "last")
planoai trace

# List available trace IDs
planoai trace --list

# Show all traces
planoai trace any

# Show a specific trace (short 8-char or full 32-char ID)
planoai trace 7f4e9a1c
planoai trace 7f4e9a1c0d9d4a0bb9bf5a8a7d13f62a

# Filter by specific span attributes (AND semantics for repeated --where)
planoai trace any --where llm.model=gpt-4o-mini

# Filter by user ID (if header prefix is x-katanemo-, x-katanemo-user-id maps to user.id)
planoai trace any --where user.id=user_123

# Limit results for a quick sanity check
planoai trace any --limit 5

# Time window filter
planoai trace any --since 30m

# Filter displayed attributes by key pattern
planoai trace any --filter "http.*"

# Output machine-readable JSON
planoai trace any --json
```

**What to look for in traces:**


| Span name           | What it tells you                                             |
| ------------------- | ------------------------------------------------------------- |
| `plano.routing`     | Which routing preference matched and which model was selected |
| `plano.filter.<id>` | How long each filter in the chain took                        |
| `plano.llm.request` | Time to first token and full response time                    |
| `plano.agent.route` | Which agent description matched for agent listeners           |


Reference: [https://github.com/katanemo/archgw](https://github.com/katanemo/archgw)

---

## Section 6: CLI Operations

*Using the planoai CLI for startup, tracing, CLI agents, project init, and code generation.*

### 6.1 Follow the `planoai up` Validation Workflow Before Debugging Runtime Issues

**Impact:** `HIGH` — `planoai up` validates config, checks API keys, and health-checks all listeners — skipping this diagnostic information leads to unnecessary debugging of container or network issues
**Tags:** `cli`, `startup`, `validation`, `debugging`, `workflow`

## Follow the `planoai up` Validation Workflow Before Debugging Runtime Issues

`planoai up` is the entry point for running Plano. It performs sequential checks before the container starts: schema validation, API key presence check, container startup, and health checks on all configured listener ports. Understanding what each failure stage means prevents chasing the wrong root cause.

**Validation stages and failure signals:**

```
Stage 1: Schema validation        → "config.yaml: invalid against schema"
Stage 2: API key check            → "Missing required environment variables: OPENAI_API_KEY"
Stage 3: Container start          → "Docker daemon not running" or image pull errors
Stage 4: Health check (/healthz)  → "Listener not healthy after 120s" (timeout)
```

**Development startup workflow:**

```bash
# Standard startup — config.yaml in current directory
planoai up

# Explicit config file path
planoai up my-config.yaml

# Start in foreground to see all logs immediately (great for debugging)
planoai up config.yaml --foreground

# Start with built-in OTEL trace collector
planoai up config.yaml --with-tracing

# Enable verbose logging for debugging routing decisions
LOG_LEVEL=debug planoai up config.yaml --foreground
```

**Checking what's running:**

```bash
# Stream recent logs (last N lines, then exit)
planoai logs

# Follow logs in real-time
planoai logs --follow

# Include Envoy/gateway debug messages
planoai logs --debug --follow
```

**Stopping and restarting after config changes:**

```bash
# Stop the current container
planoai down

# Restart with updated config
planoai up config.yaml
```

**Common failure patterns:**

```bash
# API key missing — check your .env file or shell environment
export OPENAI_API_KEY=sk-proj-...
planoai up config.yaml

# Health check timeout — listener port may conflict
# Check if another process uses port 12000
lsof -i :12000

# Container fails to start — verify Docker daemon is running
docker ps
```

`planoai down` fully stops and removes the Plano container. Always run `planoai down` before `planoai up` when changing config to avoid stale container state.

Reference: https://github.com/katanemo/archgw

---

### 6.2 Generate Prompt Targets from Python Functions with `planoai generate_prompt_targets`

**Impact:** `MEDIUM` — Manually writing prompt_targets YAML for existing Python APIs is error-prone — the generator introspects function signatures and produces correct YAML automatically
**Tags:** `cli`, `generate`, `prompt-targets`, `python`, `code-generation`

## Generate Prompt Targets from Python Functions with `planoai generate_prompt_targets`

`planoai generate_prompt_targets` introspects Python function signatures and docstrings to generate `prompt_targets` YAML for your Plano config. This is the fastest way to expose existing Python APIs as LLM-callable functions without manually writing the YAML schema.

**Python function requirements for generation:**
- Use simple type annotations: `int`, `float`, `bool`, `str`, `list`, `tuple`, `set`, `dict`
- Include a docstring describing what the function does (becomes the `description`)
- Complex Pydantic models must be flattened into primitive typed parameters first

**Example Python file:**

```python
# api.py

def get_stock_quote(symbol: str, exchange: str = "NYSE") -> dict:
    """Get the current stock price and trading data for a given stock symbol.

    Returns price, volume, market cap, and 24h change percentage.
    """
    # Implementation calls stock API
    pass

def get_weather_forecast(city: str, days: int = 3, units: str = "celsius") -> dict:
    """Get the weather forecast for a city.

    Returns temperature, precipitation, and conditions for the specified number of days.
    """
    pass

def search_flights(origin: str, destination: str, date: str, passengers: int = 1) -> list:
    """Search for available flights between two airports on a given date.

    Date format: YYYY-MM-DD. Returns list of flight options with prices.
    """
    pass
```

**Running the generator:**

```bash
planoai generate_prompt_targets --file api.py
```

**Generated output (add to your config.yaml):**

```yaml
prompt_targets:
  - name: get_stock_quote
    description: Get the current stock price and trading data for a given stock symbol.
    parameters:
      - name: symbol
        type: str
        required: true
      - name: exchange
        type: str
        required: false
        default: NYSE
    # Add endpoint manually:
    endpoint:
      name: stock_api
      path: /quote?symbol={symbol}&exchange={exchange}

  - name: get_weather_forecast
    description: Get the weather forecast for a city.
    parameters:
      - name: city
        type: str
        required: true
      - name: days
        type: int
        required: false
        default: 3
      - name: units
        type: str
        required: false
        default: celsius
    endpoint:
      name: weather_api
      path: /forecast?city={city}&days={days}&units={units}
```

After generation, manually add the `endpoint` blocks pointing to your actual API. The generator produces the schema; you wire in the connectivity.

Reference: https://github.com/katanemo/archgw

---

### 6.3 Use `planoai cli_agent` to Connect Claude Code Through Plano

**Impact:** `MEDIUM-HIGH` — Running Claude Code directly against provider APIs bypasses Plano's routing, observability, and guardrails — cli_agent routes all Claude Code traffic through your configured Plano instance
**Tags:** `cli`, `cli-agent`, `claude`, `coding-agent`, `integration`

## Use `planoai cli_agent` to Connect Claude Code Through Plano

`planoai cli_agent` starts a Claude Code session that routes all LLM traffic through your running Plano instance instead of directly to Anthropic. This gives you routing preferences, model aliases, tracing, and guardrails for your coding agent workflows — making Claude Code a first-class citizen of your Plano configuration.

**Prerequisites:**

```bash
# 1. Plano must be running with a model listener
planoai up config.yaml

# 2. ANTHROPIC_API_KEY must be set (Claude Code uses it for auth)
export ANTHROPIC_API_KEY=sk-ant-...
```

**Starting the CLI agent:**

```bash
# Start CLI agent using config.yaml in current directory
planoai cli_agent claude

# Use a specific config file
planoai cli_agent claude config.yaml

# Use a config in a different directory
planoai cli_agent claude --path /path/to/project
```

**Recommended config for Claude Code routing:**

```yaml
version: v0.3.0

listeners:
  - type: model
    name: claude_code_router
    port: 12000

model_providers:
  - model: anthropic/claude-sonnet-4-20250514
    access_key: $ANTHROPIC_API_KEY
    default: true
    routing_preferences:
      - name: general coding
        description: >
          Writing code, debugging, code review, explaining concepts,
          answering programming questions, general development tasks.

  - model: anthropic/claude-opus-4-6
    access_key: $ANTHROPIC_API_KEY
    routing_preferences:
      - name: complex architecture
        description: >
          System design, complex refactoring across many files,
          architectural decisions, performance optimization, security audits.

model_aliases:
  claude.fast.v1:
    target: claude-sonnet-4-20250514
  claude.smart.v1:
    target: claude-opus-4-6

tracing:
  random_sampling: 100
  trace_arch_internal: true

overrides:
  upstream_connect_timeout: "10s"
```

**What happens when cli_agent runs:**

1. Reads your config.yaml to find the model listener port
2. Configures Claude Code to use `http://localhost:<port>` as its API endpoint
3. Starts a Claude Code session in your terminal
4. All Claude Code LLM calls flow through Plano — routing, tracing, and guardrails apply

After your session, use `planoai trace` to inspect every LLM call Claude Code made, which model was selected, and why.

Reference: [https://github.com/katanemo/archgw](https://github.com/katanemo/archgw)

---

### 6.4 Use `planoai init` Templates to Bootstrap New Projects Correctly

**Impact:** `MEDIUM` — Starting from a blank config.yaml leads to missing required fields and common structural mistakes — templates provide validated, idiomatic starting points
**Tags:** `cli`, `init`, `templates`, `getting-started`, `project-setup`

## Use `planoai init` Templates to Bootstrap New Projects Correctly

`planoai init` generates a valid `config.yaml` from built-in templates. Each template demonstrates a specific Plano capability with correct structure, realistic examples, and comments. Use this instead of writing config from scratch — it ensures you start with a valid, working configuration.

**Available templates:**

| Template ID | What It Demonstrates | Best For |
|---|---|---|
| `sub_agent_orchestration` | Multi-agent routing with specialized sub-agents | Building agentic applications |
| `coding_agent_routing` | Routing preferences + model aliases for coding workflows | Claude Code and coding assistants |
| `preference_aware_routing` | Automatic LLM routing based on task type | Multi-model cost optimization |
| `filter_chain_guardrails` | Input guards, query rewrite, context builder | RAG + safety pipelines |
| `conversational_state_v1_responses` | Stateful conversations with memory | Chatbots, multi-turn assistants |

**Usage:**

```bash
# Initialize with a template
planoai init --template sub_agent_orchestration

# Initialize coding agent routing setup
planoai init --template coding_agent_routing

# Initialize a RAG with guardrails project
planoai init --template filter_chain_guardrails
```

**Typical project setup workflow:**

```bash
# 1. Create project directory
mkdir my-plano-agent && cd my-plano-agent

# 2. Bootstrap with the closest matching template
planoai init --template preference_aware_routing

# 3. Edit config.yaml to add your specific models, agents, and API keys
#    (keys are already using $VAR substitution — just set your env vars)

# 4. Create .env file for local development
cat > .env << EOF
OPENAI_API_KEY=sk-proj-...
ANTHROPIC_API_KEY=sk-ant-...
EOF

echo ".env" >> .gitignore

# 5. Start Plano
planoai up

# 6. Test your configuration
curl http://localhost:12000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "gpt-4o", "messages": [{"role": "user", "content": "Hello"}]}'
```

Start with `preference_aware_routing` for most LLM gateway use cases and `sub_agent_orchestration` for multi-agent applications. Both can be combined after you understand each independently.

Reference: https://github.com/katanemo/archgw

---

## Section 7: Deployment & Security

*Docker deployment, environment variable management, health checks, and state storage for production.*

### 7.1 Understand Plano's Docker Network Topology for Agent URL Configuration

**Impact:** `HIGH` — Using `localhost` for agent URLs inside Docker always fails — Plano runs in a container and cannot reach host services via localhost
**Tags:** `deployment`, `docker`, `networking`, `agents`, `urls`

## Understand Plano's Docker Network Topology for Agent URL Configuration

Plano runs inside a Docker container managed by `planoai up`. Services running on your host machine (agent servers, filter servers, databases) are not accessible as `localhost` from inside the container. Use Docker's special hostname `host.docker.internal` to reach host services.

**Docker network rules:**
- `localhost` / `127.0.0.1` inside the container → Plano's own container (not your host)
- `host.docker.internal` → Your host machine's loopback interface
- Container name or `docker network` hostname → Other Docker containers
- External domain / IP → Reachable if Docker has network access

**Incorrect (using localhost — agent unreachable from inside container):**

```yaml
version: v0.3.0

agents:
  - id: weather_agent
    url: http://localhost:8001       # Wrong: this is Plano's own container

  - id: flight_agent
    url: http://127.0.0.1:8002      # Wrong: same issue

filters:
  - id: input_guards
    url: http://localhost:10500      # Wrong: filter server unreachable
```

**Correct (using host.docker.internal for host-side services):**

```yaml
version: v0.3.0

agents:
  - id: weather_agent
    url: http://host.docker.internal:8001    # Correct: reaches host port 8001

  - id: flight_agent
    url: http://host.docker.internal:8002    # Correct: reaches host port 8002

filters:
  - id: input_guards
    url: http://host.docker.internal:10500   # Correct: reaches filter server on host

endpoints:
  internal_api:
    endpoint: host.docker.internal            # Correct for internal API on host
    protocol: http
```

**Production deployment patterns:**

```yaml
# Kubernetes / Docker Compose — use service names
agents:
  - id: weather_agent
    url: http://weather-service:8001    # Kubernetes service DNS

# External cloud services — use full domain
agents:
  - id: cloud_agent
    url: https://my-agent.us-east-1.amazonaws.com/v1

# Custom TLS (self-signed or internal CA)
overrides:
  upstream_tls_ca_path: /etc/ssl/certs/internal-ca.pem
```

**Ports exposed by Plano's container:**
- All `port` values from your `listeners` blocks are automatically mapped
- `9901` — Envoy admin interface (for advanced debugging)
- `12001` — Plano internal management API

Reference: https://github.com/katanemo/archgw

---

### 7.2 Use PostgreSQL State Storage for Multi-Turn Conversations in Production

**Impact:** `HIGH` — The default in-memory state storage loses all conversation history when the container restarts — production multi-turn agents require persistent PostgreSQL storage
**Tags:** `deployment`, `state`, `postgres`, `memory`, `multi-turn`, `production`

## Use PostgreSQL State Storage for Multi-Turn Conversations in Production

`state_storage` enables Plano to maintain conversation context across requests. Without it, each request is stateless. The `memory` type works for development and testing — all state is lost on container restart. Use `postgres` for any production deployment where conversation continuity matters.

**Incorrect (memory storage in production):**

```yaml
version: v0.3.0

# Memory storage — all conversations lost on planoai down / container restart
state_storage:
  type: memory

listeners:
  - type: agent
    name: customer_support
    port: 8000
    router: plano_orchestrator_v1
    agents:
      - id: support_agent
        description: Customer support assistant with conversation history.
```

**Correct (PostgreSQL for production persistence):**

```yaml
version: v0.3.0

state_storage:
  type: postgres
  connection_string: "postgresql://${DB_USER}:${DB_PASS}@${DB_HOST}:5432/${DB_NAME}"

listeners:
  - type: agent
    name: customer_support
    port: 8000
    router: plano_orchestrator_v1
    agents:
      - id: support_agent
        description: Customer support assistant with access to full conversation history.

model_providers:
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    default: true
```

**Setting up PostgreSQL for local development:**

```bash
# Start PostgreSQL with Docker
docker run -d \
  --name plano-postgres \
  -e POSTGRES_USER=plano \
  -e POSTGRES_PASSWORD=devpassword \
  -e POSTGRES_DB=plano \
  -p 5432:5432 \
  postgres:16

# Set environment variables
export DB_USER=plano
export DB_PASS=devpassword
export DB_HOST=host.docker.internal   # Use host.docker.internal from inside Plano container
export DB_NAME=plano
```

**Production `.env` pattern:**

```bash
DB_USER=plano_prod
DB_PASS=<strong-random-password>
DB_HOST=your-rds-endpoint.amazonaws.com
DB_NAME=plano
```

Plano automatically creates its state tables on first startup. The `connection_string` supports all standard PostgreSQL connection parameters including SSL: `postgresql://user:pass@host:5432/db?sslmode=require`.

Reference: https://github.com/katanemo/archgw

---

### 7.3 Verify Listener Health Before Sending Requests

**Impact:** `MEDIUM` — Sending requests to Plano before listeners are healthy results in connection refused errors that look like application bugs — always confirm health before testing
**Tags:** `deployment`, `health-checks`, `readiness`, `debugging`

## Verify Listener Health Before Sending Requests

Each Plano listener exposes a `/healthz` HTTP endpoint. `planoai up` automatically health-checks all listeners during startup (120s timeout), but in CI/CD pipelines, custom scripts, or when troubleshooting, you may need to check health manually.

**Health check endpoints:**

```bash
# Check model listener health (port from your config)
curl -f http://localhost:12000/healthz
# Returns 200 OK when healthy

# Check prompt listener
curl -f http://localhost:10000/healthz

# Check agent listener
curl -f http://localhost:8000/healthz
```

**Polling health in scripts (CI/CD pattern):**

```bash
#!/bin/bash
# wait-for-plano.sh

LISTENER_PORT=${1:-12000}
MAX_WAIT=120
INTERVAL=2
elapsed=0

echo "Waiting for Plano listener on port $LISTENER_PORT..."

until curl -sf "http://localhost:$LISTENER_PORT/healthz" > /dev/null; do
  if [ $elapsed -ge $MAX_WAIT ]; then
    echo "ERROR: Plano listener not healthy after ${MAX_WAIT}s"
    planoai logs --debug
    exit 1
  fi
  sleep $INTERVAL
  elapsed=$((elapsed + INTERVAL))
done

echo "Plano listener healthy after ${elapsed}s"
```

**Docker Compose health check:**

```yaml
# docker-compose.yml for services that depend on Plano
services:
  plano:
    image: katanemo/plano:latest
    # Plano is managed by planoai, not directly via compose in most setups
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:12000/healthz"]
      interval: 5s
      timeout: 3s
      retries: 24
      start_period: 10s

  my-agent:
    image: my-agent:latest
    depends_on:
      plano:
        condition: service_healthy
```

**Debug unhealthy listeners:**

```bash
# See startup logs
planoai logs --debug

# Check if port is already in use
lsof -i :12000

# Check container status
docker ps -a --filter name=plano

# Restart from scratch
planoai down && planoai up config.yaml --foreground
```

Reference: https://github.com/katanemo/archgw

---

## Section 8: Advanced Patterns

*Prompt targets, external API integration, rate limiting, and multi-listener architectures.*

### 8.1 Combine Multiple Listener Types for Layered Agent Architectures

**Impact:** `MEDIUM` — Using a single listener type forces all traffic through one gateway pattern — combining types lets you serve different clients with the right interface without running multiple Plano instances
**Tags:** `advanced`, `multi-listener`, `architecture`, `agent`, `model`, `prompt`

## Combine Multiple Listener Types for Layered Agent Architectures

A single Plano `config.yaml` can define multiple listeners of different types, each on a separate port. This lets you serve different client types simultaneously: an OpenAI-compatible model gateway for direct API clients, a prompt gateway for LLM-callable function applications, and an agent orchestrator for multi-agent workflows — all from one Plano instance sharing the same model providers.

**Single listener (limited — forces all clients through one interface):**

```yaml
version: v0.3.0

listeners:
  - type: model             # Only model clients can use this
    name: model_gateway
    port: 12000

# Prompt target clients and agent clients cannot connect
```

**Multi-listener architecture (serves all client types):**

```yaml
version: v0.3.0

# --- Shared model providers ---
model_providers:
  - model: openai/gpt-4o-mini
    access_key: $OPENAI_API_KEY
    default: true
    routing_preferences:
      - name: quick tasks
        description: Short answers, formatting, classification, simple generation

  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    routing_preferences:
      - name: complex reasoning
        description: Multi-step analysis, code generation, research synthesis

  - model: anthropic/claude-sonnet-4-20250514
    access_key: $ANTHROPIC_API_KEY
    routing_preferences:
      - name: long documents
        description: Summarizing or analyzing very long documents, PDFs, transcripts

# --- Listener 1: OpenAI-compatible API gateway ---
# For: SDK clients, Claude Code, LangChain, etc.
listeners:
  - type: model
    name: model_gateway
    port: 12000
    timeout: "120s"

# --- Listener 2: Prompt function gateway ---
# For: Applications that expose LLM-callable APIs
  - type: prompt
    name: function_gateway
    port: 10000
    timeout: "60s"

# --- Listener 3: Agent orchestration gateway ---
# For: Multi-agent application clients
  - type: agent
    name: agent_orchestrator
    port: 8000
    timeout: "90s"
    router: plano_orchestrator_v1
    agents:
      - id: research_agent
        description: Searches, synthesizes, and summarizes information from multiple sources.
        filter_chain:
          - input_guards
          - context_builder
      - id: code_agent
        description: Writes, reviews, debugs, and explains code across all languages.
        default: true

# --- Agents ---
agents:
  - id: research_agent
    url: http://host.docker.internal:8001
  - id: code_agent
    url: http://host.docker.internal:8002

# --- Filters ---
filters:
  - id: input_guards
    url: http://host.docker.internal:10500
    type: mcp
    transport: streamable-http
  - id: context_builder
    url: http://host.docker.internal:10501
    type: mcp
    transport: streamable-http

# --- Prompt targets (for function gateway) ---
endpoints:
  internal_api:
    endpoint: host.docker.internal
    protocol: http

prompt_targets:
  - name: search_knowledge_base
    description: Search the internal knowledge base for relevant documents and facts.
    parameters:
      - name: query
        type: str
        required: true
        description: Search query to find relevant information
    endpoint:
      name: internal_api
      path: /kb/search?q={query}
      http_method: GET

# --- Observability ---
model_aliases:
  plano.fast.v1:
    target: gpt-4o-mini
  plano.smart.v1:
    target: gpt-4o

tracing:
  random_sampling: 50
  trace_arch_internal: true
  span_attributes:
    static:
      environment: production
    header_prefixes:
      - x-katanemo-
```

This architecture serves: SDK clients on `:12000`, function-calling apps on `:10000`, and multi-agent orchestration on `:8000` — with shared cost-optimized routing across all three.

Reference: [https://github.com/katanemo/archgw](https://github.com/katanemo/archgw)

---

### 8.2 Design Prompt Targets with Precise Parameter Schemas

**Impact:** `HIGH` — Imprecise parameter definitions cause the LLM to hallucinate values, skip required fields, or produce malformed API calls — the schema is the contract between the LLM and your API
**Tags:** `advanced`, `prompt-targets`, `functions`, `llm`, `api-integration`

## Design Prompt Targets with Precise Parameter Schemas

`prompt_targets` define functions that Plano's LLM can call autonomously when it determines a user request matches the function's description. The parameter schema tells the LLM exactly what values to extract from user input — vague schemas lead to hallucinated parameters and failed API calls.

**Incorrect (too few constraints — LLM must guess):**

```yaml
prompt_targets:
  - name: get_flight_info
    description: Get flight information
    parameters:
      - name: flight         # What format? "AA123"? "AA 123"? "American 123"?
        type: str
        required: true
    endpoint:
      name: flights_api
      path: /flight?id={flight}
```

**Correct (fully specified schema with descriptions, formats, and enums):**

```yaml
version: v0.3.0

endpoints:
  flights_api:
    endpoint: api.flightaware.com
    protocol: https
    connect_timeout: "5s"

prompt_targets:
  - name: get_flight_status
    description: >
      Get real-time status, gate information, and delays for a specific flight number.
      Use when the user asks about a flight's current status, arrival time, or gate.
    parameters:
      - name: flight_number
        description: >
          IATA airline code followed by flight number, e.g., "AA123", "UA456", "DL789".
          Extract from user message — do not include spaces.
        type: str
        required: true
        format: "^[A-Z]{2}[0-9]{1,4}$"    # Regex hint for validation

      - name: date
        description: >
          Flight date in YYYY-MM-DD format. Use today's date if not specified.
        type: str
        required: false
        format: date

    endpoint:
      name: flights_api
      path: /flights/{flight_number}?date={date}
      http_method: GET
      http_headers:
        Authorization: "Bearer $FLIGHTAWARE_API_KEY"

  - name: search_flights
    description: >
      Search for available flights between two cities or airports.
      Use when the user wants to find flights, compare options, or book travel.
    parameters:
      - name: origin
        description: Departure airport IATA code (e.g., "JFK", "LAX", "ORD")
        type: str
        required: true
      - name: destination
        description: Arrival airport IATA code (e.g., "LHR", "CDG", "NRT")
        type: str
        required: true
      - name: departure_date
        description: Departure date in YYYY-MM-DD format
        type: str
        required: true
        format: date
      - name: cabin_class
        description: Preferred cabin class
        type: str
        required: false
        default: economy
        enum: [economy, premium_economy, business, first]
      - name: passengers
        description: Number of adult passengers (1-9)
        type: int
        required: false
        default: 1

    endpoint:
      name: flights_api
      path: /search?from={origin}&to={destination}&date={departure_date}&class={cabin_class}&pax={passengers}
      http_method: GET
      http_headers:
        Authorization: "Bearer $FLIGHTAWARE_API_KEY"

    system_prompt: |
      You are a travel assistant. Present flight search results clearly,
      highlighting the best value options. Include price, duration, and
      number of stops for each option.

model_providers:
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    default: true

listeners:
  - type: prompt
    name: travel_functions
    port: 10000
    timeout: "30s"
```

**Key principles:**
- `description` on the target tells the LLM when to call it — be specific about trigger conditions
- `description` on each parameter tells the LLM what value to extract — include format examples
- Use `enum` to constrain categorical values — prevents the LLM from inventing categories
- Use `format: date` or regex patterns to hint at expected format
- Use `default` for optional parameters so the API never receives null values
- `system_prompt` on the target customizes how the LLM formats the API response to the user

Reference: https://github.com/katanemo/archgw

---

*Generated from individual rule files in `rules/`.*
*To contribute, see [CONTRIBUTING](https://github.com/katanemo/archgw/blob/main/CONTRIBUTING.md).*
