"""Pure validation and transformation functions for Plano config.

Every function in this module takes data in, returns data out, and raises
ConfigValidationError on failure. No file I/O, no print(), no exit().
"""

import logging
import os
from copy import deepcopy
from urllib.parse import urlparse

import yaml
from jsonschema import validate as jsonschema_validate

from planoai.config_providers import (
    INTERNAL_PROVIDERS,
    SUPPORTED_PROVIDERS,
    SUPPORTED_PROVIDERS_WITH_BASE_URL,
    ConfigValidationError,
    parse_url_endpoint,
)

log = logging.getLogger(__name__)


def validate_schema(config, schema):
    """Validate config dict against JSON schema dict.

    Raises ConfigValidationError with a clear message on failure.
    """
    try:
        jsonschema_validate(config, schema)
    except Exception as e:
        raise ConfigValidationError(f"Schema validation failed: {e}") from e


def migrate_legacy_providers(config):
    """Migrate llm_providers -> model_providers if needed.

    Returns a new config dict (does not mutate input).
    Raises ConfigValidationError if both are present.
    """
    config = deepcopy(config)

    if "llm_providers" in config:
        if "model_providers" in config:
            raise ConfigValidationError(
                "Please provide either llm_providers or model_providers, not both. "
                "llm_providers is deprecated, please use model_providers instead"
            )
        config["model_providers"] = config.pop("llm_providers")

    return config


def validate_agents(agents, filters):
    """Validate agent/filter entries and infer endpoint clusters from URLs.

    Returns dict of inferred endpoint clusters keyed by agent_id.
    Raises ConfigValidationError on duplicate IDs.
    """
    combined = agents + filters
    seen_ids = set()
    inferred_endpoints = {}

    for agent in combined:
        agent_id = agent.get("id")
        if agent_id in seen_ids:
            raise ConfigValidationError(
                f"Duplicate agent id {agent_id}, please provide unique id for each agent"
            )
        seen_ids.add(agent_id)

        agent_url = agent.get("url")
        if agent_id and agent_url:
            result = urlparse(agent_url)
            if result.scheme and result.hostname:
                port = result.port
                if port is None:
                    port = 80 if result.scheme == "http" else 443

                inferred_endpoints[agent_id] = {
                    "endpoint": result.hostname,
                    "port": port,
                    "protocol": result.scheme,
                }

    return inferred_endpoints


def build_clusters(endpoints, agent_inferred):
    """Merge explicit endpoints with agent-inferred clusters.

    Returns the final cluster dict.
    """
    clusters = dict(agent_inferred)

    for name, endpoint_details in endpoints.items():
        clusters[name] = dict(endpoint_details)
        # Resolve port for manually defined endpoints that lack one
        if "port" not in clusters[name]:
            endpoint = clusters[name]["endpoint"]
            protocol = clusters[name].get("protocol", "http")
            if ":" in endpoint:
                parts = endpoint.split(":")
                clusters[name]["endpoint"] = parts[0]
                clusters[name]["port"] = int(parts[1])
            else:
                clusters[name]["port"] = 80 if protocol == "http" else 443

    return clusters


def validate_prompt_targets(config, clusters):
    """Validate that prompt_targets reference valid endpoints."""
    for prompt_target in config.get("prompt_targets", []):
        name = prompt_target.get("endpoint", {}).get("name", None)
        if not name:
            continue
        if name not in clusters:
            raise ConfigValidationError(
                f"Unknown endpoint {name}, please add it in endpoints section "
                "in your arch_config.yaml file"
            )


def validate_tracing(tracing_config, default_endpoint):
    """Validate and resolve the tracing configuration.

    Handles env var resolution for opentracing_grpc_endpoint.
    Returns the resolved tracing dict.
    Raises ConfigValidationError for invalid endpoints.
    """
    tracing = deepcopy(tracing_config)

    # Resolution order: config yaml > OTEL_TRACING_GRPC_ENDPOINT env var > default
    endpoint = tracing.get(
        "opentracing_grpc_endpoint",
        os.environ.get("OTEL_TRACING_GRPC_ENDPOINT", default_endpoint),
    )

    # Resolve env var references like $VAR or ${VAR}
    if endpoint and "$" in endpoint:
        endpoint = os.path.expandvars(endpoint)
        log.info("Resolved opentracing_grpc_endpoint to %s", endpoint)

    tracing["opentracing_grpc_endpoint"] = endpoint

    if endpoint:
        result = urlparse(endpoint)
        if result.scheme != "http":
            raise ConfigValidationError(
                f"Invalid opentracing_grpc_endpoint {endpoint}, scheme must be http"
            )
        if result.path and result.path != "/":
            raise ConfigValidationError(
                f"Invalid opentracing_grpc_endpoint {endpoint}, path must be empty"
            )

    return tracing


