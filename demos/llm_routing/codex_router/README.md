# Codex Router - Multi-Model Access with Intelligent Routing

Plano extends Codex CLI to access multiple LLM providers through a single interface. This gives you:

1. **Access to Models**: Connect to OpenAI, Anthropic, xAI, Gemini, and local models via Ollama
2. **Intelligent Routing via Preferences for Coding Tasks**: Configure which models handle specific development tasks:
   - Code generation and implementation
   - Code understanding and analysis
   - Debugging and optimization
   - Architecture and system design

Uses a [1.5B preference-aligned router LLM](https://arxiv.org/abs/2506.16655) to automatically select the best model based on your request type.

## Benefits

- **Single Interface**: Access multiple LLM providers through the same Codex CLI
- **Task-Aware Routing**: Requests are analyzed and routed to models based on task type (code generation vs code understanding)
- **Provider Flexibility**: Add or remove providers without changing your workflow
- **Routing Transparency**: See which model handles each request and why

## Quick Start

### Prerequisites

```bash
# Install Codex CLI
npm install -g @openai/codex

# Install Plano CLI
pip install planoai
```

### Step 1: Open the Demo

```bash
git clone https://github.com/katanemo/arch.git
cd arch/demos/llm_routing/codex_router
```

### Step 2: Set API Keys

```bash
export OPENAI_API_KEY="your-openai-key-here"
export ANTHROPIC_API_KEY="your-anthropic-key-here"
export XAI_API_KEY="your-xai-key-here"
export GEMINI_API_KEY="your-gemini-key-here"
```

### Step 3: Start Plano

```bash
planoai up
# or: uvx planoai up
```

### Step 4: Launch Codex Through Plano

```bash
planoai cli-agent codex
# or: uvx planoai cli-agent codex
```

By default, `planoai cli-agent codex` starts Codex with `gpt-5.3-codex`. With this demo config:

- `code understanding` prompts are routed to `gpt-5-2025-08-07`
- `code generation` prompts are routed to `gpt-5.3-codex`

## Monitor Routing Decisions

In a second terminal:

```bash
sh pretty_model_resolution.sh
```

This shows each request model and the final model selected by Plano's router.

## Configuration Highlights

`config.yaml` demonstrates:

- OpenAI default model for Codex sessions (`gpt-5.3-codex`)
- Routing preference override for code understanding (`gpt-5-2025-08-07`)
- Additional providers (Anthropic, xAI, Gemini, Ollama local) to show cross-provider routing support

## Optional Overrides

Set a different Codex session model:

```bash
planoai cli-agent codex --settings='{"CODEX_MODEL":"gpt-5-2025-08-07"}'
```
