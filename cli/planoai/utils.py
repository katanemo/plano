import glob
import os
import subprocess
import sys
import yaml
import logging
from planoai.consts import PLANO_DOCKER_NAME


logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - %(name)s - %(levelname)s - %(message)s",
)


def getLogger(name="cli"):
    logger = logging.getLogger(name)
    logger.setLevel(logging.INFO)
    return logger


log = getLogger(__name__)


def find_repo_root(start_path=None):
    """Find the repository root by looking for Dockerfile or .git directory."""
    if start_path is None:
        start_path = os.getcwd()

    current = os.path.abspath(start_path)

    while current != os.path.dirname(current):  # Stop at filesystem root
        # Check for markers that indicate repo root
        if (
            os.path.exists(os.path.join(current, "Dockerfile"))
            and os.path.exists(os.path.join(current, "crates"))
            and os.path.exists(os.path.join(current, "config"))
        ):
            return current

        # Also check for .git as fallback
        if os.path.exists(os.path.join(current, ".git")):
            # Verify it's the right repo by checking for expected structure
            if os.path.exists(os.path.join(current, "crates")):
                return current

        current = os.path.dirname(current)

    return None


def has_ingress_listener(arch_config_file):
    """Check if the arch config file has ingress_traffic listener configured."""
    try:
        with open(arch_config_file) as f:
            arch_config_dict = yaml.safe_load(f)

        ingress_traffic = arch_config_dict.get("listeners", {}).get(
            "ingress_traffic", {}
        )

        return bool(ingress_traffic)
    except Exception as e:
        log.error(f"Error reading config file {arch_config_file}: {e}")
        return False


def convert_legacy_listeners(
    listeners: dict | list, model_providers: list | None
) -> tuple[list, dict | None, dict | None]:
    llm_gateway_listener = {
        "name": "egress_traffic",
        "type": "model_listener",
        "port": 12000,
        "address": "0.0.0.0",
        "timeout": "30s",
        "model_providers": model_providers or [],
    }

    prompt_gateway_listener = {
        "name": "ingress_traffic",
        "type": "prompt_listener",
        "port": 10000,
        "address": "0.0.0.0",
        "timeout": "30s",
    }

    # Handle None case
    if listeners is None:
        return [llm_gateway_listener], llm_gateway_listener, prompt_gateway_listener

    if isinstance(listeners, dict):
        # legacy listeners
        # check if type is array or object
        # if its dict its legacy format let's convert it to array
        updated_listeners = []
        ingress_traffic = listeners.get("ingress_traffic", {})
        egress_traffic = listeners.get("egress_traffic", {})

        llm_gateway_listener["port"] = egress_traffic.get(
            "port", llm_gateway_listener["port"]
        )
        llm_gateway_listener["address"] = egress_traffic.get(
            "address", llm_gateway_listener["address"]
        )
        llm_gateway_listener["timeout"] = egress_traffic.get(
            "timeout", llm_gateway_listener["timeout"]
        )
        if model_providers is None or model_providers == []:
            raise ValueError("model_providers cannot be empty when using legacy format")

        llm_gateway_listener["model_providers"] = model_providers
        updated_listeners.append(llm_gateway_listener)

        if ingress_traffic and ingress_traffic != {}:
            prompt_gateway_listener["port"] = ingress_traffic.get(
                "port", prompt_gateway_listener["port"]
            )
            prompt_gateway_listener["address"] = ingress_traffic.get(
                "address", prompt_gateway_listener["address"]
            )
            prompt_gateway_listener["timeout"] = ingress_traffic.get(
                "timeout", prompt_gateway_listener["timeout"]
            )
            updated_listeners.append(prompt_gateway_listener)

        return updated_listeners, llm_gateway_listener, prompt_gateway_listener

    model_provider_set = False
    for listener in listeners:
        if listener.get("type") == "model_listener":
            if model_provider_set:
                raise ValueError(
                    "Currently only one listener can have model_providers set"
                )
            listener["model_providers"] = model_providers or []
            model_provider_set = True
            llm_gateway_listener = listener
    if not model_provider_set:
        listeners.append(llm_gateway_listener)

    return listeners, llm_gateway_listener, prompt_gateway_listener


