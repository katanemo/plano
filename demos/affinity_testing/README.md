# Affinity Testing (OpenAI SDK)

Quick demo to verify model affinity pinning using standard OpenAI SDK calls against Plano.

## 1) Start Plano with affinity config

```bash
export OPENAI_API_KEY=<your-key>

planoai up demos/affinity_testing/config.yaml
```

`config.yaml` enables affinity cache settings:

```yaml
routing:
  session_ttl_seconds: 600
  session_max_entries: 1000
```

## 2) Run the demo script

```bash
python demos/affinity_testing/demo.py
```

The script uses this exact SDK pattern:

```python
from openai import OpenAI
import uuid

client = OpenAI(base_url="http://localhost:12000/v1", api_key="EMPTY")
affinity_id = str(uuid.uuid4())

response = client.chat.completions.create(
    model="gpt-5.2",
    messages=messages,
    extra_headers={"X-Model-Affinity": affinity_id},
)
```

## Expected behavior

- Call 1 and call 2 share the same affinity ID and should stay on the same selected model.
- Call 3 uses a new affinity ID and should be free to route independently.
