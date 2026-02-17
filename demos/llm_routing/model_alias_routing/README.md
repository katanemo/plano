# Model Alias Demo Suite

This directory contains demos for the model alias feature in Plano.

## Overview

Model aliases allow clients to use friendly, semantic names instead of provider-specific model names. For example:
- `arch.summarize.v1` → `4o-mini` (fast, cheap model for summaries)
- `arch.reasoning.v1` → `gpt-4o` (capable model for complex reasoning)
- `creative-model` → `claude-3-5-sonnet` (creative tasks)

## Configuration

The `arch_config_with_aliases.yaml` file defines several aliases:

```yaml
# Model aliases - friendly names that map to actual provider names
model_aliases:
  # Alias for summarization tasks -> fast/cheap model
  arch.summarize.v1:
    target: gpt-4o-mini

  # Alias for general purpose tasks -> latest model
  arch.v1:
    target: o3

  # Alias for reasoning tasks -> capable model
  arch.reasoning.v1:
    target: gpt-4o

  # Alias for creative tasks -> Claude model
  arch.creative.v1:
    target: claude-3-5-sonnet-20241022

  # Alias for quick responses -> fast model
  arch.fast.v1:
    target: claude-3-haiku-20240307

  # Semantic aliases
  summary-model:
    target: gpt-4o-mini

  chat-model:
    target: gpt-4o

  creative-model:
    target: claude-3-5-sonnet-20241022
```

## Prerequisites
- Install all dependencies as described in the main Plano README ([link](https://github.com/katanemo/plano/?tab=readme-ov-file#prerequisites))
- Set your API keys in your environment:
  - `export OPENAI_API_KEY=your-openai-key`
  - `export ANTHROPIC_API_KEY=your-anthropic-key` (optional, but recommended for Anthropic tests)

## How to Run

1. Start the demo:
   ```sh
   sh run_demo.sh
   ```
   - This will create a `.env` file with your API keys (if not present).
   - Starts Plano gateway with model alias config (`arch_config_with_aliases.yaml`).

2. To stop the demo:
   ```sh
   sh run_demo.sh down
   ```
   - This will stop Plano gateway and any related services.

## Example Requests

### OpenAI client with alias `arch.summarize.v1`
```sh
curl -sS -X POST "http://localhost:12000/v1/chat/completions" \
  -H "Authorization: Bearer test-key" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "arch.summarize.v1",
    "max_tokens": 50,
    "messages": [
      { "role": "user",
        "content": "Hello, please respond with exactly: Hello from alias arch.summarize.v1!"
      }
    ]
  }' | jq .
```

### OpenAI client with alias `arch.v1`
```sh
curl -sS -X POST "http://localhost:12000/v1/chat/completions" \
  -H "Authorization: Bearer test-key" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "arch.v1",
    "max_tokens": 50,
    "messages": [
      { "role": "user",
        "content": "Hello, please respond with exactly: Hello from alias arch.v1!"
      }
    ]
  }' | jq .
```

### Anthropic client with alias `arch.summarize.v1`
```sh
curl -sS -X POST "http://localhost:12000/v1/messages" \
  -H "x-api-key: test-key" \
  -H "anthropic-version: 2023-06-01" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "arch.summarize.v1",
    "max_tokens": 50,
    "messages": [
      { "role": "user",
        "content": "Hello, please respond with exactly: Hello from alias arch.summarize.v1 via Anthropic!"
      }
    ]
  }' | jq .
```

### Anthropic client with alias `arch.v1`
```sh
curl -sS -X POST "http://localhost:12000/v1/messages" \
  -H "x-api-key: test-key" \
  -H "anthropic-version: 2023-06-01" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "arch.summarize.v1",
    "max_tokens": 50,
    "messages": [
      { "role": "user",
        "content": "Hello, please respond with exactly: Hello from alias arch.summarize.v1 via Anthropic!"
      }
    ]
  }' | jq .
```

## Notes
- The `.env` file will be created automatically if missing, with your API keys.
- If `ANTHROPIC_API_KEY` is not set, Anthropic requests will not work.
- You can add more aliases in `arch_config_with_aliases.yaml`.
- All curl examples use `jq .` for pretty-printing JSON responses.

## Troubleshooting
- Ensure your API keys are set in your environment before running the demo.
- If you see errors about missing keys, set them and re-run the script.
- For more details, see the main Plano documentation.
