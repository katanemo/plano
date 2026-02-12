"""Config generator: loads config files, validates, and renders Envoy template.

This module is the I/O boundary. It reads files, calls pure validation
functions from config_validator and config_providers, then writes output.

Entry point: ``python -m planoai.config_generator`` (called by supervisord).
"""

import logging
import os

import yaml
from copy import deepcopy
from jinja2 import Environment, FileSystemLoader

from planoai.config_providers import (
    ConfigValidationError,
    # Re-export for backward compatibility
    SUPPORTED_PROVIDERS,
    SUPPORTED_PROVIDERS_WITH_BASE_URL,
    SUPPORTED_PROVIDERS_WITHOUT_BASE_URL,
)
from planoai.config_validator import (
    build_clusters,
    build_template_data,
    migrate_legacy_providers,
    process_model_providers,
    resolve_agent_orchestrator,
    validate_agents,
    validate_listeners,
    validate_model_aliases,
    validate_prompt_targets,
    validate_schema,
    validate_tracing,
)
from planoai.consts import DEFAULT_OTEL_TRACING_GRPC_ENDPOINT
from planoai.utils import convert_legacy_listeners

log = logging.getLogger(__name__)


def load_yaml_file(path):
    """Read a YAML file and return the parsed dict."""
    with open(path, "r") as f:
        raw = f.read()
    return yaml.safe_load(raw)


def validate_and_render_schema():
    """Main orchestrator: load -> validate -> process -> render -> write.

    Reads env vars for file paths (Docker integration).
    Raises ConfigValidationError on validation failure.
    """
    # --- Read environment config ---
    template_file = os.getenv("ENVOY_CONFIG_TEMPLATE_FILE", "envoy.template.yaml")
    config_path = os.getenv("ARCH_CONFIG_FILE", "/app/arch_config.yaml")
    rendered_config_path = os.getenv(
        "ARCH_CONFIG_FILE_RENDERED", "/app/arch_config_rendered.yaml"
    )
    envoy_rendered_path = os.getenv(
        "ENVOY_CONFIG_FILE_RENDERED", "/etc/envoy/envoy.yaml"
    )
    schema_path = os.getenv("ARCH_CONFIG_SCHEMA_FILE", "arch_config_schema.yaml")
    template_root = os.getenv("TEMPLATE_ROOT", "./")

    # --- Load files (each read exactly once) ---
    config = load_yaml_file(config_path)
    schema = load_yaml_file(schema_path)

    env = Environment(loader=FileSystemLoader(template_root))
    template = env.get_template(template_file)

    # --- Validate and process ---
    validate_schema(config, schema)
    config = migrate_legacy_providers(config)

    listeners, llm_gateway, prompt_gateway = convert_legacy_listeners(
        config.get("listeners"), config.get("model_providers")
    )
    config["listeners"] = listeners

    agent_endpoints = validate_agents(
        config.get("agents", []), config.get("filters", [])
    )
    clusters = build_clusters(config.get("endpoints", {}), agent_endpoints)
    log.info("Defined clusters: %s", clusters)

    validate_prompt_targets(config, clusters)

    tracing = validate_tracing(
        config.get("tracing", {}), DEFAULT_OTEL_TRACING_GRPC_ENDPOINT
    )

    updated_providers, llms_with_endpoint, model_name_keys = process_model_providers(
        listeners, config.get("routing", {})
    )
    config["model_providers"] = deepcopy(updated_providers)

    validate_listeners(listeners)

    if "model_aliases" in config:
        validate_model_aliases(config["model_aliases"], model_name_keys)

    agent_orchestrator = resolve_agent_orchestrator(
        config, config.get("endpoints", {})
    )

    data = build_template_data(
        prompt_gateway,
        llm_gateway,
        config,
        clusters,
        updated_providers,
        tracing,
        llms_with_endpoint,
        agent_orchestrator,
        listeners,
    )

    # --- Render and write ---
    rendered = template.render(data)
    log.info("Writing Envoy config to %s", envoy_rendered_path)

    with open(envoy_rendered_path, "w") as f:
        f.write(rendered)

    config_string = yaml.dump(config)
    with open(rendered_config_path, "w") as f:
        f.write(config_string)


if __name__ == "__main__":
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s - %(name)s - %(levelname)s - %(message)s",
    )
    try:
        validate_and_render_schema()
    except ConfigValidationError as e:
        log.error(str(e))
        exit(1)
    except Exception as e:
        log.error("Unexpected error: %s", e)
        exit(1)
