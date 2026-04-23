---
title: Use Environment Variable Substitution for All Secrets
impact: CRITICAL
impactDescription: Hardcoded API keys in config.yaml will be committed to version control and exposed in Docker container inspect output
tags: config, security, secrets, api-keys, environment-variables
---

## Use Environment Variable Substitution for All Secrets

Plano supports `$VAR_NAME` substitution in config values. This applies to `access_key` fields, `connection_string` for state storage, and `http_headers` in prompt targets and endpoints. Never hardcode credentials — Plano reads them from environment variables or a `.env` file at startup via `planoai up`.

**Incorrect (hardcoded secrets):**

```yaml
version: v0.3.0

model_providers:
  - model: openai/gpt-4o
    access_key: abcdefghijklmnopqrstuvwxyz...   # Hardcoded — never do this

state_storage:
  type: postgres
  connection_string: "postgresql://admin:mysecretpassword@prod-db:5432/plano"

prompt_targets:
  - name: get_data
    endpoint:
      name: my_api
      http_headers:
        Authorization: "Bearer abcdefghijklmnopqrstuvwxyz"   # Hardcoded token
```

**Correct (environment variable substitution):**

```yaml
version: v0.3.0

model_providers:
  - model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
    default: true

  - model: anthropic/claude-sonnet-4-20250514
    access_key: $ANTHROPIC_API_KEY

state_storage:
  type: postgres
  connection_string: "postgresql://${DB_USER}:${DB_PASS}@${DB_HOST}:5432/${DB_NAME}"

prompt_targets:
  - name: get_data
    endpoint:
      name: my_api
      http_headers:
        Authorization: "Bearer $MY_API_TOKEN"
```

**`.env` file pattern (loaded automatically by `planoai up`):**

```bash
# .env — add to .gitignore
OPENAI_API_KEY=sk-proj-...
ANTHROPIC_API_KEY=sk-ant-...
DB_USER=plano
DB_PASS=secure-password
DB_HOST=localhost
MY_API_TOKEN=tok_live_...
```

Plano also accepts keys set directly in the shell environment. Variables referenced in config but not found at startup cause `planoai up` to fail with a clear error listing the missing keys.

Reference: https://github.com/katanemo/archgw
