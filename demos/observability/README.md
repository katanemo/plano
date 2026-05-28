# Plano Observability Stack

Grafana dashboard for monitoring Plano LLM gateway traffic using trace-derived metrics.

## Architecture

```
Plano (brightstaff) --OTLP gRPC--> OTEL Collector --traces--> Tempo
                                        |
                                   spanmetrics connector
                                        |
                                        v
                                   Prometheus <--- Grafana
                                        ^
                                        |
                              Envoy /stats/prometheus
```

The OTEL Collector receives traces from Plano and does two things:
1. Forwards them to Tempo for trace viewing
2. Derives Prometheus metrics (request counts, latency histograms) from spans via the **spanmetrics connector**

Prometheus also scrapes Envoy's native stats endpoint for WASM metrics like `ratelimited_rq`.

## Quick Start

### 1. Start the observability stack

```bash
cd demos/observability
docker compose up -d
```

### 2. Configure Plano to send traces to the OTEL Collector

Add or update the `tracing` section in your `plano_config.yaml`:

```yaml
tracing:
  # Sample 100% of requests (adjust for production)
  random_sampling: 100
  # Point at the OTEL Collector's OTLP gRPC port (host port 9317)
  opentracing_grpc_endpoint: http://localhost:9317
```

If Plano is running inside Docker on the same network, use the service name
and the container-internal port instead:

```yaml
tracing:
  random_sampling: 100
  opentracing_grpc_endpoint: http://otel-collector:4317
```

### 3. Restart Plano

Restart Plano so brightstaff picks up the new tracing config. Traces will flow
into the OTEL Collector, which forwards them to Tempo and generates Prometheus
metrics from span data.

### 4. Open Grafana

Navigate to http://localhost:9000 and log in with `admin` / `admin`.
The **Plano - Requests Overview** dashboard is auto-provisioned under the
"Plano" folder. Send a few requests through Plano and the panels will
start populating within ~15 seconds (the Prometheus scrape interval).

## Access

| Service        | URL                          | Credentials   |
|----------------|------------------------------|---------------|
| Grafana        | http://localhost:9000         | admin / admin |
| Tempo          | http://localhost:9200         |               |
| Prometheus     | http://localhost:9190         |               |
| OTEL Collector | http://localhost:9317 (gRPC)  |               |

The **Plano - Requests Overview** dashboard is auto-provisioned in Grafana under the "Plano" folder.

## Dashboard Panels

| Panel | Query Source | What It Shows |
|-------|-------------|---------------|
| LLM Requests/sec by Model | spanmetrics `calls_total{service_name="plano(llm)"}` by `llm_model` | Per-model request rate over time |
| Agent Requests/sec by Agent | spanmetrics `calls_total{service_name="plano(agent)"}` by `agent_id` | Per-agent invocation rate over time |
| Total Requests/sec | spanmetrics `calls_total` by service | Aggregate request rate across LLM, agent, and orchestrator |
| Rate-Limited Requests/sec | Envoy `envoy_wasmcustom_ratelimited_rq` | Global rate-limit rejections (no per-model breakdown) |
| LLM Latency p50/p95/p99 by Model | spanmetrics `duration_milliseconds_bucket` | End-to-end latency percentiles per model |
| Cumulative Request Count | spanmetrics `calls_total` | Total requests per model since start |

## Envoy Stats

For the rate-limit panel to work, Prometheus needs to scrape Envoy's admin stats endpoint.
The default config assumes Envoy's admin interface is at `host.docker.internal:9901`.
Adjust `prometheus.yaml` if your Envoy admin port differs.

## Span Attributes Used

These attributes are set by brightstaff's tracing instrumentation:

- `service.name` — `plano(llm)`, `plano(agent)`, `plano(orchestrator)`, `plano(filter)`, `plano(routing)`
- `llm.model` — model name (e.g., `gpt-4`, `claude-3-sonnet`)
- `agent_id` — agent identifier from the orchestrator
- `selection.listener` — listener that triggered agent selection
