# Claude Code → Plano → CLIProxyAPI (Codex OAuth)

This demo routes Claude Code through Plano's hosted preference router, then forwards the selected Anthropic-protocol request to a local [CLIProxyAPI](https://github.com/router-for-me/CLIProxyAPI) process backed by a Codex OAuth subscription.

It is intentionally self-contained and secret-free. The checked-in files contain no OAuth credentials or usable API keys.

## Architecture

```text
Claude Code
  │ Anthropic Messages API (/v1/messages)
  │ model aliases: fable / opus / sonnet / haiku
  ▼
Plano on 127.0.0.1:12000
  │ hosted preference classification
  ├─ diagnosis-only deep reasoning ──▶ Sol
  ├─ implementation/review/testing ─▶ Terra
  └─ small lookup/format/summary ────▶ Luna
  │ Anthropic Messages API
  ▼
CLIProxyAPI on 127.0.0.1:8317
  │ validates a local static API key
  │ owns Codex OAuth credentials and protocol translation
  ▼
Codex subscription backend
```

The boundaries are deliberate:

- **Claude Code** speaks the Anthropic Messages protocol.
- **Plano** owns hosted preference classification, model aliases, and provider selection.
- **CLIProxyAPI** owns Codex OAuth and Anthropic-to-Codex compatibility. Plano does not receive the OAuth token.
- **The upstream provider** ultimately determines model availability, context limits, quotas, and subscription policy.

## Versions tested

This path was tested with Plano 0.4.27 and CLIProxyAPI 7.2.70.

| Component   | Version     |
| ----------- | ----------- |
| Plano       | **0.4.27**  |
| CLIProxyAPI | **7.2.70**  |
| Claude Code | **2.1.209** |

CLIProxyAPI **7.2.75** was the latest release researched on 2026-07-14. It is not required by this demo; use it only after reviewing its release notes and rerunning the included tests.

## Prerequisites

