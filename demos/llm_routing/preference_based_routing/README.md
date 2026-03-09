# Usage based LLM Routing
This demo shows how you can use user preferences to route user prompts to appropriate llm. See [config.yaml](config.yaml) for details on how you can define user preferences.

## How to start the demo

Make sure you have Plano CLI installed (`pip install planoai` or `uv tool install planoai`).

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

1. Start Plano
```bash
planoai up config.yaml
```

2. (Optional) Start AnythingLLM and Jaeger
```bash
docker compose up -d
```

3. Test with curl or open AnythingLLM http://localhost:3001/

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
