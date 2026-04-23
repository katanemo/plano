# Usage based LLM Routing
This demo shows how you can use user preferences to route user prompts to appropriate llm. See [config.yaml](config.yaml) for details on how you can define user preferences.

## How to start the demo

Make sure you have Plano CLI installed (`pip install planoai==0.4.21` or `uv tool install planoai==0.4.21`).

```bash
cd demos/llm_routing/preference_based_routing
./run_demo.sh
```

To also start AnythingLLM (chat UI) and Jaeger (tracing):

```bash
./run_demo.sh --with-ui
```

Then open AnythingLLM at http://localhost:3001/

Or start manually:

1. (Optional) Start AnythingLLM and Jaeger
```bash
docker compose up -d
```

2. Start Plano
```bash
planoai up config.yaml
```

3. Test with curl or open AnythingLLM http://localhost:3001/

## Running with local routing model (via Ollama)

By default, Plano uses a hosted Plano-Orchestrator endpoint. To self-host a routing model locally using Ollama:

1. Install [Ollama](https://ollama.ai) and pull the model:
```bash
ollama pull hf.co/katanemo/Arch-Router-1.5B.gguf:Q4_K_M
```

2. Make sure Ollama is running (`ollama serve` or the macOS app).

3. Start Plano with the local config:
```bash
planoai up plano_config_local.yaml
```

4. Test routing:
```bash
curl -s "http://localhost:12000/routing/v1/messages" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o-mini",
    "max_tokens": 1024,
    "messages": [
      {"role": "user", "content": "Create a REST API endpoint in Rust using actix-web"}
    ]
  }'
```

You should see the router select the appropriate model based on the routing preferences defined in `plano_config_local.yaml`.

# Testing out preference based routing

We have defined two routes 1. code generation and 2. code understanding

For code generation query LLM that is better suited for code generation wil handle the request,


If you look at the logs you'd see that code generation llm was selected,

```
...
2025-05-31T01:02:19.382716Z  INFO brightstaff::router::llm_router: router response: {'route': 'code_generation'}, response time: 203ms
...
```

<img width="1036" alt="image" src="https://github.com/user-attachments/assets/f923944b-ddbe-462e-9fd5-c75504adc8cf" />

Now if you ask for query related to code understanding you'd see llm that is better suited to handle code understanding in handled,

```
...
2025-05-31T01:06:33.555680Z  INFO brightstaff::router::llm_router: router response: {'route': 'code_understanding'}, response time: 327ms
...
```

<img width="1081" alt="image" src="https://github.com/user-attachments/assets/e50d167c-46a0-4e3a-ba77-e84db1bd376d" />
