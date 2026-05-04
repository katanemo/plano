# Claude Code CLI as a Plano provider

This demo wires the locally installed `claude` binary as a Plano
`model_provider`. The single line under `model_providers:`

```yaml
model_providers:
  - model: claude-cli/*
    default: true
```

is enough to:

1. Auto-fill `provider_interface: claude-cli`, `base_url: http://127.0.0.1:14001`
  and a placeholder `access_key` (the CLI uses its own login keychain).
2. Start a localhost bridge inside `brightstaff` that spawns `claude -p
  --output-format stream-json --input-format stream-json` for each
   conversation.
3. Expose every Claude Code model — `claude-cli/sonnet`, `claude-cli/opus`,
  `claude-cli/haiku`, plus dated full ids — at `GET /v1/models`.

## Running

```bash
# Make sure the CLI is logged in. You can use API krey billing or a paid Claude subscription.
claude auth login

# Start Plano in native mode.
planoai up demos/integrations/claude_cli/config.yaml
```

Then point any OpenAI- or Anthropic-style client at `http://localhost:12000`
and pick any `claude-cli/...` model. Plano routes the request through Envoy
to the brightstaff bridge, which asks the local `claude` binary to handle
it.

## Optional overrides

Set these env vars before `planoai up` if you need to tweak the bridge:


| Env var                       | Default             | Meaning                                |
| ----------------------------- | ------------------- | -------------------------------------- |
| `CLAUDE_CLI_BIN`              | `claude`            | Path to the CLI binary.                |
| `CLAUDE_CLI_PERMISSION_MODE`  | `bypassPermissions` | `--permission-mode` flag value.        |
| `CLAUDE_CLI_LISTEN_ADDR`      | `127.0.0.1:14001`   | Bridge listen address.                 |
| `CLAUDE_CLI_SESSION_TTL_SECS` | `600`               | Idle TTL before a child is killed.     |
| `CLAUDE_CLI_WATCHDOG_SECS`    | `120`               | Per-line watchdog inside one CLI turn. |
| `CLAUDE_CLI_MAX_SESSIONS`     | `64`                | Hard cap on concurrent CLI children.   |
