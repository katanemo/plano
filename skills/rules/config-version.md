---
title: Always Specify a Supported Config Version
impact: CRITICAL
impactDescription: Plano rejects configs with missing or unsupported version fields — the version field gates all other validation
tags: config, versioning, validation
---

## Always Specify a Supported Config Version

Every Plano `config.yaml` must include a `version` field at the top level. Plano validates configs against a versioned JSON schema — an unrecognized or missing version will cause `planoai up` to fail immediately with a schema validation error before the container starts.

**Incorrect (missing or invalid version):**

```yaml
# No version field — fails schema validation
listeners:
  - type: model
    name: model_listener
    port: 12000

model_providers:
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
```

**Correct (explicit supported version):**

```yaml
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

Use the latest supported version unless you are targeting a specific deployed Plano image. Current supported versions: `v0.1`, `v0.1.0`, `0.1-beta`, `v0.2.0`, `v0.3.0`. Prefer `v0.3.0` for all new projects.

Reference: https://github.com/katanemo/archgw/blob/main/config/plano_config_schema.yaml
