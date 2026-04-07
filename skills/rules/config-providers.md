---
title: Register Model Providers with Correct Format Identifiers
impact: CRITICAL
impactDescription: Incorrect provider format causes request translation failures — Plano must know the wire format each provider expects
tags: config, model-providers, llm, api-format
---

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
