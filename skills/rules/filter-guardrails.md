---
title: Configure Prompt Guards with Actionable Rejection Messages
impact: MEDIUM
impactDescription: A generic or empty rejection message leaves users confused about why their request was blocked and unable to rephrase appropriately
tags: filter, guardrails, jailbreak, security, ux
---

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
