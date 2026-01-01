import json
import os
from planoai.utils import convert_legacy_listeners
from jinja2 import Environment, FileSystemLoader
import yaml
from jsonschema import validate
from urllib.parse import urlparse
from copy import deepcopy


SUPPORTED_PROVIDERS_WITH_BASE_URL = [
    "azure_openai",
    "ollama",
    "qwen",
    "amazon_bedrock",
    "arch",
]

SUPPORTED_PROVIDERS_WITHOUT_BASE_URL = [
    "deepseek",
    "groq",
    "mistral",
    "openai",
    "gemini",
    "anthropic",
    "together_ai",
    "xai",
    "moonshotai",
    "zhipu",
]

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


def validate_and_render_schema():
    ENVOY_CONFIG_TEMPLATE_FILE = os.getenv(
        "ENVOY_CONFIG_TEMPLATE_FILE", "envoy.template.yaml"
    )
    ARCH_CONFIG_FILE = os.getenv("ARCH_CONFIG_FILE", "/app/arch_config.yaml")
    ARCH_CONFIG_FILE_RENDERED = os.getenv(
        "ARCH_CONFIG_FILE_RENDERED", "/app/arch_config_rendered.yaml"
    )
    ENVOY_CONFIG_FILE_RENDERED = os.getenv(
        "ENVOY_CONFIG_FILE_RENDERED", "/etc/envoy/envoy.yaml"
    )
    ARCH_CONFIG_SCHEMA_FILE = os.getenv(
        "ARCH_CONFIG_SCHEMA_FILE", "arch_config_schema.yaml"
    )

    env = Environment(loader=FileSystemLoader(os.getenv("TEMPLATE_ROOT", "./")))
    template = env.get_template(ENVOY_CONFIG_TEMPLATE_FILE)

    try:
        validate_prompt_config(ARCH_CONFIG_FILE, ARCH_CONFIG_SCHEMA_FILE)
    except Exception as e:
        print(str(e))
        exit(1)  # validate_prompt_config failed. Exit

    with open(ARCH_CONFIG_FILE, "r") as file:
        arch_config = file.read()

    with open(ARCH_CONFIG_SCHEMA_FILE, "r") as file:
        arch_config_schema = file.read()

    config_yaml = yaml.safe_load(arch_config)
    _ = yaml.safe_load(arch_config_schema)
    inferred_clusters = {}

    # Convert legacy llm_providers to model_providers
    if "llm_providers" in config_yaml:
        if "model_providers" in config_yaml:
            raise Exception(
                "Please provide either llm_providers or model_providers, not both. llm_providers is deprecated, please use model_providers instead"
            )
        config_yaml["model_providers"] = config_yaml["llm_providers"]
        del config_yaml["llm_providers"]

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

    print("defined clusters from arch_config.yaml: ", json.dumps(inferred_clusters))

    if "prompt_targets" in config_yaml:
        for prompt_target in config_yaml["prompt_targets"]:
            name = prompt_target.get("endpoint", {}).get("name", None)
            if not name:
                continue
            if name not in inferred_clusters:
                raise Exception(
                    f"Unknown endpoint {name}, please add it in endpoints section in your arch_config.yaml file"
                )

    arch_tracing = config_yaml.get("tracing", {})

    llms_with_endpoint = []
    llms_with_endpoint_cluster_names = set()
    updated_model_providers = []
    model_provider_name_set = set()
    llms_with_usage = []
    model_name_keys = set()
    model_usage_name_keys = set()

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
            if model_name in model_name_keys:
                raise Exception(
                    f"Duplicate model name {model_name}, please provide unique model name for each model_provider"
                )
            model_name_keys.add(model_name)
            if model_provider.get("name") is None:
                model_provider["name"] = model_name

            model_provider_name_set.add(model_provider.get("name"))

            model_name_tokens = model_name.split("/")
            if len(model_name_tokens) < 2:
                raise Exception(
                    f"Invalid model name {model_name}. Please provide model name in the format <provider>/<model_id>."
                )
            provider = model_name_tokens[0]

            # Validate azure_openai and ollama provider requires base_url
            if (provider in SUPPORTED_PROVIDERS_WITH_BASE_URL) and model_provider.get(
                "base_url"
            ) is None:
                raise Exception(
                    f"Provider '{provider}' requires 'base_url' to be set for model {model_name}"
                )

            model_id = "/".join(model_name_tokens[1:])
            if provider not in SUPPORTED_PROVIDERS:
                if (
                    model_provider.get("base_url", None) is None
                    or model_provider.get("provider_interface", None) is None
                ):
                    raise Exception(
                        f"Must provide base_url and provider_interface for unsupported provider {provider} for model {model_name}. Supported providers are: {', '.join(SUPPORTED_PROVIDERS)}"
                    )
                provider = model_provider.get("provider_interface", None)
            elif model_provider.get("provider_interface", None) is not None:
                raise Exception(
                    f"Please provide provider interface as part of model name {model_name} using the format <provider>/<model_id>. For example, use 'openai/gpt-3.5-turbo' instead of 'gpt-3.5-turbo' "
                )

            if model_id in model_name_keys:
                raise Exception(
                    f"Duplicate model_id {model_id}, please provide unique model_id for each model_provider"
                )
            model_name_keys.add(model_id)

            for routing_preference in model_provider.get("routing_preferences", []):
                if routing_preference.get("name") in model_usage_name_keys:
                    raise Exception(
                        f"Duplicate routing preference name \"{routing_preference.get('name')}\", please provide unique name for each routing preference"
                    )
                model_usage_name_keys.add(routing_preference.get("name"))

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

    if len(model_usage_name_keys) > 0:
        routing_model_provider = config_yaml.get("routing", {}).get(
            "model_provider", None
        )
        if (
            routing_model_provider
            and routing_model_provider not in model_provider_name_set
        ):
            raise Exception(
                f"Routing model_provider {routing_model_provider} is not defined in model_providers"
            )
        if (
            routing_model_provider is None
            and "arch-router" not in model_provider_name_set
        ):
            updated_model_providers.append(
                {
                    "name": "arch-router",
                    "provider_interface": "arch",
                    "model": config_yaml.get("routing", {}).get("model", "Arch-Router"),
                }
            )

    # Always add arch-function model provider if not already defined
    if "arch-function" not in model_provider_name_set:
        updated_model_providers.append(
            {
                "name": "arch-function",
                "provider_interface": "arch",
                "model": "Arch-Function",
            }
        )

    if "plano-orchestrator" not in model_provider_name_set:
        updated_model_providers.append(
            {
                "name": "plano-orchestrator",
                "provider_interface": "arch",
                "model": "Plano-Orchestrator",
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

    # Validate model aliases if present
    if "model_aliases" in config_yaml:
        model_aliases = config_yaml["model_aliases"]
        for alias_name, alias_config in model_aliases.items():
            target = alias_config.get("target")
            if target not in model_name_keys:
                raise Exception(
                    f"Model alias 2 - '{alias_name}' targets '{target}' which is not defined as a model. Available models: {', '.join(sorted(model_name_keys))}"
                )

    arch_config_string = yaml.dump(config_yaml)
    arch_llm_config_string = yaml.dump(config_yaml)

    use_agent_orchestrator = config_yaml.get("overrides", {}).get(
        "use_agent_orchestrator", False
    )

    agent_orchestrator = None
    if use_agent_orchestrator:
        print("Using agent orchestrator")

        if len(endpoints) == 0:
            raise Exception(
                "Please provide agent orchestrator in the endpoints section in your arch_config.yaml file"
            )
        elif len(endpoints) > 1:
            raise Exception(
                "Please provide single agent orchestrator in the endpoints section in your arch_config.yaml file"
            )
        else:
            agent_orchestrator = list(endpoints.keys())[0]

    print("agent_orchestrator: ", agent_orchestrator)

    data = {
        "prompt_gateway_listener": prompt_gateway,
        "llm_gateway_listener": llm_gateway,
        "arch_config": arch_config_string,
        "arch_llm_config": arch_llm_config_string,
        "arch_clusters": inferred_clusters,
        "arch_model_providers": updated_model_providers,
        "arch_tracing": arch_tracing,
        "local_llms": llms_with_endpoint,
        "agent_orchestrator": agent_orchestrator,
        "listeners": listeners,
    }

    rendered = template.render(data)
    print(ENVOY_CONFIG_FILE_RENDERED)
    print(rendered)
    with open(ENVOY_CONFIG_FILE_RENDERED, "w") as file:
        file.write(rendered)

    with open(ARCH_CONFIG_FILE_RENDERED, "w") as file:
        file.write(arch_config_string)


def validate_prompt_config(arch_config_file, arch_config_schema_file):
    with open(arch_config_file, "r") as file:
        arch_config = file.read()

    with open(arch_config_schema_file, "r") as file:
        arch_config_schema = file.read()

    config_yaml = yaml.safe_load(arch_config)
    config_schema_yaml = yaml.safe_load(arch_config_schema)

    try:
        validate(config_yaml, config_schema_yaml)
    except Exception as e:
        print(
            f"Error validating arch_config file: {arch_config_file}, schema file: {arch_config_schema_file}, error: {e}"
        )
        raise e


if __name__ == "__main__":
    validate_and_render_schema()