def process_model_providers(listeners, routing_config):
    """Process all model providers from listeners.

    Validates names, models, provider interfaces, base_urls, wildcards,
    routing preferences, and injects internal providers.

    Args:
        listeners: List of listener dicts from config.
        routing_config: The 'routing' section from config (may be empty dict).

    Returns:
        Tuple of (updated_model_providers, llms_with_endpoint, model_name_keys).

    Raises:
        ConfigValidationError on any validation failure.
    """
    llms_with_endpoint = []
    llms_with_endpoint_cluster_names = set()
    updated_model_providers = []
    model_provider_name_set = set()
    model_name_keys = set()
    model_usage_name_keys = set()

    for listener in listeners:
        if not listener.get("model_providers"):
            continue

        for model_provider in listener.get("model_providers", []):
            _validate_and_process_single_provider(
                model_provider,
                model_name_keys,
                model_provider_name_set,
                model_usage_name_keys,
                updated_model_providers,
                llms_with_endpoint,
                llms_with_endpoint_cluster_names,
            )

    # Inject internal providers
    _inject_internal_providers(
        updated_model_providers,
        model_provider_name_set,
        model_usage_name_keys,
        routing_config,
    )

    return updated_model_providers, llms_with_endpoint, model_name_keys


def _validate_and_process_single_provider(
    model_provider,
    model_name_keys,
    model_provider_name_set,
    model_usage_name_keys,
    updated_model_providers,
    llms_with_endpoint,
    llms_with_endpoint_cluster_names,
):
    """Validate and normalize a single model_provider entry."""
    # Check duplicate provider name
    if model_provider.get("name") in model_provider_name_set:
        raise ConfigValidationError(
            f"Duplicate model_provider name {model_provider.get('name')}, "
            "please provide unique name for each model_provider"
        )

    model_name = model_provider.get("model")

    # Parse model name into provider/model_id
    model_name_tokens = model_name.split("/")
    if len(model_name_tokens) < 2:
        raise ConfigValidationError(
            f"Invalid model name {model_name}. Please provide model name in the "
            "format <provider>/<model_id> or <provider>/* for wildcards."
        )

    provider = model_name_tokens[0].strip()
    model_id = "/".join(model_name_tokens[1:])
    is_wildcard = model_name_tokens[-1].strip() == "*"

    # Check duplicate model name (non-wildcard only)
    if model_name in model_name_keys and not is_wildcard:
        raise ConfigValidationError(
            f"Duplicate model name {model_name}, please provide unique model "
            "name for each model_provider"
        )

    if not is_wildcard:
        model_name_keys.add(model_name)

    # Auto-name if not provided
    if model_provider.get("name") is None:
        model_provider["name"] = model_name

    model_provider_name_set.add(model_provider.get("name"))

    # Validate wildcard constraints
    if is_wildcard:
        if model_provider.get("default", False):
            raise ConfigValidationError(
                f"Model {model_name} is configured as default but uses wildcard (*). "
                "Default models cannot be wildcards."
            )
        if model_provider.get("routing_preferences"):
            raise ConfigValidationError(
                f"Model {model_name} has routing_preferences but uses wildcard (*). "
                "Models with routing preferences cannot be wildcards."
            )

    # Validate provider requires base_url
    if provider in SUPPORTED_PROVIDERS_WITH_BASE_URL and not model_provider.get(
        "base_url"
    ):
        raise ConfigValidationError(
            f"Provider '{provider}' requires 'base_url' to be set for model {model_name}"
        )

    # Resolve provider interface
    if provider not in SUPPORTED_PROVIDERS:
        if not model_provider.get("base_url") or not model_provider.get(
            "provider_interface"
        ):
            raise ConfigValidationError(
                f"Must provide base_url and provider_interface for unsupported "
                f"provider {provider} for {'wildcard ' if is_wildcard else ''}model "
                f"{model_name}. Supported providers are: "
                f"{', '.join(SUPPORTED_PROVIDERS)}"
            )
        provider = model_provider.get("provider_interface")
    elif model_provider.get("provider_interface") is not None:
        raise ConfigValidationError(
            f"Please provide provider interface as part of model name {model_name} "
            "using the format <provider>/<model_id>. For example, use "
            "'openai/gpt-3.5-turbo' instead of 'gpt-3.5-turbo' "
        )

    # Check duplicate model_id (non-wildcard only)
    if not is_wildcard:
        if model_id in model_name_keys:
            raise ConfigValidationError(
                f"Duplicate model_id {model_id}, please provide unique model_id "
                "for each model_provider"
            )
        model_name_keys.add(model_id)

    # Validate routing preferences
    for routing_preference in model_provider.get("routing_preferences", []):
        pref_name = routing_preference.get("name")
        if pref_name in model_usage_name_keys:
            raise ConfigValidationError(
                f'Duplicate routing preference name "{pref_name}", please provide '
                "unique name for each routing preference"
            )
        model_usage_name_keys.add(pref_name)

    # Warn if both passthrough_auth and access_key are configured
    if model_provider.get("passthrough_auth") and model_provider.get("access_key"):
        log.warning(
            "Model provider '%s' has both 'passthrough_auth: true' and 'access_key' "
            "configured. The access_key will be ignored and the client's Authorization "
            "header will be forwarded instead.",
            model_provider.get("name"),
        )

    # Normalize provider fields
    model_provider["model"] = model_id
    model_provider["provider_interface"] = provider
    model_provider_name_set.add(model_provider.get("name"))

    if model_provider.get("provider") and model_provider.get("provider_interface"):
        raise ConfigValidationError(
            "Please provide either provider or provider_interface, not both"
        )
    if model_provider.get("provider"):
        provider = model_provider["provider"]
        model_provider["provider_interface"] = provider
        del model_provider["provider"]

    updated_model_providers.append(model_provider)

    # Process base_url into cluster endpoint info
    if model_provider.get("base_url"):
        _process_base_url(
            model_provider,
            provider,
            llms_with_endpoint,
            llms_with_endpoint_cluster_names,
        )


