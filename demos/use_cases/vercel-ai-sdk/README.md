# Plano Demo: Next.js + AI SDK + Observability (Jaeger)

This is a **quick demo of Plano’s capabilities** as an LLM gateway:

- **Routing & model selection**: all LLM traffic goes through Plano.
- **OpenAI-compatible gateway**: the app talks to Plano using the OpenAI API shape.
- **Observability**: traces exported to **Jaeger** so you can inspect requests end-to-end.

The app also includes **tool calling with generative UI**:
- `getWeather`
- `getCurrencyExchange`

Both use open and free APIs.

## Quickstart

### 1) Start Plano + Jaeger (Docker)

From `demos/use_cases/vercel-ai-sdk/`:

```bash
docker compose up
```

- **Plano Gateway**: `http://localhost:12000/v1`
- **Jaeger UI**: `http://localhost:16686`

### 2) Point the app at Plano

Create `demos/use_cases/vercel-ai-sdk/.env.local`:

```bash
# Generate a random secret: https://generate-secret.vercel.app/32 or `openssl rand -base64 32`
AUTH_SECRET=****

# Instructions to create a Vercel Blob Store here: https://vercel.com/docs/vercel-blob
BLOB_READ_WRITE_TOKEN=****

# Instructions to create a PostgreSQL database here: https://vercel.com/docs/postgres
POSTGRES_URL=****

# Instructions to create a Redis store here:
# https://vercel.com/docs/redis
REDIS_URL=****

PLANO_BASE_URL=http://localhost:12000/v1

```



### 3) Start the Next.js app (local)

In a second terminal (same directory):

```bash
npm install --legacy-peer-deps
npm run dev
```

Now open the app at `http://localhost:3000`.

> **Note**: This repo uses fast-moving dependencies (AI SDK betas, React 19, Next.js 16). npm’s strict peer dependency resolver can fail installs; passing `--legacy-peer-deps` helps keep the install unblocked.

## What to try

- **Currency**: “Convert 100 USD to EUR”
- **Weather**: “What’s the weather in San Francisco?”

## Tracing

Open Jaeger (`http://localhost:16686`) and search traces for the Plano service to see routing + latency breakdowns.
