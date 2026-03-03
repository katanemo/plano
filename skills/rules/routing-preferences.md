---
title: Write Task-Specific Routing Preference Descriptions
impact: HIGH
impactDescription: Vague preference descriptions cause Plano's internal router LLM to misclassify requests, routing expensive tasks to cheap models and vice versa
tags: routing, model-selection, preferences, llm-routing
---

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
