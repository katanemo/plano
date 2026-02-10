# ADR 003: Single Container with Supervisord

**Status:** Accepted

## Context

Plano has three runtime processes:
1. **Envoy Proxy** — the data plane with WASM filters
2. **Brightstaff** — the Rust HTTP service for routing and orchestration
3. **Config generator** — Python script that validates config and renders Envoy's YAML (runs at startup)

The options for deployment were:
1. **Separate containers** — each process in its own container, orchestrated by Docker Compose / K8s
2. **Single container with process manager** — all processes in one container, managed by Supervisord
3. **Single binary** — embed Envoy or reimplement its core functionality

## Decision

Run all processes in a **single container** managed by **Supervisord**. The startup sequence:
1. Config generator validates `arch_config.yaml` and renders `envoy.yaml`
2. Supervisord starts Brightstaff and Envoy in parallel
3. A log tail process unifies access log output

## Consequences

**Enables:**
- Simple deployment: one container, one image, `docker run` just works
- No network latency between Envoy and Brightstaff (localhost communication)
- Config generation happens at container startup — no external config rendering step
- Easy development: `docker compose up` with volume mounts for hot-reload

**Requires:**
- Supervisord configuration (`config/supervisord.conf`) to manage process lifecycle
- Health checks must account for both Envoy and Brightstaff readiness
- Logs from all processes need unified output (handled by the tail process)

**Prevents:**
- Independent scaling of Envoy vs. Brightstaff (they scale together as one unit)
- Kubernetes sidecar pattern (though this could be reconsidered)
- Process-level fault isolation (though Supervisord restarts failed processes)

**Trade-off:** Simplicity of deployment over horizontal scaling flexibility. For a gateway that needs to be deployed at the edge or as a sidecar, single-container simplicity is more valuable than the ability to scale components independently.
