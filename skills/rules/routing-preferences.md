---
title: Write Task-Specific Routing Preference Descriptions
impact: HIGH
impactDescription: Vague preference descriptions cause Plano's internal router LLM to misclassify requests, routing expensive tasks to cheap models and vice versa
tags: routing, model-selection, preferences, llm-routing
---

## Write Task-Specific Routing Preference Descriptions

Plano's `plano_orchestrator_v1` router uses a 1.5B preference-aligned LLM to classify incoming requests against your `routing_preferences` descriptions. It returns an ordered `models` list for the matched route; the client uses `models[0]` as primary and falls back to `models[1]`, `models[2]`... on `429`/`5xx` errors. Description quality directly determines routing accuracy.

Starting in `v0.4.0`, `routing_preferences` lives at the **top level** of the config and each entry carries its own `models: [...]` candidate pool. Configs still using the legacy v0.3.0 inline shape (under each `model_provider`) are auto-migrated with a deprecation warning — prefer the top-level form below.

**Incorrect (vague, overlapping descriptions):**

```yaml
version: v0.4.0

model_providers:
  - model: openai/gpt-4o-mini
    access_key: $OPENAI_API_KEY
    default: true

  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY

routing_preferences:
  - name: simple
    description: easy tasks      # Too vague — what is "easy"?
    models:
      - openai/gpt-4o-mini
  - name: hard
    description: hard tasks      # Too vague — overlaps with "easy"
    models:
      - openai/gpt-4o
```

**Correct (specific, distinct task descriptions, multi-model fallbacks):**

```yaml
version: v0.4.0

model_providers:
  - model: openai/gpt-4o-mini
    access_key: $OPENAI_API_KEY
    default: true

  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY

  - model: anthropic/claude-sonnet-4-5
    access_key: $ANTHROPIC_API_KEY

routing_preferences:
  - name: summarization
    description: >
      Summarizing documents, articles, emails, or meeting transcripts.
      Extracting key points, generating TL;DR sections, condensing long text.
    models:
      - openai/gpt-4o-mini
      - openai/gpt-4o
  - name: classification
    description: >
      Categorizing inputs, sentiment analysis, spam detection,
      intent classification, labeling structured data fields.
    models:
      - openai/gpt-4o-mini
  - name: translation
    description: >
      Translating text between languages, localization tasks.
    models:
      - openai/gpt-4o-mini
      - anthropic/claude-sonnet-4-5
  - name: code_generation
    description: >
      Writing new functions, classes, or modules from scratch.
      Implementing algorithms, boilerplate generation, API integrations.
    models:
      - openai/gpt-4o
      - anthropic/claude-sonnet-4-5
  - name: code_review
    description: >
      Reviewing code for bugs, security vulnerabilities, performance issues.
      Suggesting refactors, explaining complex code, debugging errors.
    models:
      - anthropic/claude-sonnet-4-5
      - openai/gpt-4o
  - name: complex_reasoning
    description: >
      Multi-step math problems, logical deduction, strategic planning,
      research synthesis requiring chain-of-thought reasoning.
    models:
      - openai/gpt-4o
      - anthropic/claude-sonnet-4-5
```

**Key principles for good preference descriptions:**
- Use concrete action verbs: "writing", "reviewing", "translating", "summarizing"
- List 3–5 specific sub-tasks or synonyms for each preference
- Ensure preferences across routes are mutually exclusive in scope
- Order `models` from most preferred to least — the client will fall back in order on `429`/`5xx`
- List multiple models under one route to get automatic provider fallback without additional client logic
- Every model listed in `models` must be declared in `model_providers`
- Test with representative queries using `planoai trace` and `--where` filters to verify routing decisions

Reference: [Routing API](../../docs/routing-api.md) · https://github.com/katanemo/archgw
