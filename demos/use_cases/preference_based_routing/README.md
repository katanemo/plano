# Usage based LLM Routing
This demo shows how you can use user preferences to route user prompts to appropriate llm. See [config.yaml](config.yaml) for details on how you can define user preferences.

## How to start the demo

Make sure your machine is up to date with [latest version of plano]([url](https://github.com/katanemo/plano/tree/main?tab=readme-ov-file#prerequisites)). And you have activated the virtual environment.


1. start the openwebui
```bash
(venv) $ cd demos/use_cases/preference_based_routing
(venv) $ docker compose up -d
```
2. start plano in the foreground
```bash
(venv) $ planoai up --service plano --foreground
# Or if installed with uv: uvx planoai up --service plano --foreground
2025-05-30 18:00:09,953 - planoai.main - INFO - Starting plano cli version: 0.4.2
2025-05-30 18:00:09,953 - planoai.main - INFO - Validating /Users/adilhafeez/src/intelligent-prompt-gateway/demos/use_cases/preference_based_routing/config.yaml
2025-05-30 18:00:10,422 - cli.core - INFO - Starting arch gateway, image name: plano, tag: katanemo/plano:0.4.2
2025-05-30 18:00:10,662 - cli.core - INFO - plano status: running, health status: starting
2025-05-30 18:00:11,712 - cli.core - INFO - plano status: running, health status: starting
2025-05-30 18:00:12,761 - cli.core - INFO - plano is running and is healthy!
...
```

3. open openwebui http://localhost:8080/

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
