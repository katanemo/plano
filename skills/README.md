# Plano Agent Skills

A structured repository of best practices for building agents and agentic applications with [Plano](https://github.com/katanemo/archgw) — the AI-native proxy and dataplane. Optimized for coding agents and LLMs.

## What Are Skills?

Skills are principle-based guides that help coding agents (Claude Code, Cursor, Copilot, etc.) make better decisions when working with Plano. They cover configuration patterns, routing strategies, agent orchestration, observability, and CLI workflows — acting as operating principles, not documentation replacements.

## Installing

```bash
# Install via npx skills
npx skills add katanemo/plano
```

This skills collection is published from the `skills/` directory in the `katanemo/plano` monorepo.

Install a specific skill:

```bash
npx skills add katanemo/plano --skill plano-routing-model-selection
```

List available skills before install:

```bash
npx skills add katanemo/plano --list
```

## Using Skills in Agents

After installation, these skills are available to your coding agent and can be invoked with normal language. You do not need special syntax unless your tooling requires it.

### Natural Language Invocation Examples

- "Use the Plano skills to validate this `config.yaml` and fix issues."
- "Apply Plano routing best practices to improve model/provider selection."
- "Review this agent listener config with the orchestration rules."
- "Refactor this filter chain to follow guardrail ordering best practices."
- "Audit this setup against Plano deployment and security recommendations."

### Prompting Tips for Better Results

- Name your goal and file: "Harden `config.yaml` for production."
- Ask for an action: "Generate a patch," "fix directly," or "explain the changes."
- Include runtime context when relevant: trace output, logs, listener errors.
- Ask for verification: "Run a final validation check after edits."

### Invoke by Skill Area (Optional)

- **Configuration:** "Use Plano configuration fundamentals on this config."
- **Routing:** "Use routing/model-selection skills to tune defaults and aliases."
- **Agent orchestration:** "Use agent orchestration skills to improve routing accuracy."
- **Filters/guardrails:** "Use filter-chain skills to harden input/output safety."
- **Observability:** "Use observability skills to add traceability and debug routing."
- **CLI/deployment:** "Use CLI and deployment skills to produce a startup checklist."

## Available Skills

- `plano-agent-skills` - Umbrella skill covering all Plano areas
- `plano-config-fundamentals` - Config versioning, listeners, providers, secrets
- `plano-routing-model-selection` - Defaults, aliases, passthrough auth, preferences
- `plano-agent-orchestration` - Agent registration and routing descriptions
- `plano-filter-guardrails` - MCP filters, guardrail messaging, filter ordering
- `plano-observability-debugging` - Tracing setup, span attributes, trace analysis
- `plano-cli-operations` - `planoai up`, `cli_agent`, init, prompt target generation
- `plano-deployment-security` - Docker networking, health checks, state storage
- `plano-advanced-patterns` - Multi-listener architecture and prompt target schema design

## Local Testing

```bash
# From repo root
npx skills add ./skills --list
npx skills add ./skills --skill plano-agent-skills -y
npx skills list
```

## Structure

```
skills/
├── rules/                    # Individual rule files (one per rule)
│   ├── _sections.md          # Section metadata and prefix definitions
│   ├── _template.md          # Template for creating new rules
│   ├── config-*.md           # Section 1: Configuration Fundamentals
│   ├── routing-*.md          # Section 2: Routing & Model Selection
│   ├── agent-*.md            # Section 3: Agent Orchestration
│   ├── filter-*.md           # Section 4: Filter Chains & Guardrails
│   ├── observe-*.md          # Section 5: Observability & Debugging
│   ├── cli-*.md              # Section 6: CLI Operations
│   ├── deploy-*.md           # Section 7: Deployment & Security
│   └── advanced-*.md         # Section 8: Advanced Patterns
├── src/
│   ├── build.ts              # Compiles rules/ into AGENTS.md
│   ├── validate.ts           # Validates rule files
│   └── extract-tests.ts      # Extracts test cases for LLM evaluation
├── metadata.json             # Document metadata
├── AGENTS.md                 # Compiled output (generated — do not edit directly)
├── test-cases.json           # Test cases for LLM evaluation (generated)
└── package.json
```

## Sections

| # | Prefix | Section | Rules |
|---|--------|---------|-------|
| 1 | `config-` | Configuration Fundamentals | Version, listeners, providers, secrets, timeouts |
| 2 | `routing-` | Routing & Model Selection | Preferences, aliases, defaults, passthrough |
| 3 | `agent-` | Agent Orchestration | Descriptions, agent registration |
| 4 | `filter-` | Filter Chains & Guardrails | Ordering, MCP integration, guardrails |
| 5 | `observe-` | Observability & Debugging | Tracing, trace inspection, span attributes |
| 6 | `cli-` | CLI Operations | Startup, CLI agent, init, code generation |
| 7 | `deploy-` | Deployment & Security | Docker networking, state storage, health checks |
| 8 | `advanced-` | Advanced Patterns | Prompt targets, rate limits, multi-listener |

## Getting Started

```bash
# Install dependencies
npm install

# Validate all rule files
npm run validate

# Build AGENTS.md from rules
npm run build

# Extract test cases for LLM evaluation
npm run extract-tests

# Run all of the above
npm run dev
```

## Creating a New Rule

1. Copy `rules/_template.md` to `rules/<prefix>-<description>.md`

2. Choose the correct prefix for your section:
   - `config-` — Configuration Fundamentals
   - `routing-` — Routing & Model Selection
   - `agent-` — Agent Orchestration
   - `filter-` — Filter Chains & Guardrails
   - `observe-` — Observability & Debugging
   - `cli-` — CLI Operations
   - `deploy-` — Deployment & Security
   - `advanced-` — Advanced Patterns

3. Fill in the frontmatter:
   ```yaml
   ---
   title: Clear, Actionable Rule Title
   impact: HIGH
   impactDescription: One-line description of why this matters
   tags: config, routing, relevant-tags
   ---
   ```

4. Write the rule body with:
   - Brief explanation of the principle and why it matters
   - **Incorrect** example (YAML config or CLI command showing the wrong pattern)
   - **Correct** example (the right pattern with comments)
   - Optional explanatory notes

5. Run `npm run dev` to validate and regenerate

## Rule File Structure

```markdown
---
title: Rule Title Here
impact: CRITICAL
impactDescription: One sentence on the impact
tags: tag1, tag2, tag3
---

## Rule Title Here

Brief explanation of the rule and why it matters for Plano developers.

**Incorrect (describe what's wrong):**

```yaml
# Bad example
```

**Correct (describe what's right):**

```yaml
# Good example with comments explaining the decisions
```

Optional explanatory text, lists, or tables.

Reference: https://github.com/katanemo/archgw



## Impact Levels

| Level | Description |
|-------|-------------|
| `CRITICAL` | Causes startup failures or silent misbehavior — always fix |
| `HIGH` | Significantly degrades routing accuracy, security, or reliability |
| `MEDIUM-HIGH` | Important for production deployments |
| `MEDIUM` | Best practice for maintainability and developer experience |
| `LOW-MEDIUM` | Incremental improvements |
| `LOW` | Nice to have |

## Key Rules at a Glance

- **Always set `version: v0.3.0`** — config is rejected without it
- **Use `host.docker.internal`** for agent/filter URLs — `localhost` doesn't work inside Docker
- **Set exactly one `default: true` provider** — unmatched requests need a fallback
- **Write specific routing preference descriptions** — vague descriptions cause misroutes
- **Order filter chains: guards → rewriters → context builders** — never build context before blocking bad input
- **Use `$VAR_NAME` for all secrets** — never hardcode API keys in config.yaml
- **Enable tracing with `--with-tracing`** — traces are the primary debugging tool

## Scripts

| Command | Description |
|---------|-------------|
| `npm run build` | Compile `rules/` into `AGENTS.md` |
| `npm run validate` | Validate all rule files for required fields and structure |
| `npm run extract-tests` | Generate `test-cases.json` for LLM evaluation |
| `npm run dev` | Validate + build + extract tests |

## Contributing

Rules are automatically sorted alphabetically by title within each section — no need to manage numbers. IDs (`1.1`, `1.2`, etc.) are assigned during build.

When adding rules:
1. Use the correct filename prefix for your section
2. Follow `_template.md` structure
3. Include clear bad/good YAML or CLI examples
4. Add relevant tags
5. Run `npm run dev` to validate and regenerate

## License

Apache-2.0 — see [LICENSE](../LICENSE)
