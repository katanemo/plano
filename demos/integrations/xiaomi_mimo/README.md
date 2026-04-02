# Xiaomi MiMo via Plano

This demo configures Plano to call Xiaomi MiMo as a standard LLM upstream using the OpenAI-compatible API surface.

## Prerequisites

1. Ensure the [prerequisites](https://github.com/katanemo/arch/?tab=readme-ov-file#prerequisites) are installed correctly.
2. Export your MiMo API key:

```sh
export MIMO_API_KEY=your_mimo_api_key
```

## Start the demo

```sh
sh run_demo.sh
```

Plano will start a model listener on `http://localhost:12000`.

## First API call through Plano

```sh
curl --location --request POST 'http://localhost:12000/v1/chat/completions' \
  --header "Content-Type: application/json" \
  --data-raw '{
    "model": "mimo-v2-pro",
    "messages": [
      {
        "role": "system",
        "content": "You are MiMo, an AI assistant developed by Xiaomi. Today is Tuesday, December 16, 2025. Your knowledge cutoff date is December 2024."
      },
      {
        "role": "user",
        "content": "please introduce yourself"
      }
    ],
    "max_completion_tokens": 1024,
    "temperature": 1.0,
    "top_p": 0.95,
    "stream": false
  }'
```

## Optional: OpenAI Python SDK against Plano

```python
from openai import OpenAI

client = OpenAI(
    api_key="unused-when-calling-plano",
    base_url="http://localhost:12000/v1",
)

resp = client.chat.completions.create(
    model="mimo-v2-pro",
    messages=[{"role": "user", "content": "please introduce yourself"}],
)

print(resp.model_dump_json(indent=2))
```

## Stop the demo

```sh
sh run_demo.sh down
```
