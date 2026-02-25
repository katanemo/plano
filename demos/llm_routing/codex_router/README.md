# Codex Router - Multi-Model Access with Intelligent Routing

Plano extends Codex to access multiple LLM providers through a single interface and route coding requests to the best configured model.

## Benefits

- **Single Interface**: Use Codex while routing through Plano
- **Task-Aware Routing**: Route requests based on coding task intent
- **Provider Flexibility**: Mix OpenAI, Anthropic, and local models behind one endpoint
- **Routing Transparency**: Inspect exactly which model served each request

## How It Works

Plano sits between Codex and configured providers:

```text
Your Request -> Codex -> Plano -> Selected Model -> Response
```

## Quick Start

### Prerequisites

```bash
# Install Codex CLI
npm install -g @openai/codex

# Ensure Docker is running
docker --version
```

### 1) Enter this demo directory

```bash
cd demos/llm_routing/codex_router
```

### 2) Set API keys

```bash
export OPENAI_API_KEY="your-openai-key-here"
export ANTHROPIC_API_KEY="your-anthropic-key-here"
```

### 3) Start Plano

```bash
# Install with uv (recommended)
uv tool install planoai
planoai up

# Or if already installed with uv
uvx planoai up
```

### 4) Launch Codex through Plano

```bash
planoai cli-agent codex
# Or if installed with uv:
uvx planoai cli-agent codex
```

The Codex launcher integration configures:

```bash
OPENAI_BASE_URL=http://127.0.0.1:12000/v1
OPENAI_API_KEY=test
```

If `arch.codex.default` exists in `model_aliases`, `planoai cli-agent codex` automatically starts Codex with:

```bash
codex -m arch.codex.default
```

## Monitor Routing Decisions

In a second terminal:

```bash
sh pretty_model_resolution.sh
```

This prints `MODEL_RESOLUTION` lines so you can see request model -> resolved model mappings in real time.

## Advanced Usage

### Override Codex model for a session

```bash
planoai cli-agent codex --settings='{"CODEX_MODEL":"openai/gpt-4.1-2025-04-14"}'
```

### Context window guidance

Codex works best with a large context window. Use models/configuration that support at least 64k context when possible.

## Notes

- Plano's `default: true` model is only used when a client request does not specify a model.
- If Codex sends an explicit model in requests, aliasing/routing rules decide the final upstream model.