- macOS with [Homebrew](https://brew.sh/) for the exact CLIProxyAPI service commands below. Other platforms can use the [official CLIProxyAPI quick start](https://help.router-for.me/introduction/quick-start).
- [uv](https://docs.astral.sh/uv/) for the pinned Plano CLI installation.
- Node.js and npm for Claude Code.
- `bash`, `curl`, and `lsof`.
- Ruby with Psych and JSON support. The launcher uses it to read `api-keys[0]` when needed and validate CLIProxyAPI's model catalog.
- A Codex-capable account whose plan and applicable terms permit this use.

## 1. Install the CLIs

```bash
brew install cliproxyapi
uv tool install 'planoai==0.4.27'
npm install -g @anthropic-ai/claude-code

planoai --version
cliproxyapi --version
claude --version
```

If `planoai` is already installed with `uv tool`, reinstall the pinned version with:

```bash
uv tool install --force 'planoai==0.4.27'
```

## 2. Configure CLIProxyAPI locally

Homebrew's default config path is `$(brew --prefix)/etc/cliproxyapi.conf`, and the launcher autodetects that path. Outside Homebrew, set `CLIPROXY_CONFIG` explicitly; the launcher does not guess a non-Homebrew config location. An already-exported `CLIPROXY_LOCAL_API_KEY` bypasses config-file key extraction.

The following creates a backup, installs the local-only template, generates a random client key, and writes the key without putting it in the repository:

```bash
CLIPROXY_CONFIG="$(brew --prefix)/etc/cliproxyapi.conf"

brew services stop cliproxyapi 2>/dev/null || true
if [[ -f "$CLIPROXY_CONFIG" ]]; then
  cp -p "$CLIPROXY_CONFIG" "${CLIPROXY_CONFIG}.backup"
fi
install -m 600 cliproxyapi.conf.example "$CLIPROXY_CONFIG"

export CLIPROXY_LOCAL_API_KEY="$(openssl rand -hex 32)"
ruby -rpsych -e '
  path = ARGV.fetch(0)
  config = Psych.safe_load_file(path, permitted_classes: [], permitted_symbols: [], aliases: false)
  config["api-keys"] = [ENV.fetch("CLIPROXY_LOCAL_API_KEY")]
  File.write(path, Psych.dump(config))
' "$CLIPROXY_CONFIG"
unset CLIPROXY_LOCAL_API_KEY
chmod 600 "$CLIPROXY_CONFIG"
```

If this machine already has a customized CLIProxyAPI config, merge these fields instead of replacing the file:

```yaml
host: 127.0.0.1
port: 8317
remote-management:
  allow-remote: false
  secret-key: ""
  disable-control-panel: true
api-keys:
  - "<random local client key>"
```

An empty `remote-management.secret-key` disables the Management API. `disable-control-panel: true` separately disables the bundled control panel.

## 3. Authenticate CLIProxyAPI with Codex OAuth

Run the upstream login flow. It opens the provider authorization page and stores the resulting credential under CLIProxyAPI's auth directory; it does not write the OAuth token into this demo.

```bash
cliproxyapi --codex-login
```

For a headless machine, consult `cliproxyapi --help` and the [CLIProxyAPI documentation](https://help.router-for.me/) for the no-browser login option.

Start the local service:

```bash
brew services start cliproxyapi
brew services list | grep cliproxyapi
lsof -nP -iTCP:8317 -sTCP:LISTEN
```

Verify authentication without printing the local key:

```bash
CLIPROXY_CONFIG="$(brew --prefix)/etc/cliproxyapi.conf"
CLIPROXY_LOCAL_API_KEY="$(ruby -rpsych -e '
  config = Psych.safe_load_file(ARGV.fetch(0), permitted_classes: [], permitted_symbols: [], aliases: false)
  print config.fetch("api-keys").fetch(0)
' "$CLIPROXY_CONFIG")"

printf 'Authorization: Bearer %s\n' "$CLIPROXY_LOCAL_API_KEY" | \
  curl -q --noproxy '*' --fail --silent --show-error \
    --header @- http://127.0.0.1:8317/v1/models
```

CLIProxyAPI accepts the Anthropic `/v1/messages` endpoint with either `Authorization: Bearer …` or `x-api-key`. The launcher uses bearer authentication between Claude Code and Plano; Plano replaces that placeholder credential with `CLIPROXY_LOCAL_API_KEY` for the local upstream.

## 4. Validate the templates

The structural validator reads YAML only. The test suite uses command mocks. Neither command starts CLIProxyAPI or Plano, contacts Plano's hosted router, or runs inference.

```bash
ruby validate_configs.rb config.yaml cliproxyapi.conf.example
bash test.sh
```

The checked-in Plano config can also be checked with Plano's repository validator from this demo directory:

```bash
bash ../../../config/validate_plano_config.sh
```

## 5. Start Plano, then launch Claude Code

The launcher is preflight-only. It never calls `planoai up` or `planoai down`, and it does not create PID files, locks, state, or background processes. Start both services explicitly before invoking it.

CLIProxyAPI should already be running from step 3. Load its local client key without printing it, then start Plano:

```bash
CLIPROXY_CONFIG="$(brew --prefix)/etc/cliproxyapi.conf"
CLIPROXY_LOCAL_API_KEY="$(ruby -rpsych -e '
  config = Psych.safe_load_file(ARGV.fetch(0), permitted_classes: [], permitted_symbols: [], aliases: false)
  print config.fetch("api-keys").fetch(0)
' "$CLIPROXY_CONFIG")"

umask 077
BIND_ADDRESS=127.0.0.1:9091 \
METRICS_BIND_ADDRESS=127.0.0.1:9092 \
CLIPROXY_LOCAL_API_KEY="$CLIPROXY_LOCAL_API_KEY" \
  planoai up config.yaml
```

`BIND_ADDRESS` and `METRICS_BIND_ADDRESS` constrain Brightstaff's native control and metrics listeners. `umask 077` protects newly created native runtime files that contain the rendered provider credential. In Plano 0.4.27 native mode, Envoy's unauthenticated admin listener is still generated on `0.0.0.0:9901`, and its config dump can expose rendered provider configuration. Treat native mode as a local development convenience, apply host firewall controls, inspect existing `~/.plano/run` permissions if Plano has already run, and rotate the local key after suspected exposure. For production, use Docker or another isolated deployment boundary with explicit port publishing, secret mounts, and no externally reachable admin surface.

After both health checks pass, launch Claude Code:

```bash
chmod +x claude-plano
CLIPROXY_CONFIG="$CLIPROXY_CONFIG" ./claude-plano
```

The launcher verifies that CLIProxyAPI is listening only on loopback, performs an authenticated `GET /v1/models`, requires the Sol, Terra, and Luna model IDs, checks Plano's `/healthz`, removes `CLIPROXY_LOCAL_API_KEY` from Claude's environment, and then replaces itself with Claude Code. It never adds `--dangerously-skip-permissions`; normal Claude Code permission controls remain active.

To install the launcher separately from the demo, copy the config and launcher, start Plano explicitly with the copied config, and point the launcher at the active services:

```bash
mkdir -p "$HOME/.config/claude-plano" "$HOME/.local/bin"
install -m 600 config.yaml "$HOME/.config/claude-plano/config.yaml"
install -m 755 claude-plano "$HOME/.local/bin/claude-plano"

umask 077
BIND_ADDRESS=127.0.0.1:9091 \
METRICS_BIND_ADDRESS=127.0.0.1:9092 \
CLIPROXY_LOCAL_API_KEY="$CLIPROXY_LOCAL_API_KEY" \
  planoai up "$HOME/.config/claude-plano/config.yaml"

CLIPROXY_CONFIG="$CLIPROXY_CONFIG" \
  "$HOME/.local/bin/claude-plano"
```

### Launcher overrides

| Variable                    | Default                                  | Purpose                                                                  |
| --------------------------- | ---------------------------------------- | ------------------------------------------------------------------------ |
| `CLIPROXY_URL`              | `http://127.0.0.1:8317`                  | CLIProxyAPI base URL and listener port                                   |
| `CLIPROXY_CONFIG`           | Homebrew config path; otherwise required | Active CLIProxyAPI config used only when the key is not already exported |
| `CLIPROXY_LOCAL_API_KEY`    | Read from `api-keys[0]` with Ruby/Psych  | Existing local client key or secret-manager injection                    |
| `PLANO_URL`                 | `http://127.0.0.1:12000`                 | Running Plano base URL                                                   |
| `PLANO_CLIENT_AUTH_TOKEN`   | `local-plano-proxy`                      | Nonsecret bearer token Claude Code sends to local Plano                  |
| `CLAUDE_PLANO_MODEL`        | `opus`                                   | Initial Claude Code alias; may be `opus[1m]`                             |
| `CLAUDE_PLANO_FABLE_MODEL`  | `claude-fable-5`                         | Full model name used to resolve the `fable` alias                        |
| `CLAUDE_PLANO_OPUS_MODEL`   | `claude-opus-4-8`                        | Full model name used to resolve `opus` and Fable's automatic fallback    |
| `CLAUDE_PLANO_SONNET_MODEL` | `claude-sonnet-5`                        | Full model name used to resolve the `sonnet` alias                       |
| `CLAUDE_PLANO_HAIKU_MODEL`  | `claude-haiku-4-5`                       | Full model name used for `haiku` and Claude Code background work         |
| `CLAUDE_BIN`                | `claude`                                 | Claude Code executable                                                   |
| `RUBY_BIN`                  | `ruby`                                   | Ruby executable                                                          |
| `CURL_BIN`                  | `curl`                                   | curl executable                                                          |
| `LSOF_BIN`                  | `lsof`                                   | lsof executable                                                          |

All arguments are forwarded to Claude Code unchanged, for example:

```bash
./claude-plano --print "Summarize this repository in five bullets."
```

## Model aliases and the context boundary

Plano model names include a protocol prefix:

- `anthropic/gpt-5.6-sol`
- `anthropic/gpt-5.6-terra`
- `anthropic/gpt-5.6-luna`

The `anthropic/` segment tells Plano to use the Anthropic Messages protocol. The public model IDs sent on the upstream wire are the unsuffixed `gpt-5.6-sol`, `gpt-5.6-terra`, and `gpt-5.6-luna` IDs.

The launcher starts Claude Code with `ANTHROPIC_MODEL=opus` by default and pins each Claude family alias to a full Claude model name. Plano then translates those full names through `model_aliases`:

| Claude Code alias | Full name seen by Plano | Plano target | Role                            |
| ----------------- | ----------------------- | ------------ | ------------------------------- |
| `fable`           | `claude-fable-5`        | Sol          | Deep diagnosis and architecture |
| `opus`            | `claude-opus-4-8`       | Terra        | Default implementation tier     |
| `sonnet`          | `claude-sonnet-5`       | Luna         | Fast general tier               |
| `haiku`           | `claude-haiku-4-5`      | Luna         | Background and small tasks      |

Keeping full Claude family names at the client boundary is deliberate. Claude Code uses `ANTHROPIC_DEFAULT_FABLE_MODEL` and `ANTHROPIC_DEFAULT_OPUS_MODEL` both for alias resolution and to recognize the Fable-to-Opus automatic fallback. In this demo, a classifier-flagged interactive Fable request can therefore fall back from Sol to Terra. Non-interactive Claude Code cannot show the fallback prompt and may end the turn with a refusal. The family variables are not a general provider failover chain: Sonnet and Haiku do not automatically fall back through the other aliases.

Claude Code also understands the optional client annotation `opus[1m]`. If desired, use it only at the Claude Code boundary:

```bash
CLAUDE_PLANO_MODEL='opus[1m]' ./claude-plano
```

Claude Code strips `[1m]` before sending the model name to the gateway. It is **not** a CLIProxyAPI or Plano model ID and must never appear in `model_providers`, `routing_preferences`, or alias targets. The annotation asks the client to budget for an extended context window; it does not create capacity in CLIProxyAPI, Codex, or the subscription backend, and this demo does **not** promise a one-million-token context. Actual behavior depends on the client version, gateway translation, selected upstream model, and account entitlement.

## Route behavior

The three descriptions are written to be mutually exclusive:

1. **`deep-technical-reasoning` → Sol**: diagnosis-only root-cause, concurrency, architecture, security, or incident analysis; excludes writing, review, and testing.
2. **`software-implementation` → Terra**: nontrivial writing, modification, refactoring, review, or testing; excludes diagnosis-only and trivial work.
3. **`quick-developer-utility` → Luna**: short lookup, simple summary, trivial formatting, or renaming; explicitly excludes debugging, architecture, security, review, testing, and implementation.

Terra is the default provider if a request reaches provider resolution without a preference selection. A hosted-classifier error is surfaced as an error; it is not silently treated as a successful route.

## Verify routing

Check local health:

```bash
printf 'Authorization: Bearer %s\n' "$CLIPROXY_LOCAL_API_KEY" | \
  curl -q --noproxy '*' --fail --silent --show-error \
    --header @- http://127.0.0.1:8317/v1/models >/dev/null
plano_status=$(curl -q --noproxy '*' --silent --show-error \
  --output /dev/null --write-out '%{http_code}' \
  http://127.0.0.1:12000/healthz)
[[ $plano_status == 200 ]]
```

Watch Plano's native logs in a second terminal:

```bash
planoai logs --debug --follow
```

Then exercise each preference:

```bash
./claude-plano --print "Diagnose the likely root cause of a distributed deadlock. Do not write or modify code."
./claude-plano --print "Implement a tested parser for this repository and update the relevant files."
./claude-plano --print "Summarize the current directory in three short bullets."
```

Verify the selected provider in Plano's logs. Do not infer routing from tone, latency, or response style.

For a direct protocol smoke test through Plano:

```bash
curl -q --noproxy '*' --fail --silent --show-error \
  http://127.0.0.1:12000/v1/messages \
  -H 'Content-Type: application/json' \
  -H 'anthropic-version: 2023-06-01' \
  -H 'Authorization: Bearer local-plano-proxy' \
  --data '{
    "model": "claude-opus-4-8",
    "max_tokens": 64,
    "messages": [{"role": "user", "content": "Reply with exactly: routed"}]
  }'
```

## Troubleshooting

### `CLIProxyAPI is not listening`

```bash
brew services list | grep cliproxyapi
brew services restart cliproxyapi
lsof -nP -iTCP:8317 -sTCP:LISTEN
```

Check CLIProxyAPI logs and rerun `cliproxyapi --codex-login` if its OAuth credential expired or was revoked.

### Ruby is unavailable

Ruby is required even when `CLIPROXY_LOCAL_API_KEY` is already set because the launcher parses and validates the JSON returned by `/v1/models`. Install Ruby or point `RUBY_BIN` at a compatible executable.

### `CLIProxyAPI listener must be loopback-only`

Inspect the active socket:

```bash
lsof -nP -iTCP:8317 -sTCP:LISTEN
```

A `*:8317`, `0.0.0.0:8317`, or non-loopback address is rejected. Set `host: 127.0.0.1` in the active CLIProxyAPI config and restart the service.

### `Plano health check failed`

The launcher does not start or restart Plano. Start it explicitly with the command in step 5, then verify:

```bash
plano_status=$(curl -q --noproxy '*' --silent --show-error \
  --output /dev/null --write-out '%{http_code}' \
  http://127.0.0.1:12000/healthz)
[[ $plano_status == 200 ]]
```

### `model not found` or an unexpected tier

- Confirm `/v1/models` on CLIProxyAPI includes the required GPT tier.
- Keep `[1m]` out of the Plano and CLIProxyAPI model IDs.
- Run `ruby validate_configs.rb config.yaml cliproxyapi.conf.example`.
- Confirm `CLIPROXY_CONFIG` points at the same config used by the running service.
- Inspect `planoai logs --debug --follow` for the selected provider and upstream response.

### Claude Code asks for a login

Launch through `claude-plano`, which sets both `ANTHROPIC_BASE_URL` and `ANTHROPIC_AUTH_TOKEN` before Claude Code starts. A base URL by itself is not a credential.

### Hosted routing is unavailable

Use one of the bypass options below. The local CLIProxyAPI service and its OAuth state are independent of Plano's hosted classifier.

## Rollback and direct bypass

Stop Plano without changing CLIProxyAPI:

```bash
planoai down
```

Bypass Plano and send Claude Code directly to local CLIProxyAPI:

```bash
CLIPROXY_LOCAL_API_KEY='<local key from your secret store>'
ANTHROPIC_BASE_URL=http://127.0.0.1:8317 \
ANTHROPIC_AUTH_TOKEN="$CLIPROXY_LOCAL_API_KEY" \
ANTHROPIC_MODEL=gpt-5.6-terra \
ANTHROPIC_DEFAULT_FABLE_MODEL=gpt-5.6-sol \
ANTHROPIC_DEFAULT_OPUS_MODEL=gpt-5.6-terra \
ANTHROPIC_DEFAULT_SONNET_MODEL=gpt-5.6-luna \
ANTHROPIC_DEFAULT_HAIKU_MODEL=gpt-5.6-luna \
  claude
```

This bypass uses CLIProxyAPI's real model IDs because Plano's aliases are no longer in the path. The family variables still define Claude Code's `fable`, `opus`, `sonnet`, and `haiku` selections; they also let the client recognize the direct Sol-to-Terra Fable fallback.

To bypass both local gateways and return to Claude Code's normal configured provider, start a clean shell or unset the gateway variables:

```bash
unset ANTHROPIC_BASE_URL ANTHROPIC_AUTH_TOKEN ANTHROPIC_MODEL
unset ANTHROPIC_DEFAULT_FABLE_MODEL ANTHROPIC_DEFAULT_OPUS_MODEL
unset ANTHROPIC_DEFAULT_SONNET_MODEL ANTHROPIC_DEFAULT_HAIKU_MODEL
claude
```

## Privacy, terms, and security

- Request content traverses local Plano and local CLIProxyAPI. The hosted Plano preference service receives the conversational text needed to classify the route. Review Plano's current privacy, retention, and service terms before using sensitive material.
- The selected Codex backend receives the request content. Use the OAuth integration only where the account plan, provider terms, organizational policy, and applicable law permit it.
- CLIProxyAPI credentials are stored in its auth directory. Keep that directory and the active config owner-readable only, back them up carefully, and rotate credentials after suspected exposure.
- Keep the CLIProxyAPI and Plano data listeners on loopback. Do not publish ports `8317`, `12000`, `9091`, or `9092` to a LAN or the internet. Native Plano's Envoy admin listener on `9901` is the caveat described in step 5.
- The local API key is a bearer credential. Never commit it, pass it on a command line that exposes process arguments, enable shell tracing around it, or paste it into logs.
- The launcher passes the key to curl through standard input, uses it only for the authenticated preflight, and unsets `CLIPROXY_LOCAL_API_KEY` before executing Claude Code.
- CLIProxyAPI is MIT-licensed. This demo links to the upstream project and documentation rather than copying its source or license.

## Docker and production hardening

This local developer demo is not a production deployment recipe. For production, run Plano and CLIProxyAPI in separately hardened containers or hosts, pin versions or image digests, run as non-root, use read-only root filesystems where possible, drop Linux capabilities, apply CPU/memory/process limits, isolate networks, mount credentials read-only from a secret manager, redact request logs, and add health/readiness monitoring.

Remember that `127.0.0.1` is container-local: publishing a container port can still expose it externally. Avoid publishing the CLIProxyAPI management surface, keep management disabled, and add authenticated TLS at any boundary that is not strictly local. Reassess hosted-routing privacy, account terms, retention, and failure behavior before handling production or regulated data.

## Upstream references

- [Plano documentation](https://docs.planoai.dev/)
- [Plano Claude Code router demo](../claude_code_router/)
- [CLIProxyAPI repository](https://github.com/router-for-me/CLIProxyAPI) — MIT
- [CLIProxyAPI documentation](https://help.router-for.me/)
- [CLIProxyAPI quick start](https://help.router-for.me/introduction/quick-start)
- [CLIProxyAPI Claude Code client guide](https://help.router-for.me/agent-client/claude-code)
- [Claude Code model configuration](https://code.claude.com/docs/en/model-config)
- [Claude Code LLM gateway connection guide](https://code.claude.com/docs/en/llm-gateway-connect)
