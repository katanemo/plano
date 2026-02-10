# ADR 006: Config Generation Pipeline (Python + Jinja2)

**Status:** Accepted

## Context

Envoy's configuration is a large YAML file that must describe all listeners, clusters, filter chains, TLS contexts, and WASM filter configs. This configuration depends on user-provided settings (which LLM providers to use, which agents to connect, which endpoints to expose).

The options were:
1. **Static Envoy config** — users edit Envoy YAML directly
2. **Rust-based config generator** — generate Envoy config from a Rust binary
3. **Python + Jinja2 template** — validate user config against a schema, then render Envoy config from a template

## Decision

Use a **Python config generator** (`cli/planoai/config_generator.py`) that:
1. Validates user's `arch_config.yaml` against a JSON Schema (`config/arch_config_schema.yaml`)
2. Applies transformations (legacy format conversion, cluster inference, internal model injection)
3. Renders `config/envoy.template.yaml` (Jinja2) into the final `envoy.yaml`
4. Produces `arch_config_rendered.yaml` for Brightstaff and WASM filter consumption

This runs at container startup, before Envoy starts.

## Consequences

**Enables:**
- Simple user-facing config format (`arch_config.yaml`) — users don't need to understand Envoy internals
- JSON Schema validation catches errors before Envoy starts
- Jinja2 templating is mature, well-understood, and powerful for generating complex YAML
- Python CLI (`planoai`) can also handle Docker management and other tooling
- Config validation is independently testable (`cli/test/test_config_generator.py`)

**Requires:**
- Python runtime in the Docker image (adds image size)
- Config changes need updates in 4 places: schema, template, Python validator, Rust struct
- Understanding of Jinja2 templating for Envoy config modifications
- `arch_config_rendered.yaml` must be kept in sync between Python generator and Rust deserialization

**Prevents:**
- Dynamic config reloading without container restart (config is generated at startup)
- Using Envoy's xDS protocol for dynamic configuration (could be added later)
- Rust-only development workflow — Python is required for config generation

**4-file update rule:** Every new user-facing config field requires changes to:
1. `config/arch_config_schema.yaml` — JSON Schema definition
2. `config/envoy.template.yaml` — Jinja2 template (if Envoy needs the value)
3. `cli/planoai/config_generator.py` — Python validation and rendering logic
4. `common/src/configuration.rs` — Rust `Configuration` struct (for runtime consumption)
