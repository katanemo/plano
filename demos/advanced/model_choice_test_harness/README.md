# Model Choice Newsletter Demo

This folder demonstrates a practical workflow for rapid model adoption and safe model switching using Plano (`plano`). It includes both a minimal test harness and a sample proxy configuration.

---

## Step-by-Step Walkthrough: Adopting New Models

### Part 1 — Testing Infrastructure

**Goal:** Quickly evaluate candidate models for a task using a repeatable, automated harness.

#### 1. Write Test Fixtures

Create a YAML file (`evals_summarize.yaml`) with real examples for your task. Each fixture includes:
- `input`: The prompt or scenario.
- `must_include`: List of anchor words that must appear in the output.
- `schema`: The expected output schema.

Example:
```yaml
# evals_summarize.yaml
task: summarize
fixtures:
  - id: sum-001
    input: "Thread about a billing dispute…"
    must_include: ["invoice"]
    schema: SummarizeOut
  - id: sum-002
    input: "Thread about a shipping delay…"
    must_include: ["status"]
    schema: SummarizeOut
```

#### 2. Candidate Models

List the model aliases (e.g., `arch.summarize.v1`, `arch.reason.v1`) you want to test. The harness will route requests through `plano`, so you don’t need provider API keys in your code.

#### 3. Minimal Python Harness

See `bench.py` for a complete example. It:
- Loads fixtures.
- Sends requests to each candidate model via `plano`.
- Validates output against schema and anchor words.
- Reports success rate and latency.

Example usage:
```sh
uv sync
python bench.py
```

**Benchmarks:**
- ≥90% schema-valid
- ≥80% anchors present
- Latency within SLO
- Cost within budget

---

### Part 2 — Network Infrastructure

**Goal:** Use a proxy server (`plano`) to decouple your app from vendor-specific model names and centralize control.

#### Why Use a Proxy?

- Consistent API across providers
- Centralized key management
- Unified logging, metrics, and guardrails
- Intent-based model aliases (e.g., `arch.summarize.v1`)
- Safe model promotions and rollbacks
- Central governance and observability

#### Example Proxy Config

See `config.yaml` for a sample configuration mapping aliases to provider models.

---

## How to Run This Demo

1. **Install uv** (if not already installed):
   ```sh
   curl -LsSf https://astral.sh/uv/install.sh | sh
   ```

2. **Install dependencies:**
  - Install all dependencies as described in the main Plano README ([link](https://github.com/katanemo/plano/?tab=readme-ov-file#prerequisites))
  - Then run
    ```sh
    uv sync
    ```

3. **Start Plano**
   ```sh
    run_demo.sh
   ```

4. **Run the test harness:**
   ```sh
   python bench.py
   ```

---

## Files in This Folder

- `bench.py` — Minimal Python test harness
- `evals_summarize.yaml` — Example test fixtures
- `pyproject.toml` — Python project configuration
- `config.yaml` — Sample plano config (if present)

---

## Troubleshooting

- If you see `Success: 0/2 (0%)`, check your anchor words and prompt clarity.
- Make sure plano is running and accessible at `http://localhost:12000/`.
- For schema validation errors, ensure your prompt instructs the model to output the correct JSON structure.
