---
title: Use Model Aliases for Semantic, Stable Model References
impact: MEDIUM
impactDescription: Hardcoded model names in client code require code changes when you swap providers; aliases let you update routing in config.yaml alone
tags: routing, model-aliases, maintainability, client-integration
---

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
