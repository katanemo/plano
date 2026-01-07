
### Use Arch for (Model-based) LLM Routing Step 1. Create arch config file
Create `config.yaml` file with following content:

```yaml
version: v0.1.0

listeners:
  egress_traffic:
    address: 0.0.0.0
    port: 12000
    message_format: openai
    timeout: 30s

llm_providers:
  - access_key: $OPENAI_API_KEY
    model: openai/gpt-4o
    default: true

  - access_key: $MISTRAL_API_KEY
    model: mistral/ministral-3b-latest
```

### Step 2. Start arch gateway

Once the config file is created ensure that you have env vars setup for `MISTRAL_API_KEY` and `OPENAI_API_KEY` (or these are defined in `.env` file).

Start arch gateway,

```
$ planoai up config.yaml
# Or if installed with uv: uvx planoai up config.yaml
2024-12-05 11:24:51,288 - planoai.main - INFO - Starting plano cli version: 0.4.2
2024-12-05 11:24:51,825 - planoai.utils - INFO - Schema validation successful!
2024-12-05 11:24:51,825 - planoai.main - INFO - Starting arch model server and arch gateway
...
2024-12-05 11:25:16,131 - planoai.core - INFO - Container is healthy!
```

### Step 3: Interact with LLM

#### Step 3.1: Using OpenAI python client

Make outbound calls via Arch gateway

```python
from openai import OpenAI

# Use the OpenAI client as usual
client = OpenAI(
  # No need to set a specific openai.api_key since it's configured in Arch's gateway
  api_key = '--',
  # Set the OpenAI API base URL to the Arch gateway endpoint
  base_url = "http://127.0.0.1:12000/v1"
)

response = client.chat.completions.create(
    # we select model from arch_config file
    model="None",
    messages=[{"role": "user", "content": "What is the capital of France?"}],
)

print("OpenAI Response:", response.choices[0].message.content)

```

#### Step 3.2: Using curl command
```
$ curl --header 'Content-Type: application/json' \
  --data '{"messages": [{"role": "user","content": "What is the capital of France?"}], "model": "none"}' \
  http://localhost:12000/v1/chat/completions

{
  ...
  "model": "gpt-4o-2024-08-06",
  "choices": [
    {
      ...
      "messages": {
        "role": "assistant",
        "content": "The capital of France is Paris.",
      },
    }
  ],
...
}

```

You can override model selection using `x-arch-llm-provider-hint` header. For example if you want to use mistral using following curl command,

```
$ curl --header 'Content-Type: application/json' \
  --header 'x-arch-llm-provider-hint: ministral-3b' \
  --data '{"messages": [{"role": "user","content": "What is the capital of France?"}], "model": "none"}' \
  http://localhost:12000/v1/chat/completions
{
  ...
  "model": "ministral-3b-latest",
  "choices": [
    {
      "messages": {
        "role": "assistant",
        "content": "The capital of France is Paris. It is the most populous city in France and is known for its iconic landmarks such as the Eiffel Tower, the Louvre Museum, and Notre-Dame Cathedral. Paris is also a major global center for art, fashion, gastronomy, and culture.",
      },
      ...
    }
  ],
  ...
}

```
