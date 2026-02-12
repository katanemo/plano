# Running Plano Natively (without Docker)

Run Plano directly on your machine — no Docker required. Envoy is auto-downloaded on first run, and WASM plugins + brightstaff are compiled from source.

## Prerequisites

- **Rust** with the `wasm32-wasip1` target:
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  rustup target add wasm32-wasip1
  ```
- **OpenSSL dev headers** (for brightstaff):
  ```bash
  # Debian/Ubuntu
  sudo apt-get install libssl-dev pkg-config

  # macOS
  brew install openssl
  ```
- **planoai CLI**:
  ```bash
  cd cli && uv sync
  ```

## Quick Start

```bash
# 1. Build WASM plugins and brightstaff from source
planoai build --native

# 2. Set your API key (or create a .env file in this directory)
export OPENAI_API_KEY="sk-..."

# 3. Start Plano
planoai up demos/native_run/config.yaml --native

# 4. Send a request
curl -s http://localhost:12000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "gpt-4o", "messages": [{"role": "user", "content": "hello"}]}'

# 5. Stop Plano
planoai down --native
```

## Commands

### `planoai build --native`

Compiles the Rust crates from source:
- WASM plugins (`prompt_gateway.wasm`, `llm_gateway.wasm`) targeting `wasm32-wasip1`
- `brightstaff` binary (native target)

Build artifacts are placed under `crates/target/`.

### `planoai up <config> --native`

Starts Plano natively:
1. Validates the config file (in-process, no Docker)
2. Downloads Envoy if not cached at `~/.plano/bin/envoy` (from [tetratelabs/archive-envoy](https://github.com/tetratelabs/archive-envoy))
3. Renders the Envoy config with local WASM plugin paths
4. Starts brightstaff and envoy as background daemons
5. Health-checks listener ports until ready

Options:
- `--foreground` — stay attached and stream logs (Ctrl+C to stop)
- `--with-tracing` — start a local OTLP trace collector

Runtime files are stored in `~/.plano/run/`:
```
~/.plano/
├── bin/
│   ├── envoy            # cached envoy binary
│   └── envoy.version    # pinned version tag
└── run/
    ├── envoy.yaml       # rendered envoy config
    ├── arch_config_rendered.yaml
    ├── plano.pid         # process IDs for shutdown
    └── logs/
        ├── envoy.log
        ├── brightstaff.log
        └── access_*.log
```

### `planoai down --native`

Sends SIGTERM to envoy and brightstaff, waits for graceful shutdown, and cleans up the PID file.

## Tracing

The demo config includes `tracing: random_sampling: 100` which enables full trace collection. To view traces:

```bash
# Start with tracing (starts an in-process OTLP collector on port 4317)
planoai up demos/native_run/config.yaml --native --with-tracing

# Send a request
curl -s http://localhost:12000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "gpt-4o", "messages": [{"role": "user", "content": "hello"}]}'

# View the last trace
planoai trace

# List all traces
planoai trace --list

# View traces as JSON
planoai trace --json
```

If your config doesn't have a `tracing` section, `--with-tracing` automatically injects `random_sampling: 100` so traces are collected without any config changes.

## Config Files

### `config.yaml` — Server-side API key

Plano injects the API key from the environment (or `.env` file). Clients don't need to send auth headers:

```yaml
version: v0.3.0
listeners:
  egress_traffic:
    port: 12000
model_providers:
  - name: openai-main
    model: openai/gpt-4o
    access_key: $OPENAI_API_KEY
tracing:
  random_sampling: 100
```

```bash
# No Authorization header needed — Plano injects it
curl -s http://localhost:12000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model": "gpt-4o", "messages": [{"role": "user", "content": "hello"}]}'
```

### `config_passthrough.yaml` — Client-side API key

Plano forwards the client's Authorization header to the provider as-is:

```yaml
version: v0.3.0
listeners:
  egress_traffic:
    port: 12000
model_providers:
  - name: openai-passthrough
    model: openai/gpt-4o
    passthrough_auth: true
```

```bash
# Client provides the key
curl -s http://localhost:12000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer sk-..." \
  -d '{"model": "gpt-4o", "messages": [{"role": "user", "content": "hello"}]}'
```

## Supported Platforms

| Platform | Architecture | Status |
|----------|-------------|--------|
| Linux    | x86_64      | Supported |
| Linux    | aarch64     | Supported |
| macOS    | Apple Silicon (arm64) | Supported |
| macOS    | Intel (x86_64) | Not available (no upstream Envoy binary) |

## Automated Demo

Run the full demo (build + start) with:

```bash
./demo.sh
```
