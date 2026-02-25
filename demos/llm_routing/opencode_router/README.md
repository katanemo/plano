# OpenCode Router - Multi-Model Access with Intelligent Routing

Plano extends OpenCode to access multiple LLM providers through a single interface and route coding requests to the best configured model.

## Benefits

- **Single Interface**: Use OpenCode while routing through Plano
- **Task-Aware Routing**: Route requests based on coding task intent
- **Provider Flexibility**: Mix OpenAI, Anthropic, and local models behind one endpoint
- **Routing Transparency**: Inspect exactly which model served each request

## How It Works

Plano sits between OpenCode and configured providers:

```text
Your Request -> OpenCode -> Plano -> Selected Model -> Response
```

## Quick Start

### Prerequisites

- OpenCode CLI installed and available on your `PATH` (`opencode` command)
- Docker running

### 1) Enter this demo directory

```bash
cd demos/llm_routing/opencode_router
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

### 4) Launch OpenCode through Plano

```bash
planoai cli-agent opencode
# Or if installed with uv:
uvx planoai cli-agent opencode
```

The OpenCode launcher integration configures:

```bash
OPENAI_BASE_URL=http://127.0.0.1:12000/v1
OPENAI_API_KEY=test
```

If `arch.opencode.default` exists in `model_aliases`, `planoai cli-agent opencode` exports:

```bash
OPENAI_MODEL=<target-from-arch.opencode.default>
```

## Monitor Routing Decisions

In a second terminal:

```bash
sh pretty_model_resolution.sh
```

This prints `MODEL_RESOLUTION` lines so you can see request model -> resolved model mappings in real time.

## Advanced Usage

### Override OpenCode model for a session

```bash
planoai cli-agent opencode --settings='{"OPENCODE_MODEL":"openai/gpt-4.1-2025-04-14"}'
```

### Context window guidance

OpenCode works best with a large context window. Use models/configuration that support at least 64k context when possible.

## Notes

- Plano's `default: true` model is only used when a client request does not specify a model.
- If OpenCode sends an explicit model in requests, aliasing/routing rules decide the final upstream model.
