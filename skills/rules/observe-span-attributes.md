---
title: Add Custom Span Attributes for Correlation and Filtering
impact: MEDIUM
impactDescription: Without custom span attributes, traces cannot be filtered by user, session, or environment — making production debugging significantly harder
tags: observability, tracing, span-attributes, correlation
---

## Add Custom Span Attributes for Correlation and Filtering

Plano can automatically extract HTTP request headers and attach them as span attributes, plus attach static key-value pairs to every span. This enables filtering traces by user, session, tenant, environment, or any other dimension that matters to your application.

**Incorrect (no span attributes — traces are unfiltered blobs):**

```yaml
tracing:
  random_sampling: 20
  # No span_attributes — cannot filter by user, session, or environment
```

**Correct (rich span attributes for production correlation):**

```yaml
version: v0.3.0

tracing:
  random_sampling: 20
  trace_arch_internal: true

  span_attributes:
    # Match all headers with this prefix, then map to span attributes by:
    # 1) stripping the prefix and 2) converting hyphens to dots
    header_prefixes:
      - x-katanemo-

    # Static attributes added to every span from this Plano instance
    static:
      environment: production
      service.name: plano-gateway
      deployment.region: us-east-1
      service.version: "2.1.0"
      team: platform-engineering
```

**Sending correlation headers from client code:**

```python
import httpx

response = httpx.post(
    "http://localhost:12000/v1/chat/completions",
    headers={
        "x-katanemo-request-id": "req_abc123",
        "x-katanemo-user-id": "usr_12",
        "x-katanemo-session-id": "sess_xyz456",
        "x-katanemo-tenant-id": "acme-corp",
    },
    json={"model": "plano.v1", "messages": [...]}
)
```

**Querying by custom attribute:**

```bash
# Find all requests from a specific user
planoai trace --where user.id=usr_12

# Find all traces from production environment
planoai trace --where environment=production

# Find traces from a specific tenant
planoai trace --where tenant.id=acme-corp
```

Header prefix matching is a prefix match. With `x-katanemo-`, these mappings apply:

- `x-katanemo-user-id` -> `user.id`
- `x-katanemo-tenant-id` -> `tenant.id`
- `x-katanemo-request-id` -> `request.id`

Reference: [https://github.com/katanemo/archgw](https://github.com/katanemo/archgw)
