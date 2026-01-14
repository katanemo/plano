# RAG Agent Demo

A multi-agent RAG system demonstrating plano's agent filter chain with MCP protocol.

## Architecture

This demo consists of four components:
1. **Input Guards** (MCP filter) - Validates queries are within TechCorp's domain
2. **Query Rewriter** (MCP filter) - Rewrites user queries for better retrieval
3. **Context Builder** (MCP filter) - Retrieves relevant context from knowledge base
4. **RAG Agent** (REST) - Generates final responses based on augmented context

## Components

### Input Guards Filter (MCP)
- **Port**: 10500
- **Tool**: `input_guards`
- Validates queries are within TechCorp's domain
- Rejects queries about other companies or unrelated topics

### Query Rewrit3r Filter (MCP)
- **Port**: 10501
- **Tool**: `query_rewriter`
- Improves queries using LLM before retrieval

### Context Builder Filter (MCP)
- **Port**: 10502
- **Tool**: `context_builder`
- Augments queries with relevant passages from knowledge base

### RAG Agent (REST/OpenAI)
- **Port**: 10505
- **Endpoint**: `/v1/chat/completions`
- Generates responses using OpenAI-compatible API

## Quick Start

### 1. Start everything with Docker Compose
```bash
docker compose up --build
```

This brings up:
- Input Guards MCP server on port 10500
- Query Rewriter MCP server on port 10501
- Context Builder MCP server on port 10502
- RAG Agent REST server on port 10505
- Plano listener on port 8001 (and gateway on 12000)
- Jaeger UI for viewing traces at http://localhost:16686
- Open WebUI at http://localhost:8080 for interactive queries

> Set `OPENAI_API_KEY` in your environment before running; `LLM_GATEWAY_ENDPOINT` defaults to `http://host.docker.internal:12000/v1`.

### 2. Test the system

**Option A: Using Open WebUI (recommended)**

Navigate to http://localhost:8080 and send queries through the chat interface.

**Option B: Using curl**
```bash
curl -X POST http://localhost:8001/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o",
    "messages": [{"role": "user", "content": "What is the guaranteed uptime for TechCorp?"}]
  }'
```

## Configuration

The `config.yaml` defines how agents are connected:

```yaml
filters:
  - id: input_guards
    url: http://host.docker.internal:10500
    # type: mcp (default)
    # tool: input_guards (default - same as filter id)

  - id: query_rewriter
    url: http://host.docker.internal:10501
    # type: mcp (default)

  - id: context_builder
    url: http://host.docker.internal:10502
```

## How It Works

1. User sends request to plano listener on port 8001
2. Request passes through MCP filter chain:
   - **Input Guards** validates the query is within TechCorp's domain
   - **Query Rewriter** rewrites the query for better retrieval
   - **Context Builder** augments query with relevant knowledge base passages
3. Augmented request is forwarded to **RAG Agent** REST endpoint
4. RAG Agent generates final response using LLM

## Additional Configuration

See `config.yaml` for the complete filter chain setup. The MCP filters use default settings:
- `type: mcp` (default)
- `transport: streamable-http` (default)
- Tool name defaults to filter ID

See `sample_queries.md` for example queries to test the RAG system.

Example request:
```bash
curl -X POST http://localhost:8001/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o",
    "messages": [
      {
        "role": "user",
        "content": "What is the guaranteed uptime for TechCorp?"
      }
    ]
  }'
```
- `LLM_GATEWAY_ENDPOINT` - lpano endpoint (default: `http://localhost:12000/v1`)
- `OPENAI_API_KEY` - OpenAI API key for model providers

## Additional Resources

- See `sample_queries.md` for more example queries
- See `config.yaml` for complete configuration details