def _process_base_url(
    model_provider, provider, llms_with_endpoint, llms_with_endpoint_cluster_names
):
    """Parse base_url and add cluster endpoint info to the model provider."""
    base_url = model_provider["base_url"]
    parsed = parse_url_endpoint(base_url)

    if parsed.get("path_prefix"):
        model_provider["base_url_path_prefix"] = parsed["path_prefix"]

    model_provider["endpoint"] = parsed["endpoint"]
    model_provider["port"] = parsed["port"]
    model_provider["protocol"] = parsed["protocol"]

    cluster_name = provider + "_" + parsed["endpoint"]
    model_provider["cluster_name"] = cluster_name

    if cluster_name not in llms_with_endpoint_cluster_names:
        llms_with_endpoint.append(model_provider)
        llms_with_endpoint_cluster_names.add(cluster_name)


def _inject_internal_providers(
    updated_model_providers,
    model_provider_name_set,
    model_usage_name_keys,
    routing_config,
):
    """Add arch-router, arch-function, plano-orchestrator if not already defined."""
    # Add arch-router if routing preferences exist and no router is configured
    if len(model_usage_name_keys) > 0:
        routing_model_provider = routing_config.get("model_provider", None)
        if (
            routing_model_provider
            and routing_model_provider not in model_provider_name_set
        ):
            raise ConfigValidationError(
                f"Routing model_provider {routing_model_provider} is not defined "
                "in model_providers"
            )
        if (
            routing_model_provider is None
            and "arch-router" not in model_provider_name_set
        ):
            updated_model_providers.append(
                {
                    "name": "arch-router",
                    "provider_interface": "arch",
                    "model": routing_config.get("model", "Arch-Router"),
                    "internal": True,
                }
            )

    for name, provider_def in INTERNAL_PROVIDERS.items():
        if name not in model_provider_name_set:
            updated_model_providers.append(dict(provider_def))


def validate_listeners(listeners):
    """Validate that at most one listener has model_providers."""
    count = sum(1 for l in listeners if l.get("model_providers") is not None)
    if count > 1:
        raise ConfigValidationError(
            "Please provide model_providers either under listeners or at root level, "
            "not both. Currently we don't support multiple listeners with model_providers"
        )


def validate_model_aliases(aliases, model_name_keys):
    """Validate that model aliases reference existing models."""
    for alias_name, alias_config in aliases.items():
        target = alias_config.get("target")
        if target not in model_name_keys:
            raise ConfigValidationError(
                f"Model alias '{alias_name}' targets '{target}' which is not "
                f"defined as a model. Available models: "
                f"{', '.join(sorted(model_name_keys))}"
            )


def resolve_agent_orchestrator(config, endpoints):
    """Resolve agent orchestrator from config overrides.

    Returns the orchestrator endpoint name, or None if not configured.
    """
    use_orchestrator = config.get("overrides", {}).get("use_agent_orchestrator", False)
    if not use_orchestrator:
        return None

    if len(endpoints) == 0:
        raise ConfigValidationError(
            "Please provide agent orchestrator in the endpoints section "
            "in your arch_config.yaml file"
        )
    if len(endpoints) > 1:
        raise ConfigValidationError(
            "Please provide single agent orchestrator in the endpoints section "
            "in your arch_config.yaml file"
        )

    return list(endpoints.keys())[0]


def build_template_data(
    prompt_gateway,
    llm_gateway,
    config_yaml,
    clusters,
    model_providers,
    tracing,
    llms_with_endpoint,
    agent_orchestrator,
    listeners,
):
    """Assemble the Jinja2 template rendering context.

    Note: arch_config and arch_llm_config are intentionally the same value.
    Both are kept for backward compatibility with the Envoy template.
    """
    config_string = yaml.dump(config_yaml)
    return {
        "prompt_gateway_listener": prompt_gateway,
        "llm_gateway_listener": llm_gateway,
        "arch_config": config_string,
        "arch_llm_config": config_string,
        "arch_clusters": clusters,
        "arch_model_providers": model_providers,
        "arch_tracing": tracing,
        "local_llms": llms_with_endpoint,
        "agent_orchestrator": agent_orchestrator,
        "listeners": listeners,
    }
