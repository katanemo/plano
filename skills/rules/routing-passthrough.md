---
title: Use Passthrough Auth for Proxy and Multi-Tenant Setups
impact: MEDIUM
impactDescription: Without passthrough auth, self-hosted proxy services (LiteLLM, vLLM, etc.) reject Plano's requests because the wrong Authorization header is sent
tags: routing, authentication, proxy, litellm, multi-tenant
---

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
