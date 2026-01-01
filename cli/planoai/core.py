import json
import subprocess
import os
import time
import sys

import yaml
from planoai.utils import convert_legacy_listeners, getLogger
from planoai.consts import (
    PLANO_DOCKER_IMAGE,
    PLANO_DOCKER_NAME,
)
import subprocess
from planoai.docker_cli import (
    docker_container_status,
    docker_remove_container,
    docker_start_plano_detached,
    docker_stop_container,
    health_check_endpoint,
    stream_gateway_logs,
)


log = getLogger(__name__)


def _get_gateway_ports(arch_config_file: str) -> list[int]:
    PROMPT_GATEWAY_DEFAULT_PORT = 10000
    LLM_GATEWAY_DEFAULT_PORT = 12000

    # parse arch_config_file yaml file and get prompt_gateway_port
    arch_config_dict = {}
    with open(arch_config_file) as f:
        arch_config_dict = yaml.safe_load(f)

    listeners, _, _ = convert_legacy_listeners(
        arch_config_dict.get("listeners"), arch_config_dict.get("llm_providers")
    )

    all_ports = [listener.get("port") for listener in listeners]

    # unique ports
    all_ports = list(set(all_ports))

    return all_ports


def start_arch(arch_config_file, env, log_timeout=120, foreground=False):
    """
    Start Docker Compose in detached mode and stream logs until services are healthy.

    Args:
        path (str): The path where the prompt_config.yml file is located.
        log_timeout (int): Time in seconds to show logs before checking for healthy state.
    """
    log.info(
        f"Starting arch gateway, image name: {PLANO_DOCKER_NAME}, tag: {PLANO_DOCKER_IMAGE}"
    )

    try:
        plano_container_status = docker_container_status(PLANO_DOCKER_NAME)
        if plano_container_status != "not found":
            log.info("plano found in docker, stopping and removing it")
            docker_stop_container(PLANO_DOCKER_NAME)
            docker_remove_container(PLANO_DOCKER_NAME)

        gateway_ports = _get_gateway_ports(arch_config_file)

        return_code, _, plano_stderr = docker_start_plano_detached(
            arch_config_file,
            env,
            gateway_ports,
        )
        if return_code != 0:
            log.info("Failed to start plano gateway: " + str(return_code))
            log.info("stderr: " + plano_stderr)
            sys.exit(1)

        start_time = time.time()
        while True:
            all_listeners_healthy = True
            for port in gateway_ports:
                health_check_status = health_check_endpoint(
                    f"http://localhost:{port}/healthz"
                )
                if not health_check_status:
                    all_listeners_healthy = False

            plano_status = docker_container_status(PLANO_DOCKER_NAME)
            current_time = time.time()
            elapsed_time = current_time - start_time

            if plano_status == "exited":
                log.info("plano container exited unexpectedly.")
                stream_gateway_logs(follow=False)
                sys.exit(1)

            # Check if timeout is reached
            if elapsed_time > log_timeout:
                log.info(f"stopping log monitoring after {log_timeout} seconds.")
                stream_gateway_logs(follow=False)
                sys.exit(1)

            if all_listeners_healthy:
                log.info("plano is running and is healthy!")
                break
            else:
                health_check_status_str = (
                    "healthy" if health_check_status else "not healthy"
                )
                log.info(
                    f"plano status: {plano_status}, health status: {health_check_status_str}"
                )
                time.sleep(1)

        if foreground:
            stream_gateway_logs(follow=True)

    except KeyboardInterrupt:
        log.info("Keyboard interrupt received, stopping arch gateway service.")
        stop_docker_container()


def stop_docker_container(service=PLANO_DOCKER_NAME):
    """
    Shutdown all Docker Compose services by running `docker-compose down`.

    Args:
        path (str): The path where the docker-compose.yml file is located.
    """
    log.info(f"Shutting down {service} service.")

    try:
        subprocess.run(
            ["docker", "stop", service],
        )
        subprocess.run(
            ["docker", "rm", service],
        )

        log.info(f"Successfully shut down {service} service.")

    except subprocess.CalledProcessError as e:
        log.info(f"Failed to shut down services: {str(e)}")


def start_cli_agent(arch_config_file=None, settings_json="{}"):
    """Start a CLI client connected to Arch."""

    with open(arch_config_file, "r") as file:
        arch_config = file.read()
        arch_config_yaml = yaml.safe_load(arch_config)

    # Get egress listener configuration
    egress_config = arch_config_yaml.get("listeners", {}).get("egress_traffic", {})
    host = egress_config.get("host", "127.0.0.1")
    port = egress_config.get("port", 12000)

    # Parse additional settings from command line
    try:
        additional_settings = json.loads(settings_json) if settings_json else {}
    except json.JSONDecodeError:
        log.error("Settings must be valid JSON")
        sys.exit(1)

    # Set up environment variables
    env = os.environ.copy()
    env.update(
        {
            "ANTHROPIC_AUTH_TOKEN": "test",  # Use test token for arch
            "ANTHROPIC_API_KEY": "",
            "ANTHROPIC_BASE_URL": f"http://{host}:{port}",
            "NO_PROXY": host,
            "DISABLE_TELEMETRY": "true",
            "DISABLE_COST_WARNINGS": "true",
            "API_TIMEOUT_MS": "600000",
        }
    )

    # Set ANTHROPIC_SMALL_FAST_MODEL from additional_settings or model alias
    if "ANTHROPIC_SMALL_FAST_MODEL" in additional_settings:
        env["ANTHROPIC_SMALL_FAST_MODEL"] = additional_settings[
            "ANTHROPIC_SMALL_FAST_MODEL"
        ]
    else:
        # Check if arch.claude.code.small.fast alias exists in model_aliases
        model_aliases = arch_config_yaml.get("model_aliases", {})
        if "arch.claude.code.small.fast" in model_aliases:
            env["ANTHROPIC_SMALL_FAST_MODEL"] = "arch.claude.code.small.fast"
        else:
            log.info(
                "Tip: Set an alias 'arch.claude.code.small.fast' in your model_aliases config to set a small fast model Claude Code"
            )
            log.info("Or provide ANTHROPIC_SMALL_FAST_MODEL in --settings JSON")

    # Non-interactive mode configuration from additional_settings only
    if additional_settings.get("NON_INTERACTIVE_MODE", False):
        env.update(
            {
                "CI": "true",
                "FORCE_COLOR": "0",
                "NODE_NO_READLINE": "1",
                "TERM": "dumb",
            }
        )

    # Build claude command arguments
    claude_args = []

    # Add settings if provided, excluding those already handled as environment variables
    if additional_settings:
        # Filter out settings that are already processed as environment variables
        claude_settings = {
            k: v
            for k, v in additional_settings.items()
            if k not in ["ANTHROPIC_SMALL_FAST_MODEL", "NON_INTERACTIVE_MODE"]
        }
        if claude_settings:
            claude_args.append(f"--settings={json.dumps(claude_settings)}")

    # Use claude from PATH
    claude_path = "claude"
    log.info(f"Connecting Claude Code Agent to Arch at {host}:{port}")

    try:
        subprocess.run([claude_path] + claude_args, env=env, check=True)
    except subprocess.CalledProcessError as e:
        log.error(f"Error starting claude: {e}")
        sys.exit(1)
    except FileNotFoundError:
        log.error(
            f"{claude_path} not found. Make sure Claude Code is installed: npm install -g @anthropic-ai/claude-code"
        )
        sys.exit(1)
