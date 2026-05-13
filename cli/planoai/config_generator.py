import json
import os
import uuid
from pathlib import Path
from planoai.utils import convert_legacy_listeners
from jinja2 import Environment, FileSystemLoader
import yaml
from jsonschema import validate, ValidationError
from urllib.parse import urlparse
from copy import deepcopy
from planoai.consts import DEFAULT_OTEL_TRACING_GRPC_ENDPOINT
from planoai.skills import (
    MAX_CATALOG_BYTES,
    Skill,
    discover_skills,
    find_project_root,
    is_project_trusted,
    total_catalog_size,
)

SUPPORTED_PROVIDERS_WITH_BASE_URL = [
    "azure_openai",
    "ollama",
    "qwen",
    "amazon_bedrock",
    "plano",
]

SUPPORTED_PROVIDERS_WITHOUT_BASE_URL = [
    "deepseek",
    "groq",
    "mistral",
    "openai",
    "xiaomi",
    "gemini",
    "anthropic",
    "together_ai",
    "xai",
    "moonshotai",
    "zhipu",
    "chatgpt",
    "digitalocean",
    "vercel",
    "openrouter",
]

CHATGPT_API_BASE = "https://chatgpt.com/backend-api/codex"
CHATGPT_DEFAULT_ORIGINATOR = "codex_cli_rs"
CHATGPT_DEFAULT_USER_AGENT = "codex_cli_rs/0.0.0 (Unknown 0; unknown) unknown"

SUPPORTED_PROVIDERS = (
    SUPPORTED_PROVIDERS_WITHOUT_BASE_URL + SUPPORTED_PROVIDERS_WITH_BASE_URL
)


def get_endpoint_and_port(endpoint, protocol):
    endpoint_tokens = endpoint.split(":")
    if len(endpoint_tokens) > 1:
        endpoint = endpoint_tokens[0]
        port = int(endpoint_tokens[1])
        return endpoint, port
    else:
        if protocol == "http":
            port = 80
        else:
            port = 443
        return endpoint, port


def migrate_inline_routing_preferences(config_yaml):
    """Lift v0.3.0-style inline ``routing_preferences`` under each
    ``model_providers`` entry to the v0.4.0 top-level ``routing_preferences``
    list with ``models: [...]``.

    This function is a no-op for configs whose ``version`` is already
    ``v0.4.0`` or newer — those are assumed to be on the canonical
    top-level shape and are passed through untouched.

    For older configs, the version is bumped to ``v0.4.0`` up front so
    brightstaff's v0.4.0 gate for top-level ``routing_preferences``
    accepts the rendered config, then inline preferences under each
    provider are lifted into the top-level list. Preferences with the
    same ``name`` across multiple providers are merged into a single
    top-level entry whose ``models`` list contains every provider's
    full ``<provider>/<model>`` string in declaration order. The first
    ``description`` encountered wins; conflicts are warned, not errored,
    so existing v0.3.0 configs keep compiling. Any top-level preference
    already defined by the user is preserved as-is.
    """
    current_version = str(config_yaml.get("version", ""))
    if _version_tuple(current_version) >= (0, 4, 0):
        return

    config_yaml["version"] = "v0.4.0"

    model_providers = config_yaml.get("model_providers") or []
    if not model_providers:
        return

    migrated = {}
    for model_provider in model_providers:
        inline_prefs = model_provider.get("routing_preferences")
        if not inline_prefs:
            continue

        full_model_name = model_provider.get("model")
        if not full_model_name:
            continue

        if "/" in full_model_name and full_model_name.split("/")[-1].strip() == "*":
            raise Exception(
                f"Model {full_model_name} has routing_preferences but uses wildcard (*). Models with routing preferences cannot be wildcards."
            )

        for pref in inline_prefs:
            name = pref.get("name")
            description = pref.get("description", "")
            if not name:
                continue
            if name in migrated:
                entry = migrated[name]
                if description and description != entry["description"]:
                    print(
                        f"WARNING: routing preference '{name}' has conflicting descriptions across providers; keeping the first one."
                    )
                if full_model_name not in entry["models"]:
                    entry["models"].append(full_model_name)
            else:
                migrated[name] = {
                    "name": name,
                    "description": description,
                    "models": [full_model_name],
                }

    if not migrated:
        return

    for model_provider in model_providers:
        if "routing_preferences" in model_provider:
            del model_provider["routing_preferences"]

    existing_top_level = config_yaml.get("routing_preferences") or []
    existing_names = {entry.get("name") for entry in existing_top_level}
    merged = list(existing_top_level)
    for name, entry in migrated.items():
        if name in existing_names:
            continue
        merged.append(entry)
    config_yaml["routing_preferences"] = merged

    print(
        "WARNING: inline routing_preferences under model_providers is deprecated "
        "and has been auto-migrated to top-level routing_preferences. Update your "
        "config to v0.4.0 top-level form. See docs/routing-api.md"
    )


