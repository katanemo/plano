---
title: Configure MCP Filters with Explicit Type and Transport
impact: MEDIUM
impactDescription: Omitting type and transport fields relies on defaults that may not match your MCP server's protocol implementation
tags: filter, mcp, integration, configuration
---

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
