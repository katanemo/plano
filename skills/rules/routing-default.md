---
title: Always Set Exactly One Default Model Provider
impact: HIGH
impactDescription: Without a default provider, Plano has no fallback when routing preferences do not match — requests with unclassified intent will fail
tags: routing, defaults, model-providers, reliability
---

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
