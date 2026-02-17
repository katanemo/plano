# OpenClaw + Plano: Smart Model Routing for Personal AI Assistants

OpenClaw is an open-source personal AI assistant that connects to WhatsApp, Telegram, Slack, and Discord. By pointing it at Plano instead of a single LLM provider, every message is automatically routed to the best model — conversational requests go to Kimi K2.5 (cost-effective), while code generation, testing, and complex reasoning go to Claude (most capable) — with zero application code changes.

## Architecture

```
[WhatsApp / Telegram / Slack / Discord]
                |
        [OpenClaw Gateway]
         ws://127.0.0.1:18789
                |
        [Plano :12000]  ──────────────>  Kimi K2.5  (conversation, agentic tasks)
                |                           $0.60/M input tokens
                |──────────────────────>  Claude     (code, tests, reasoning)
                |
        [Arch-Router 1.5B]
        (local via Ollama, ~200ms)
```

Plano's 1.5B [Arch-Router](https://arxiv.org/abs/2506.16655) model analyzes each prompt locally and selects the best backend based on configured routing preferences.

## Prerequisites

- **Docker** running
- **Ollama** installed ([ollama.com](https://ollama.com))
- **Plano CLI**: `uv tool install planoai` or `pip install planoai`
- **OpenClaw**: `npm install -g openclaw@latest`
- **API keys**:
  - `MOONSHOT_API_KEY` — from [Moonshot AI](https://platform.moonshot.cn/)
  - `ANTHROPIC_API_KEY` — from [Anthropic](https://console.anthropic.com/)

## Quick Start

### 1. Set Environment Variables

```bash
export MOONSHOT_API_KEY="your-moonshot-key"
export ANTHROPIC_API_KEY="your-anthropic-key"
```

### 2. Start the Demo

```bash
cd demos/llm_routing/openclaw_routing
bash run_demo.sh
```

This will:
- Pull the Arch-Router model into Ollama
- Start Jaeger for tracing
- Start Plano on port 12000

### 3. Configure OpenClaw

In `~/.openclaw/openclaw.json`, set:

```json
{
  "agent": {
    "model": "kimi-k2.5",
    "baseURL": "http://127.0.0.1:12000/v1"
  }
}
```

Then run:

```bash
openclaw onboard --install-daemon
```

### 4. Test Routing

Run the test script to verify routing decisions:

```bash
bash test_routing.sh
```

## Demo Scenarios

| # | Message | Expected Route | Why |
|---|---------|---------------|-----|
| 1 | "Hey, what's up? Tell me something interesting." | **Kimi K2.5** | General conversation — cheap and fast |
| 2 | "Remind me tomorrow at 9am and ping Slack about the deploy" | **Kimi K2.5** | Agentic multi-step task orchestration |
| 3 | "Write a Python rate limiter with the token bucket algorithm" | **Claude** | Code generation — needs precision |
| 4 | "Write unit tests for the auth middleware, cover edge cases" | **Claude** | Testing & evaluation — needs thoroughness |
| 5 | "Compare WebSockets vs SSE vs polling for 10K concurrent users" | **Claude** | Complex reasoning — needs deep analysis |

OpenClaw's code doesn't change at all. It points at `http://127.0.0.1:12000/v1` instead of a direct provider URL. Plano's Arch-Router analyzes each prompt in ~200ms and picks the right backend.

## Monitoring

### Routing Decisions

Watch Plano logs for model selection:

```bash
docker logs plano 2>&1 | grep MODEL_RESOLUTION
```

### Jaeger Tracing

Open [http://localhost:16686](http://localhost:16686) to see full traces of each request, including which model was selected and the routing latency.

## Cost Impact

For a personal assistant handling ~1000 requests/day with a 60/40 conversation-to-code split:

| Without Plano (all Claude) | With Plano (routed) |
|---|---|
| 1000 req x Claude pricing | 600 req x Kimi K2.5 + 400 req x Claude |
| ~$3.00/day input tokens | ~$0.36 + $1.20 = **$1.56/day** (~48% savings) |

Same quality where it matters (code, tests), lower cost where it doesn't (chat).

## Stopping the Demo

```bash
bash run_demo.sh down
```
