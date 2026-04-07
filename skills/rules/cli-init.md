---
title: Use `planoai init` Templates to Bootstrap New Projects Correctly
impact: MEDIUM
impactDescription: Starting from a blank config.yaml leads to missing required fields and common structural mistakes — templates provide validated, idiomatic starting points
tags: cli, init, templates, getting-started, project-setup
---

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
