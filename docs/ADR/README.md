# Architecture Decision Records

This directory contains Architecture Decision Records (ADRs) for the Plano project. ADRs document key architectural decisions, their context, and rationale — preventing future contributors (human or AI) from unknowingly reversing deliberate choices.

## Index

| ADR | Title | Status |
|-----|-------|--------|
| [001](001-envoy-as-data-plane.md) | Envoy as the Data Plane | Accepted |
| [002](002-wasm-filters-over-native.md) | WASM Filters Over Native Envoy Filters | Accepted |
| [003](003-single-container-supervisord.md) | Single Container with Supervisord | Accepted |
| [004](004-hermesllm-pure-rust.md) | hermesllm as a Pure Rust Library | Accepted |
| [005](005-header-based-routing.md) | Header-Based Routing Protocol | Accepted |
| [006](006-config-generation-pipeline.md) | Config Generation Pipeline (Python + Jinja2) | Accepted |

## ADR Format

Each ADR follows this structure:
- **Status**: Proposed / Accepted / Deprecated / Superseded
- **Context**: What problem or question prompted this decision
- **Decision**: What was decided
- **Consequences**: Trade-offs, implications, and what this enables or prevents
