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
        ["docker", "rm", "-f", container], capture_output=True, text=True, check=False
    )
    return result.returncode


def _get_host_ip() -> str:
    """Resolve the host machine's IP address reachable from inside Docker containers.

    Docker Desktop (Mac/Windows) supports the ``host-gateway`` special keyword, but
    Rancher Desktop and some Linux configurations do not.  As a more portable
    alternative we read the gateway of the default Docker bridge network, which is
    the same IP that ``host-gateway`` would resolve to on standard installations.

    Falls back to ``host-gateway`` so existing Docker Desktop setups are unaffected.
    """
    result = subprocess.run(
        [
            "docker",
            "network",
            "inspect",
            "bridge",
            "--format",
            "{{range .IPAM.Config}}{{.Gateway}}{{end}}",
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    ip = result.stdout.strip()
    if result.returncode == 0 and ip:
        return ip
    return "host-gateway"


def _prepare_docker_config(plano_config_file: str) -> str:
    """Copy config to a temp file, replacing localhost with host.docker.internal.

    Configs use localhost for native-first mode, but Docker containers need
    host.docker.internal to reach services on the host.
    """
    import tempfile

    with open(plano_config_file, "r") as f:
        content = f.read()

    if "localhost" not in content:
        return plano_config_file

    content = content.replace("localhost", "host.docker.internal")
    tmp = tempfile.NamedTemporaryFile(
        mode="w", suffix=".yaml", prefix="plano_config_", delete=False
    )
    tmp.write(content)
    tmp.close()
    return tmp.name


def docker_start_plano_detached(
    plano_config_file: str,
    env: dict,
    gateway_ports: list[int],
) -> str:
    docker_config = _prepare_docker_config(plano_config_file)
    env_args = [item for key, value in env.items() for item in ["-e", f"{key}={value}"]]

    port_mappings = [
        "12001:12001",
        "19901:9901",
    ]

    for port in gateway_ports:
        port_mappings.append(f"{port}:{port}")

    port_mappings_args = [item for port in port_mappings for item in ("-p", port)]

    volume_mappings = [
        f"{docker_config}:/app/plano_config.yaml:ro",
    ]
    volume_mappings_args = [
        item for volume in volume_mappings for item in ("-v", volume)
    ]

    host_ip = _get_host_ip()

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
        f"host.docker.internal:{host_ip}",
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


def docker_validate_plano_schema(plano_config_file):
    import os

    env = os.environ.copy()
    env.pop("PATH", None)
    env_args = [item for key, value in env.items() for item in ["-e", f"{key}={value}"]]

    result = subprocess.run(
        [
            "docker",
            "run",
            "--rm",
            *env_args,
            "-v",
            f"{plano_config_file}:/app/plano_config.yaml:ro",
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
