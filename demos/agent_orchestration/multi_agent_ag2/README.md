# AG2 Multi-Agent Research Team — with Plano

**What you'll see:** A research assistant powered by AG2's multi-agent GroupChat — a team of specialized agents (researcher + analyst) collaborating behind a single Plano endpoint, with unified routing, orchestration, and observability.

## The Problem

Building multi-agent systems today forces developers to:
- **Pick one framework** - can't mix AG2, CrewAI, LangChain, or custom agents easily
- **Write plumbing code** - authentication, request routing, error handling
- **Rebuild for changes** - want to swap frameworks? Start over
- **Limited observability** - no unified view across different agent frameworks

Most demos also show one agent per endpoint. But real-world tasks need multiple agents collaborating internally — and that internal collaboration should be transparent to the orchestration layer.

## Plano + AG2 Solution

Plano acts as a **framework-agnostic proxy and data plane** that:
- Routes requests to the AG2 research team endpoint
- AG2 internally orchestrates a GroupChat (researcher → analyst)
- Provides unified tracing across the entire multi-agent flow
- Keeps internal agent collaboration transparent to the orchestration layer

## How It's Different from the CrewAI/LangChain Demo

In the [CrewAI/LangChain demo](../multi_agent_crewai_langchain/), each framework = one agent = one endpoint. Plano routes between them at the framework level.

Here, AG2 runs a **team** of agents behind a single endpoint. The researcher and analyst collaborate internally via AG2's GroupChat — Plano sees it as one agent, while the multi-agent work happens inside. This shows that Plano can orchestrate both simple agents and complex multi-agent teams equally well.

## How To Run

### Prerequisites

1. **Install uv** (Python package manager)
   ```bash
   curl -LsSf https://astral.sh/uv/install.sh | sh
   ```

2. **Install Plano CLI**
   ```bash
   uv tool install planoai
   ```

3. **Install demo dependencies** (includes [ag2] and FastAPI)
   ```bash
   cd demos/agent_orchestration/multi_agent_ag2
   uv sync
   ```

4. **Set Environment Variables**
   ```bash
   export OPENAI_API_KEY=your_key_here
   ```

### Start the Demo

```bash
# From the demo directory
cd demos/agent_orchestration/multi_agent_ag2
./run_demo.sh
```

This installs dependencies, starts Plano natively, and runs the AG2 research agent as a local process:
- **AG2 Research Agent** (port 10530) — researcher + analyst GroupChat

Plano runs natively on the host (ports 12000, 8001).

### Try It Out

1. **Using curl**
   ```bash
   curl -X POST http://localhost:8001/v1/chat/completions \
     -H "Content-Type: application/json" \
     -d '{"model": "gpt-4o", "messages": [{"role": "user", "content": "Research the current state of AI agents in production"}]}'
   ```

2. **More research queries**
   ```
   "Analyze the trade-offs between RAG and fine-tuning for enterprise LLM applications"
   "What are the key trends in multi-agent AI systems in 2025?"
   "Research the pros and cons of different vector database solutions"
   ```

   Plano routes the request to the AG2 endpoint, where the researcher and analyst collaborate internally before returning a unified response.

## Architecture

```
┌──────────────┐
│    Client    │
└──────┬───────┘
       │
       v
┌─────────────┐
│    Plano    │  (Routing & Observability)
└──────┬──────┘
       │
       v
┌──────────────────────┐
│  AG2 Research Agent  │  (Port 10530)
│  ┌────────────────┐  │
│  │   GroupChat    │  │
│  │  ┌──────────┐  │  │
│  │  │Researcher│  │  │
│  │  └────┬─────┘  │  │
│  │       │        │  │
│  │  ┌────▼─────┐  │  │
│  │  │ Analyst  │  │  │
│  │  └──────────┘  │  │
│  └────────────────┘  │
└──────────────────────┘
       │
       v
┌─────────────┐
│    Plano    │  (Proxy LLM calls)
└──────┬──────┘
       │
       v
┌─────────────┐
│   OpenAI    │
└─────────────┘
```

## AG2 Research Team

### Research Agent
- **Framework**: AG2 (formerly AutoGen)
- **Architecture**: Multi-agent GroupChat
- **Agents**:
  - **Researcher** — gathers detailed, factual information on any topic
  - **Analyst** — synthesizes findings into actionable insights and recommendations
- **Key Feature**: Internal multi-agent collaboration behind a single HTTP endpoint

## Cleanup

```bash
./run_demo.sh down
```

## Next Steps

- **Add more agents** — extend the GroupChat with a fact-checker, domain expert, or summarizer
- **Add tools** — register web search or database tools with `@agent.register_for_llm()` + `@agent.register_for_execution()`
- **Mix frameworks** — combine this AG2 team with CrewAI or LangChain agents in a single Plano config
- **Production deployment** — see [Plano docs](https://docs.planoai.dev) for scaling guidance

## Learn More

- [Plano Documentation](https://docs.planoai.dev)
- [AG2 Documentation](https://docs.ag2.ai)