def get_llm_provider_access_keys(arch_config_file):
    with open(arch_config_file, "r") as file:
        arch_config = file.read()
        arch_config_yaml = yaml.safe_load(arch_config)

    access_key_list = []

    # Convert legacy llm_providers to model_providers
    if "llm_providers" in arch_config_yaml:
        if "model_providers" in arch_config_yaml:
            raise Exception(
                "Please provide either llm_providers or model_providers, not both. llm_providers is deprecated, please use model_providers instead"
            )
        arch_config_yaml["model_providers"] = arch_config_yaml["llm_providers"]
        del arch_config_yaml["llm_providers"]

    listeners, _, _ = convert_legacy_listeners(
        arch_config_yaml.get("listeners"), arch_config_yaml.get("model_providers")
    )

    for prompt_target in arch_config_yaml.get("prompt_targets", []):
        for k, v in prompt_target.get("endpoint", {}).get("http_headers", {}).items():
            if k.lower() == "authorization":
                print(
                    f"found auth header: {k} for prompt_target: {prompt_target.get('name')}/{prompt_target.get('endpoint').get('name')}"
                )
                auth_tokens = v.split(" ")
                if len(auth_tokens) > 1:
                    access_key_list.append(auth_tokens[1])
                else:
                    access_key_list.append(v)

    for listener in listeners:
        for llm_provider in listener.get("model_providers", []):
            access_key = llm_provider.get("access_key")
            if access_key is not None:
                access_key_list.append(access_key)

    # Extract environment variables from state_storage.connection_string
    state_storage = arch_config_yaml.get("state_storage_v1_responses")
    if state_storage:
        connection_string = state_storage.get("connection_string")
        if connection_string and isinstance(connection_string, str):
            # Extract all $VAR and ${VAR} patterns from connection string
            import re

            # Match both $VAR and ${VAR} patterns
            pattern = r"\$\{?([A-Z_][A-Z0-9_]*)\}?"
            matches = re.findall(pattern, connection_string)
            for var in matches:
                access_key_list.append(f"${var}")
        else:
            raise ValueError(
                "Invalid connection string received in state_storage_v1_responses"
            )

    return access_key_list


def load_env_file_to_dict(file_path):
    env_dict = {}

    # Open and read the .env file
    with open(file_path, "r") as file:
        for line in file:
            # Strip any leading/trailing whitespaces
            line = line.strip()

            # Skip empty lines and comments
            if not line or line.startswith("#"):
                continue

            # Split the line into key and value at the first '=' sign
            if "=" in line:
                key, value = line.split("=", 1)
                key = key.strip()
                value = value.strip()

                # Add key-value pair to the dictionary
                env_dict[key] = value

    return env_dict


def find_config_file(path=".", file=None):
    """Find the appropriate config file path."""
    if file:
        # If a file is provided, process that file
        return os.path.abspath(file)
    else:
        # If no file is provided, use the path and look for arch_config.yaml first, then config.yaml for convenience
        arch_config_file = os.path.abspath(os.path.join(path, "config.yaml"))
        if not os.path.exists(arch_config_file):
            arch_config_file = os.path.abspath(os.path.join(path, "arch_config.yaml"))
        return arch_config_file


def stream_access_logs(follow):
    """
    Get the archgw access logs
    """

    follow_arg = "-f" if follow else ""

    stream_command = [
        "docker",
        "exec",
        PLANO_DOCKER_NAME,
        "sh",
        "-c",
        f"tail {follow_arg} /var/log/access_*.log",
    ]

    subprocess.run(
        stream_command,
        check=True,
        stdout=sys.stdout,
        stderr=sys.stderr,
    )