def _version_tuple(version_string):
    stripped = version_string.strip().lstrip("vV")
    if not stripped:
        return (0, 0, 0)
    parts = stripped.split("-", 1)[0].split(".")
    out = []
    for part in parts[:3]:
        try:
            out.append(int(part))
        except ValueError:
            out.append(0)
    while len(out) < 3:
        out.append(0)
    return tuple(out)


def materialize_skills_in_config(config_yaml: dict, project_root: Path) -> None:
    """Discover and inline Agent Skills referenced by `config_yaml`.

    Mutates `config_yaml` in place. The user's source config may declare
    `skills:` as a list of strings (skill names) or omit it entirely. After
    this call, `config_yaml["skills"]` is either absent or a list of fully
    materialized objects with `name`, `description`, `path`, `body`, etc.

    Project-scope skills under `<project_root>/.plano/skills/` are only loaded
    when the project has been marked trusted via `planoai skills trust`.

    Per-route `routing_preferences[].skills` allow-lists are preserved as-is
    so brightstaff can scope the catalog when that route is selected.
    """
    requested = config_yaml.get("skills")
    user_only = not is_project_trusted(project_root)

    discovered, diagnostics = discover_skills(
        project_root=project_root, include_user_scope=True
    )
    for diag in diagnostics:
        prefix = "error" if diag.severity == "error" else "warning"
        print(f"[skills] {prefix}: {diag.path}: {diag.message}")

    if user_only:
        project_skills = [s for s in discovered if s.scope == "project"]
        if project_skills:
            print(
                "[skills] note: project-scope skills are present but the project is "
                "not trusted yet; run `planoai skills trust` to enable them."
            )
        # Keep all non-project scopes (user + agents) — both are user-tier and
        # auto-trusted, so they always load regardless of project trust state.
        discovered = [s for s in discovered if s.scope != "project"]

    skills_by_name: dict[str, Skill] = {s.name: s for s in discovered}

    if requested is None:
        # Default: auto-include every discovered skill.
        selected: list[Skill] = list(discovered)
    else:
        if not isinstance(requested, list):
            raise Exception("`skills:` must be a list of strings or skill objects")
        selected = []
        seen: set[str] = set()
        for entry in requested:
            if isinstance(entry, str):
                name = entry
            elif isinstance(entry, dict):
                name = entry.get("name")
                if not isinstance(name, str):
                    raise Exception(
                        "skill entries with object form must include a string `name`"
                    )
            else:
                raise Exception(
                    f"unsupported entry in `skills:` (expected str or mapping, got {type(entry).__name__})"
                )
            if name in seen:
                continue
            seen.add(name)
            skill = skills_by_name.get(name)
            if skill is None:
                print(
                    f"[skills] warning: skill '{name}' is declared in config but no "
                    f"SKILL.md was discovered under .plano/skills/ or ~/.plano/skills/"
                )
                continue
            selected.append(skill)

    if not selected:
        config_yaml.pop("skills", None)
        _strip_unknown_route_skills(config_yaml, set())
        return

    catalog_bytes = total_catalog_size(selected)
    if catalog_bytes > MAX_CATALOG_BYTES:
        print(
            f"[skills] warning: skill catalog size is {catalog_bytes} bytes, "
            f"above the recommended cap of {MAX_CATALOG_BYTES}. Consider trimming "
            f"`routing_preferences[].skills` to the smallest useful set per route."
        )

    config_yaml["skills"] = [s.to_dict() for s in selected]
    _strip_unknown_route_skills(config_yaml, {s.name for s in selected})


