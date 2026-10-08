This demo shows how you can use Ollama as an upstream LLM through Plano's model gateway.

Before you can start the demo, make sure Ollama is up and running. You can use `ollama run llama3.2` to start the Llama 3.2 (3b) model locally at port `11434`.

Then start Plano with this demo's `config.yaml` and send OpenAI-compatible chat completion requests to `http://localhost:12000/v1`.
