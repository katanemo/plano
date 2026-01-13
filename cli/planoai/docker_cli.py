import subprocess
import json
import sys
import requests

from planoai.consts import (
    PLANO_DOCKER_IMAGE,
    PLANO_DOCKER_NAME,
)
from planoai.utils import getLogger

log = getLogger(__name__)


def docker_container_status(container: str) -> str:
    result = subprocess.run(
        ["docker", "inspect", "--type=container", container],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        return "not found"

    container_status = json.loads(result.stdout)[0]
    return container_status.get("State", {}).get("Status", "")


def docker_stop_container(container: str) -> str:
    result = subprocess.run(
        ["docker", "stop", container], capture_output=True, text=True, check=False
    )
    return result.returncode


def docker_remove_container(container: str) -> str:
    result = subprocess.run(
        ["docker", "rm", container], capture_output=True, text=True, check=False
    )
    return result.returncode


def docker_start_plano_detached(
    arch_config_file: str,
    env: dict,
    gateway_ports: list[int],
) -> str:
    env_args = [item for key, value in env.items() for item in ["-e", f"{key}={value}"]]

    port_mappings = [
        f"{12001}:{12001}",
        "19901:9901",
    ]

    for port in gateway_ports:
        port_mappings.append(f"{port}:{port}")

    port_mappings_args = [item for port in port_mappings for item in ("-p", port)]

    volume_mappings = [
        f"{arch_config_file}:/app/arch_config.yaml:ro",
    ]
    volume_mappings_args = [
        item for volume in volume_mappings for item in ("-v", volume)
    ]

    options = [
        "docker",
        "run",
        "-d",
        "--name",
        PLANO_DOCKER_NAME,
        *port_mappings_args,
        *volume_mappings_args,
        *env_args,
        "--add-host",
        "host.docker.internal:host-gateway",
        PLANO_DOCKER_IMAGE,
    ]

    result = subprocess.run(options, capture_output=True, text=True, check=False)
    return result.returncode, result.stdout, result.stderr


def health_check_endpoint(endpoint: str) -> bool:
    try:
        response = requests.get(endpoint)
        if response.status_code == 200:
            return True
    except requests.RequestException as e:
        pass
    return False


def stream_gateway_logs(follow, service="plano"):
    """
    Stream logs from the plano gateway service.
    """
    log.info("Logs from plano gateway service.")

    options = ["docker", "logs"]
    if follow:
        options.append("-f")
    options.append(service)
    try:
        # Run `docker-compose logs` to stream logs from the gateway service
        subprocess.run(
            options,
            check=True,
            stdout=sys.stdout,
            stderr=sys.stderr,
        )

    except subprocess.CalledProcessError as e:
        log.info(f"Failed to stream logs: {str(e)}")


def docker_validate_plano_schema(arch_config_file):
    result = subprocess.run(
        [
            "docker",
            "run",
            "--rm",
            "-v",
            f"{arch_config_file}:/app/arch_config.yaml:ro",
            "--entrypoint",
            "python",
            PLANO_DOCKER_IMAGE,
            "-m",
            "planoai.config_generator",
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    return result.returncode, result.stdout, result.stderr