def _strip_unknown_route_skills(config_yaml: dict, known: set) -> None:
    """Drop unknown skill names from `routing_preferences[*].skills` allow-lists.

    The orchestrator only ever sees skills referenced under some
    `routing_preferences[].skills`; an unknown name there would render the
    `<skills>` block with a stale entry the runtime can't resolve, so filter
    them out here with a warning instead.
    """
    routes = config_yaml.get("routing_preferences")
    if not isinstance(routes, list):
        return
    for route in routes:
        if not isinstance(route, dict):
            continue
        allow = route.get("skills")
        if not isinstance(allow, list):
            continue
        filtered = []
        for name in allow:
            if not isinstance(name, str):
                continue
            if name in known:
                filtered.append(name)
            else:
                print(
                    f"[skills] warning: routing_preference '{route.get('name')}' "
                    f"references unknown skill '{name}'; dropping from allow-list."
                )
        if filtered:
            route["skills"] = filtered
        else:
            route.pop("skills", None)


def validate_and_render_schema():
    ENVOY_CONFIG_TEMPLATE_FILE = os.getenv(
        "ENVOY_CONFIG_TEMPLATE_FILE", "envoy.template.yaml"
    )
    PLANO_CONFIG_FILE = os.getenv("PLANO_CONFIG_FILE", "/app/plano_config.yaml")
    PLANO_CONFIG_FILE_RENDERED = os.getenv(
        "PLANO_CONFIG_FILE_RENDERED", "/app/plano_config_rendered.yaml"
    )
    ENVOY_CONFIG_FILE_RENDERED = os.getenv(
        "ENVOY_CONFIG_FILE_RENDERED", "/etc/envoy/envoy.yaml"
    )
    PLANO_CONFIG_SCHEMA_FILE = os.getenv(
        "PLANO_CONFIG_SCHEMA_FILE", "plano_config_schema.yaml"
    )

    env = Environment(loader=FileSystemLoader(os.getenv("TEMPLATE_ROOT", "./")))
    template = env.get_template(ENVOY_CONFIG_TEMPLATE_FILE)

    try:
        validate_prompt_config(PLANO_CONFIG_FILE, PLANO_CONFIG_SCHEMA_FILE)
    except Exception as e:
        print(str(e))
        exit(1)  # validate_prompt_config failed. Exit

    with open(PLANO_CONFIG_FILE, "r") as file:
        plano_config = file.read()

    with open(PLANO_CONFIG_SCHEMA_FILE, "r") as file:
        plano_config_schema = file.read()

    config_yaml = yaml.safe_load(plano_config)
    _ = yaml.safe_load(plano_config_schema)
    inferred_clusters = {}

    # Materialize Agent Skills before further processing so the rest of the
    # pipeline (Jinja2 envoy template, dump to plano_config_rendered.yaml) sees
    # the inlined body / description / path.
    plano_config_path = Path(PLANO_CONFIG_FILE).resolve()
    project_root = find_project_root(plano_config_path.parent)
    materialize_skills_in_config(config_yaml, project_root)

    # Convert legacy llm_providers to model_providers
    if "llm_providers" in config_yaml:
        if "model_providers" in config_yaml:
            raise Exception(
                "Please provide either llm_providers or model_providers, not both. llm_providers is deprecated, please use model_providers instead"
            )
        config_yaml["model_providers"] = config_yaml["llm_providers"]
        del config_yaml["llm_providers"]

    migrate_inline_routing_preferences(config_yaml)

    listeners, llm_gateway, prompt_gateway = convert_legacy_listeners(
        config_yaml.get("listeners"), config_yaml.get("model_providers")
    )

    config_yaml["listeners"] = listeners

    endpoints = config_yaml.get("endpoints", {})

    # Process agents section and convert to endpoints
    agents = config_yaml.get("agents", [])
    filters = config_yaml.get("filters", [])
    agents_combined = agents + filters
    agent_id_keys = set()

    for agent in agents_combined:
        agent_id = agent.get("id")
        if agent_id in agent_id_keys:
            raise Exception(
                f"Duplicate agent id {agent_id}, please provide unique id for each agent"
            )
        agent_id_keys.add(agent_id)
        agent_endpoint = agent.get("url")

        if agent_id and agent_endpoint:
            urlparse_result = urlparse(agent_endpoint)
            if urlparse_result.scheme and urlparse_result.hostname:
                protocol = urlparse_result.scheme

                port = urlparse_result.port
                if port is None:
                    if protocol == "http":
                        port = 80
                    else:
                        port = 443

                endpoints[agent_id] = {
                    "endpoint": urlparse_result.hostname,
                    "port": port,
                    "protocol": protocol,
                }

    # override the inferred clusters with the ones defined in the config
    for name, endpoint_details in endpoints.items():
        inferred_clusters[name] = endpoint_details
        # Only call get_endpoint_and_port for manually defined endpoints, not agent-derived ones
        if "port" not in endpoint_details:
            endpoint = inferred_clusters[name]["endpoint"]
            protocol = inferred_clusters[name].get("protocol", "http")
            (
                inferred_clusters[name]["endpoint"],
                inferred_clusters[name]["port"],
            ) = get_endpoint_and_port(endpoint, protocol)

    print("defined clusters from plano_config.yaml: ", json.dumps(inferred_clusters))

    if "prompt_targets" in config_yaml:
        for prompt_target in config_yaml["prompt_targets"]:
            name = prompt_target.get("endpoint", {}).get("name", None)
            if not name:
                continue
            if name not in inferred_clusters:
                raise Exception(
                    f"Unknown endpoint {name}, please add it in endpoints section in your plano_config.yaml file"
                )

    plano_tracing = config_yaml.get("tracing", {})

    # Resolution order: config yaml > OTEL_TRACING_GRPC_ENDPOINT env var > hardcoded default
    opentracing_grpc_endpoint = plano_tracing.get(
        "opentracing_grpc_endpoint",
        os.environ.get(
            "OTEL_TRACING_GRPC_ENDPOINT", DEFAULT_OTEL_TRACING_GRPC_ENDPOINT
        ),
    )
    # resolve env vars in opentracing_grpc_endpoint if present
    if opentracing_grpc_endpoint and "$" in opentracing_grpc_endpoint:
        opentracing_grpc_endpoint = os.path.expandvars(opentracing_grpc_endpoint)
        print(
            f"Resolved opentracing_grpc_endpoint to {opentracing_grpc_endpoint} after expanding environment variables"
        )
    plano_tracing["opentracing_grpc_endpoint"] = opentracing_grpc_endpoint
    # ensure that opentracing_grpc_endpoint is a valid URL if present and start with http and must not have any path
    if opentracing_grpc_endpoint:
        urlparse_result = urlparse(opentracing_grpc_endpoint)
        if urlparse_result.scheme != "http":
            raise Exception(
                f"Invalid opentracing_grpc_endpoint {opentracing_grpc_endpoint}, scheme must be http"
            )
        if urlparse_result.path and urlparse_result.path != "/":
            raise Exception(
                f"Invalid opentracing_grpc_endpoint {opentracing_grpc_endpoint}, path must be empty"
            )

    llms_with_endpoint = []
    llms_with_endpoint_cluster_names = set()
    updated_model_providers = []
    model_provider_name_set = set()
    llms_with_usage = []
    model_name_keys = set()

    top_level_preferences = config_yaml.get("routing_preferences") or []
    seen_pref_names = set()
    for pref in top_level_preferences:
        pref_name = pref.get("name")
        if pref_name in seen_pref_names:
            raise Exception(
                f'Duplicate routing preference name "{pref_name}", please provide unique name for each routing preference'
            )
        seen_pref_names.add(pref_name)

    print("listeners: ", listeners)

    for listener in listeners:
        if (
            listener.get("model_providers") is None
            or listener.get("model_providers") == []
        ):
            continue
        print("Processing listener with model_providers: ", listener)
        name = listener.get("name", None)

        for model_provider in listener.get("model_providers", []):
            if model_provider.get("usage", None):
                llms_with_usage.append(model_provider["name"])
            if model_provider.get("name") in model_provider_name_set:
                raise Exception(
                    f"Duplicate model_provider name {model_provider.get('name')}, please provide unique name for each model_provider"
                )

            model_name = model_provider.get("model")
            print("Processing model_provider: ", model_provider)

            # Check if this is a wildcard model (provider/*)
            is_wildcard = False
            if "/" in model_name:
                model_name_tokens = model_name.split("/")
                if len(model_name_tokens) >= 2 and model_name_tokens[-1] == "*":
                    is_wildcard = True

            if model_name in model_name_keys and not is_wildcard:
                raise Exception(
                    f"Duplicate model name {model_name}, please provide unique model name for each model_provider"
                )

            if not is_wildcard:
                model_name_keys.add(model_name)
            if model_provider.get("name") is None:
                model_provider["name"] = model_name

            model_provider_name_set.add(model_provider.get("name"))

            model_name_tokens = model_name.split("/")
            if len(model_name_tokens) < 2:
                raise Exception(
                    f"Invalid model name {model_name}. Please provide model name in the format <provider>/<model_id> or <provider>/* for wildcards."
                )
            provider = model_name_tokens[0].strip()

            # Check if this is a wildcard (provider/*)
            is_wildcard = model_name_tokens[-1].strip() == "*"

            # Validate wildcard constraints
            if is_wildcard:
                if model_provider.get("default", False):
                    raise Exception(
                        f"Model {model_name} is configured as default but uses wildcard (*). Default models cannot be wildcards."
                    )

            # Validate azure_openai and ollama provider requires base_url
            if (provider in SUPPORTED_PROVIDERS_WITH_BASE_URL) and model_provider.get(
                "base_url"
            ) is None:
                raise Exception(
                    f"Provider '{provider}' requires 'base_url' to be set for model {model_name}"
                )

            model_id = "/".join(model_name_tokens[1:])

            # For wildcard providers, allow any provider name
            if not is_wildcard and provider not in SUPPORTED_PROVIDERS:
                if (
                    model_provider.get("base_url", None) is None
                    or model_provider.get("provider_interface", None) is None
                ):
                    raise Exception(
                        f"Must provide base_url and provider_interface for unsupported provider {provider} for model {model_name}. Supported providers are: {', '.join(SUPPORTED_PROVIDERS)}"
                    )
                provider = model_provider.get("provider_interface", None)
            elif is_wildcard and provider not in SUPPORTED_PROVIDERS:
                # Wildcard models with unsupported providers require base_url and provider_interface
                if (
                    model_provider.get("base_url", None) is None
                    or model_provider.get("provider_interface", None) is None
                ):
                    raise Exception(
                        f"Must provide base_url and provider_interface for unsupported provider {provider} for wildcard model {model_name}. Supported providers are: {', '.join(SUPPORTED_PROVIDERS)}"
                    )
                provider = model_provider.get("provider_interface", None)
            elif (
                provider in SUPPORTED_PROVIDERS
                and model_provider.get("provider_interface", None) is not None
            ):
                # For supported providers, provider_interface should not be manually set
                raise Exception(
                    f"Please provide provider interface as part of model name {model_name} using the format <provider>/<model_id>. For example, use 'openai/gpt-3.5-turbo' instead of 'gpt-3.5-turbo' "
                )

            # For wildcard models, don't add model_id to the keys since it's "*"
            if not is_wildcard:
                if model_id in model_name_keys:
                    raise Exception(
                        f"Duplicate model_id {model_id}, please provide unique model_id for each model_provider"
                    )
                model_name_keys.add(model_id)

            # Warn if both passthrough_auth and access_key are configured
            if model_provider.get("passthrough_auth") and model_provider.get(
                "access_key"
            ):
                print(
                    f"WARNING: Model provider '{model_provider.get('name')}' has both 'passthrough_auth: true' and 'access_key' configured. "
                    f"The access_key will be ignored and the client's Authorization header will be forwarded instead."
                )

            model_provider["model"] = model_id
            model_provider["provider_interface"] = provider
            model_provider_name_set.add(model_provider.get("name"))
            if model_provider.get("provider") and model_provider.get(
                "provider_interface"
            ):
                raise Exception(
                    "Please provide either provider or provider_interface, not both"
                )
            if model_provider.get("provider"):
                provider = model_provider["provider"]
                model_provider["provider_interface"] = provider
                del model_provider["provider"]

            # Auto-wire ChatGPT provider: inject base_url, passthrough_auth, and extra headers
            if provider == "chatgpt":
                if not model_provider.get("base_url"):
                    model_provider["base_url"] = CHATGPT_API_BASE
                if not model_provider.get("access_key") and not model_provider.get(
                    "passthrough_auth"
                ):
                    model_provider["passthrough_auth"] = True
                headers = model_provider.get("headers", {})
                headers.setdefault(
                    "ChatGPT-Account-Id",
                    os.environ.get("CHATGPT_ACCOUNT_ID", ""),
                )
                headers.setdefault("originator", CHATGPT_DEFAULT_ORIGINATOR)
                headers.setdefault("user-agent", CHATGPT_DEFAULT_USER_AGENT)
                headers.setdefault("session_id", str(uuid.uuid4()))
                model_provider["headers"] = headers

            updated_model_providers.append(model_provider)

            if model_provider.get("base_url", None):
                base_url = model_provider["base_url"]
                urlparse_result = urlparse(base_url)
                base_url_path_prefix = urlparse_result.path
                if base_url_path_prefix and base_url_path_prefix != "/":
                    # we will now support base_url_path_prefix. This means that the user can provide base_url like http://example.com/path and we will extract /path as base_url_path_prefix
                    model_provider["base_url_path_prefix"] = base_url_path_prefix

                if urlparse_result.scheme == "" or urlparse_result.scheme not in [
                    "http",
                    "https",
                ]:
                    raise Exception(
                        "Please provide a valid URL with scheme (http/https) in base_url"
                    )
                protocol = urlparse_result.scheme
                port = urlparse_result.port
                if port is None:
                    if protocol == "http":
                        port = 80
                    else:
                        port = 443
                endpoint = urlparse_result.hostname
                model_provider["endpoint"] = endpoint
                model_provider["port"] = port
                model_provider["protocol"] = protocol
                cluster_name = (
                    provider + "_" + endpoint
                )  # make name unique by appending endpoint
                model_provider["cluster_name"] = cluster_name
                # Only add if cluster_name is not already present to avoid duplicates
                if cluster_name not in llms_with_endpoint_cluster_names:
                    llms_with_endpoint.append(model_provider)
                    llms_with_endpoint_cluster_names.add(cluster_name)

    overrides_config = config_yaml.get("overrides", {})
    # Build lookup of model names (already prefix-stripped by config processing)
    model_name_set = {mp.get("model") for mp in updated_model_providers}

    # Auto-add plano-orchestrator provider if routing preferences exist and no provider matches the routing model
    router_model = overrides_config.get("llm_routing_model", "Plano-Orchestrator")
    router_model_id = (
        router_model.split("/", 1)[1] if "/" in router_model else router_model
    )
    if len(seen_pref_names) > 0 and router_model_id not in model_name_set:
        updated_model_providers.append(
            {
                "name": "plano-orchestrator",
                "provider_interface": "plano",
                "model": router_model_id,
                "internal": True,
            }
        )

    # Always add arch-function model provider if not already defined
    if "arch-function" not in model_provider_name_set:
        updated_model_providers.append(
            {
                "name": "arch-function",
                "provider_interface": "plano",
                "model": "Arch-Function",
                "internal": True,
            }
        )

    # Auto-add plano-orchestrator provider if no provider matches the orchestrator model
    orchestrator_model = overrides_config.get(
        "agent_orchestration_model", "Plano-Orchestrator"
    )
    orchestrator_model_id = (
        orchestrator_model.split("/", 1)[1]
        if "/" in orchestrator_model
        else orchestrator_model
    )
    if orchestrator_model_id not in model_name_set:
        updated_model_providers.append(
            {
                "name": "plano/orchestrator",
                "provider_interface": "plano",
                "model": orchestrator_model_id,
                "internal": True,
            }
        )

    config_yaml["model_providers"] = deepcopy(updated_model_providers)

    listeners_with_provider = 0
    for listener in listeners:
        print("Processing listener: ", listener)
        model_providers = listener.get("model_providers", None)
        if model_providers is not None:
            listeners_with_provider += 1
            if listeners_with_provider > 1:
                raise Exception(
                    "Please provide model_providers either under listeners or at root level, not both. Currently we don't support multiple listeners with model_providers"
                )

    # Validate input_filters IDs on listeners reference valid agent/filter IDs
    for listener in listeners:
        listener_input_filters = listener.get("input_filters", [])
        for fc_id in listener_input_filters:
            if fc_id not in agent_id_keys:
                raise Exception(
                    f"Listener '{listener.get('name', 'unknown')}' references input_filters id '{fc_id}' "
                    f"which is not defined in agents or filters. Available ids: {', '.join(sorted(agent_id_keys))}"
                )

    # Validate model aliases if present
    if "model_aliases" in config_yaml:
        model_aliases = config_yaml["model_aliases"]
        for alias_name, alias_config in model_aliases.items():
            target = alias_config.get("target")
            if target not in model_name_keys:
                raise Exception(
                    f"Model alias 2 - '{alias_name}' targets '{target}' which is not defined as a model. Available models: {', '.join(sorted(model_name_keys))}"
                )

    plano_config_string = yaml.dump(config_yaml)
    plano_llm_config_string = yaml.dump(config_yaml)

    use_agent_orchestrator = config_yaml.get("overrides", {}).get(
        "use_agent_orchestrator", False
    )

    agent_orchestrator = None
    if use_agent_orchestrator:
        print("Using agent orchestrator")

        if len(endpoints) == 0:
            raise Exception(
                "Please provide agent orchestrator in the endpoints section in your plano_config.yaml file"
            )
        elif len(endpoints) > 1:
            raise Exception(
                "Please provide single agent orchestrator in the endpoints section in your plano_config.yaml file"
            )
        else:
            agent_orchestrator = list(endpoints.keys())[0]

    print("agent_orchestrator: ", agent_orchestrator)

    overrides = config_yaml.get("overrides", {})
    upstream_connect_timeout = overrides.get("upstream_connect_timeout", "5s")
    upstream_tls_ca_path = overrides.get(
        "upstream_tls_ca_path", "/etc/ssl/certs/ca-certificates.crt"
    )

    data = {
        "prompt_gateway_listener": prompt_gateway,
        "llm_gateway_listener": llm_gateway,
        "plano_config": plano_config_string,
        "plano_llm_config": plano_llm_config_string,
        "plano_clusters": inferred_clusters,
        "plano_model_providers": updated_model_providers,
        "plano_tracing": plano_tracing,
        "local_llms": llms_with_endpoint,
        "agent_orchestrator": agent_orchestrator,
        "listeners": listeners,
        "upstream_connect_timeout": upstream_connect_timeout,
        "upstream_tls_ca_path": upstream_tls_ca_path,
    }

    rendered = template.render(data)
    print(ENVOY_CONFIG_FILE_RENDERED)
    print(rendered)
    with open(ENVOY_CONFIG_FILE_RENDERED, "w") as file:
        file.write(rendered)

    with open(PLANO_CONFIG_FILE_RENDERED, "w") as file:
        file.write(plano_config_string)


def validate_prompt_config(plano_config_file, plano_config_schema_file):
    with open(plano_config_file, "r") as file:
        plano_config = file.read()

    with open(plano_config_schema_file, "r") as file:
        plano_config_schema = file.read()

    config_yaml = yaml.safe_load(plano_config)
    config_schema_yaml = yaml.safe_load(plano_config_schema)

    try:
        validate(config_yaml, config_schema_yaml)
    except ValidationError as e:
        path = (
            " → ".join(str(p) for p in e.absolute_path) if e.absolute_path else "root"
        )
        raise ValidationError(
            f"{e.message}\n  Location: {path}\n  Value: {e.instance}"
        ) from None
    except Exception as e:
        raise


if __name__ == "__main__":
    validate_and_render_schema()
