# ADR 001: Envoy as the Data Plane

**Status:** Accepted

## Context

Plano needs to proxy all traffic between clients, LLM providers, and developer APIs. The options were:
1. Build a custom proxy from scratch in Rust (e.g., using `hyper`/`axum` directly)
2. Use an existing L7 proxy (Envoy, NGINX, HAProxy) and extend it
3. Use a service mesh sidecar approach

We need: TLS termination, connection pooling, retry policies, load balancing, header-based routing, streaming support (SSE), compression, and observability — all at production quality.

## Decision

Use **Envoy Proxy** as the data plane. All external traffic — both inbound client requests and outbound LLM/API calls — flows through Envoy. The native Rust service (Brightstaff) never makes direct outbound connections to external hosts.

## Consequences

**Enables:**
- Production-grade L7 proxying (TLS, HTTP/2, connection pooling, retries) without building it ourselves
- WASM filter extension model for inline request/response processing
- Standard observability (access logs, stats, tracing) out of the box
- Header-based routing via Envoy's route configuration — no custom routing code needed for cluster selection
- Hot-restart and graceful draining for zero-downtime updates

**Requires:**
- All Brightstaff external calls must go through Envoy listeners (localhost:12001 for LLMs, localhost:11000 for APIs)
- Custom headers (`x-arch-*`) for routing decisions — Envoy matches on these in its route config
- Envoy configuration must be generated from user config (Jinja2 template → envoy.yaml)
- Team must understand Envoy's configuration model (listeners, clusters, filter chains)

**Prevents:**
- Direct HTTP calls from Brightstaff to external services (this is intentional — it ensures all traffic gets WASM filter processing, auth injection, rate limiting, and observability)
- Simple single-binary deployment (we need Envoy + Brightstaff, managed by Supervisord)
