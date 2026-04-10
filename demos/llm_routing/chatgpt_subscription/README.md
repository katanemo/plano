# ChatGPT Subscription Routing

Route requests through your ChatGPT Plus/Pro subscription using Plano. Uses the OpenAI Responses API under the hood, targeting `chatgpt.com/backend-api/codex/responses`.

## Setup

### 1. Authenticate with ChatGPT

```bash
planoai chatgpt login
```

This opens a device code flow — visit the URL shown and enter the code. Tokens are saved to `~/.plano/chatgpt/auth.json`.

### 2. Start Plano

```bash
planoai up config.yaml
```

### 3. Send a request

```bash
curl http://localhost:12000/v1/responses \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-5.2",
    "input": "Hello, what model are you?"
  }'
```

Or use the test script:

```bash
bash test_chatgpt.sh
```

## How it works

- `chatgpt/gpt-5.2` in the config tells Plano to use the ChatGPT subscription provider
- Plano reads OAuth tokens from `~/.plano/chatgpt/auth.json` (auto-refreshes if expired)
- Requests are proxied to `https://chatgpt.com/backend-api/codex/responses` with the required headers:
  - `Authorization: Bearer <access_token>`
  - `ChatGPT-Account-Id: <account_id>`
  - `originator: codex_cli_rs`
  - `session_id: <uuid>`

## Available models

```
chatgpt/gpt-5.4
chatgpt/gpt-5.3-codex
chatgpt/gpt-5.2
```

## Managing credentials

```bash
planoai chatgpt status   # Check auth status
planoai chatgpt logout   # Remove stored credentials
```
