import rich_click as click
import os
import sys
import subprocess
import multiprocessing
import importlib.metadata
import json
from planoai import targets

# Brand color - Plano purple
PLANO_COLOR = "#969FF4"

# Configure rich-click styling
click.rich_click.USE_RICH_MARKUP = True
click.rich_click.USE_MARKDOWN = False
click.rich_click.SHOW_ARGUMENTS = True
click.rich_click.GROUP_ARGUMENTS_OPTIONS = True
click.rich_click.STYLE_ERRORS_SUGGESTION = "dim italic"
click.rich_click.ERRORS_SUGGESTION = (
    "Try running the '--help' flag for more information."
)
click.rich_click.ERRORS_EPILOGUE = ""

# Custom colors matching Plano brand
click.rich_click.STYLE_OPTION = f"dim {PLANO_COLOR}"
click.rich_click.STYLE_ARGUMENT = f"dim {PLANO_COLOR}"
click.rich_click.STYLE_COMMAND = f"bold {PLANO_COLOR}"
click.rich_click.STYLE_SWITCH = "bold green"
click.rich_click.STYLE_METAVAR = "bold yellow"
click.rich_click.STYLE_USAGE = "bold"
click.rich_click.STYLE_USAGE_COMMAND = f"bold dim {PLANO_COLOR}"
click.rich_click.STYLE_HELPTEXT_FIRST_LINE = f"white italic"
click.rich_click.STYLE_HELPTEXT = ""
click.rich_click.STYLE_HEADER_TEXT = "bold"
click.rich_click.STYLE_FOOTER_TEXT = "dim"
click.rich_click.STYLE_OPTIONS_PANEL_BORDER = "dim"
click.rich_click.ALIGN_OPTIONS_PANEL = "left"
click.rich_click.MAX_WIDTH = 100

# Option groups for better organization
click.rich_click.OPTION_GROUPS = {
    "planoai up": [
        {
            "name": "Configuration",
            "options": ["--path", "file"],
        },
        {
            "name": "Runtime Options",
            "options": ["--foreground"],
        },
    ],
    "planoai logs": [
        {
            "name": "Log Options",
            "options": ["--debug", "--follow"],
        },
    ],
}

# Command groups for main help
click.rich_click.COMMAND_GROUPS = {
    "planoai": [
        {
            "name": "Gateway Commands",
            "commands": ["up", "down", "build", "logs"],
        },
        {
            "name": "Agent Commands",
            "commands": ["cli-agent"],
        },
        {
            "name": "Utilities",
            "commands": ["validate", "generate-prompt-targets"],
        },
    ],
}
from planoai.docker_cli import (
    docker_validate_plano_schema,
    stream_gateway_logs,
    docker_container_status,
)
from planoai.utils import (
    getLogger,
    get_llm_provider_access_keys,
    has_ingress_listener,
    load_env_file_to_dict,
    set_log_level,
    stream_access_logs,
    find_config_file,
    find_repo_root,
)
from planoai.core import (
    start_arch,
    stop_docker_container,
    start_cli_agent,
)
from planoai.init_cmd import init as init_cmd
from planoai.consts import (
    DEFAULT_OTEL_TRACING_GRPC_ENDPOINT,
    PLANO_DOCKER_IMAGE,
    PLANO_DOCKER_NAME,
    SERVICE_NAME_ARCHGW,
)

log = getLogger(__name__)

# ref https://patorjk.com/software/taag/#p=display&f=Doom&t=Plano&x=none&v=4&h=4&w=80&we=false
LOGO = f"""[bold {PLANO_COLOR}]
 ______ _
 | ___ \\ |
 | |_/ / | __ _ _ __   ___
 |  __/| |/ _` | '_ \\ / _ \\
 | |   | | (_| | | | | (_) |
 \\_|   |_|\\__,_|_| |_|\\___/
[/bold {PLANO_COLOR}]"""

# Command to build plano Docker images
ARCHGW_DOCKERFILE = "./Dockerfile"

# PyPI package name for version checking
PYPI_PACKAGE_NAME = "planoai"
PYPI_URL = f"https://pypi.org/pypi/{PYPI_PACKAGE_NAME}/json"


def get_version():
    try:
        # First try to get version from package metadata (for installed packages)
        version = importlib.metadata.version("planoai")
        return version
    except importlib.metadata.PackageNotFoundError:
        # Fallback to version defined in __init__.py (for development)
        try:
            from planoai import __version__

            return __version__
        except ImportError:
            return "version not found"


def get_latest_version(timeout: float = 2.0) -> str | None:
    """Fetch the latest version from PyPI.

    Args:
        timeout: Request timeout in seconds

    Returns:
        Latest version string or None if fetch failed
    """
    import requests

    try:
        response = requests.get(PYPI_URL, timeout=timeout)
        if response.status_code == 200:
            data = response.json()
            return data.get("info", {}).get("version")
    except (requests.RequestException, ValueError):
        # Network error or invalid JSON - fail silently
        pass
    return None


def parse_version(version_str: str) -> tuple:
    """Parse version string into comparable tuple.

    Handles versions like "0.4.1", "1.0.0", "0.4.1a1"
    """
    import re

    # Remove any pre-release suffixes for comparison
    clean_version = re.split(r"[a-zA-Z]", version_str)[0]
    parts = clean_version.split(".")
    return tuple(int(p) for p in parts if p.isdigit())


def check_version_status(current: str, latest: str | None) -> dict:
    """Compare current version with latest and return status.

    Returns:
        dict with keys: is_outdated, current, latest, message
    """
    if latest is None:
        return {
            "is_outdated": False,
            "current": current,
            "latest": None,
            "message": None,
        }

    try:
        current_tuple = parse_version(current)
        latest_tuple = parse_version(latest)
        is_outdated = current_tuple < latest_tuple

        return {
            "is_outdated": is_outdated,
            "current": current,
            "latest": latest,
            "message": f"Update available: {latest}" if is_outdated else None,
        }
    except (ValueError, TypeError):
        # Version parsing failed
        return {
            "is_outdated": False,
            "current": current,
            "latest": latest,
            "message": None,
        }


@click.group(invoke_without_command=True)
@click.option("--version", is_flag=True, help="Show the Plano CLI version and exit.")
@click.pass_context
def main(ctx, version):
    # Set log level from LOG_LEVEL env var only
    set_log_level(os.environ.get("LOG_LEVEL", "info"))
    from rich.console import Console

    console = Console()

    if version:
        current_version = get_version()
        console.print(
            f"[bold {PLANO_COLOR}]plano[/bold {PLANO_COLOR}] version [cyan]{current_version}[/cyan]"
        )

        # Check for updates (skip if PLANO_SKIP_VERSION_CHECK is set)
        if not os.environ.get("PLANO_SKIP_VERSION_CHECK"):
            latest_version = get_latest_version()
            status = check_version_status(current_version, latest_version)

            if status["is_outdated"]:
                console.print(
                    f"\n[yellow]⚠ Update available:[/yellow] [bold]{status['latest']}[/bold]"
                )
                console.print(
                    f"[dim]Run: uv pip install --upgrade {PYPI_PACKAGE_NAME}[/dim]"
                )
            elif latest_version:
                console.print(f"[dim]✓ You're up to date[/dim]")

        ctx.exit()

    if ctx.invoked_subcommand is None:
        console.print(LOGO)
        console.print("[dim]The Delivery Infrastructure for Agentic Apps[/dim]\n")
        click.echo(ctx.get_help())


@click.command()
def build():
    """Build Plano from source. Works from any directory within the repo."""

    # Find the repo root
    repo_root = find_repo_root()
    if not repo_root:
        click.echo(
            "Error: Could not find repository root. Make sure you're inside the plano repository."
        )
        sys.exit(1)

    dockerfile_path = os.path.join(repo_root, "Dockerfile")

    if not os.path.exists(dockerfile_path):
        click.echo(f"Error: Dockerfile not found at {dockerfile_path}")
        sys.exit(1)

    click.echo(f"Building plano image from {repo_root}...")
    try:
        subprocess.run(
            [
                "docker",
                "build",
                "-f",
                dockerfile_path,
                "-t",
                f"{PLANO_DOCKER_IMAGE}",
                repo_root,
                "--add-host=host.docker.internal:host-gateway",
            ],
            check=True,
        )
        click.echo("plano image built successfully.")
    except subprocess.CalledProcessError as e:
        click.echo(f"Error building plano image: {e}")
        sys.exit(1)


@click.command()
@click.argument("file", required=False)  # Optional file argument
@click.option(
    "--path", default=".", help="Path to the directory containing config.yaml"
)
@click.option(
    "--foreground",
    default=False,
    help="Run Plano in the foreground. Default is False",
    is_flag=True,
)
def up(file, path, foreground):
    """Starts Plano."""
    import time
    from rich.console import Console
    from rich.status import Status
    from planoai.docker_cli import (
        docker_container_status as get_container_status,
        docker_start_plano_detached,
        docker_stop_container as docker_stop,
        docker_remove_container,
        health_check_endpoint,
    )
    from planoai.core import _get_gateway_ports, stream_gateway_logs

    console = Console()

    # Print header
    console.print(
        f"\n[bold {PLANO_COLOR}]Plano CLI[/bold {PLANO_COLOR}] [dim]v{get_version()}[/dim]\n"
    )

    # Use the utility function to find config file
    arch_config_file = find_config_file(path, file)

    # Check if the file exists
    if not os.path.exists(arch_config_file):
        console.print(
            f"[red]✗[/red] Config file not found: [dim]{arch_config_file}[/dim]"
        )
        sys.exit(1)

    with Status(
        "[dim]Validating configuration[/dim]", spinner="dots", spinner_style="dim"
    ) as status:
        (
            validation_return_code,
            validation_stdout,
            validation_stderr,
        ) = docker_validate_plano_schema(arch_config_file)
        time.sleep(0.5)

    if validation_return_code != 0:
        console.print(f"[red]✗[/red] Validation failed")
        if validation_stderr:
            console.print(f"  [dim]{validation_stderr.strip()}[/dim]")
        sys.exit(1)

    console.print(f"[green]✓[/green] Configuration valid")

    # Set up environment
    env_stage = {
        "OTEL_TRACING_GRPC_ENDPOINT": DEFAULT_OTEL_TRACING_GRPC_ENDPOINT,
    }
    env = os.environ.copy()
    env.pop("PATH", None)

    # Check access keys
    access_keys = get_llm_provider_access_keys(arch_config_file=arch_config_file)
    access_keys = set(access_keys)
    access_keys = [item[1:] if item.startswith("$") else item for item in access_keys]

    missing_keys = []
    if access_keys:
        if file:
            app_env_file = os.path.join(os.path.dirname(os.path.abspath(file)), ".env")
        else:
            app_env_file = os.path.abspath(os.path.join(path, ".env"))

        if not os.path.exists(app_env_file):
            for access_key in access_keys:
                if env.get(access_key) is None:
                    missing_keys.append(access_key)
                else:
                    env_stage[access_key] = env.get(access_key)
        else:
            env_file_dict = load_env_file_to_dict(app_env_file)
            for access_key in access_keys:
                if env_file_dict.get(access_key) is None:
                    missing_keys.append(access_key)
                else:
                    env_stage[access_key] = env_file_dict[access_key]

    if missing_keys:
        console.print(f"\n[red]✗[/red] [red]Missing API keys![/red]\n")
        for key in missing_keys:
            console.print(f"  [red]•[/red] [bold]{key}[/bold] not found")
        console.print(f"\n[dim]Set the environment variable(s):[/dim]")
        for key in missing_keys:
            console.print(f'  [cyan]export {key}="your-api-key"[/cyan]')
        console.print(f"\n[dim]Or create a .env file in the config directory.[/dim]\n")
        sys.exit(1)

    # Pass log level to the Docker container — supervisord uses LOG_LEVEL
    # to set RUST_LOG (brightstaff) and envoy component log levels
    env_stage["LOG_LEVEL"] = os.environ.get("LOG_LEVEL", "info")

    env.update(env_stage)

    start_time = time.time()

    plano_status = get_container_status(PLANO_DOCKER_NAME)
    if plano_status != "not found":
        with console.status(
            f"[{PLANO_COLOR}]Stopping existing instance...[/{PLANO_COLOR}]",
            spinner="dots",
        ):
            docker_stop(PLANO_DOCKER_NAME)
            docker_remove_container(PLANO_DOCKER_NAME)
            time.sleep(1.0)

    gateway_ports = _get_gateway_ports(arch_config_file)

    max_retries = 3
    retry_delay = 2.0

    with console.status(
        f"[{PLANO_COLOR}]Starting Plano...[/{PLANO_COLOR}]", spinner="dots"
    ) as status:
        for attempt in range(max_retries):
            return_code, _, plano_stderr = docker_start_plano_detached(
                arch_config_file,
                env,
                gateway_ports,
            )

            if return_code == 0:
                break

            if "address already in use" in plano_stderr and attempt < max_retries - 1:
                plano_status = get_container_status(PLANO_DOCKER_NAME)
                if plano_status != "not found":
                    docker_stop(PLANO_DOCKER_NAME)
                    docker_remove_container(PLANO_DOCKER_NAME)

                time.sleep(retry_delay)
                continue
            console.print(f"[red]✗[/red] Failed to start Plano")
            if plano_stderr:
                console.print(f"  [dim]{plano_stderr.strip()}[/dim]")
            sys.exit(1)

        log_timeout = 120
        consecutive_failures = 0
        max_consecutive_failures = 10

        while True:
            plano_status = get_container_status(PLANO_DOCKER_NAME)
            elapsed = time.time() - start_time

            if plano_status == "exited":
                console.print(f"[red]✗[/red] Plano container exited unexpectedly")
                stream_gateway_logs(follow=False)
                sys.exit(1)

            if plano_status != "running":
                console.print(
                    f"[red]✗[/red] Plano container is not running (status: {plano_status})"
                )
                sys.exit(1)

            if elapsed > log_timeout:
                console.print(
                    f"[red]✗[/red] Timeout waiting for Plano to become healthy"
                )
                sys.exit(1)

            all_listeners_healthy = True
            for port in gateway_ports:
                if not health_check_endpoint(f"http://localhost:{port}/healthz"):
                    all_listeners_healthy = False
                    break

            if all_listeners_healthy:
                break

            consecutive_failures += 1
            if consecutive_failures >= max_consecutive_failures and elapsed > 5:
                console.print(
                    f"[red]✗[/red] Plano failed to become healthy after {elapsed:.1f}s"
                )
                console.print(
                    f"[dim]Check logs with: docker logs {PLANO_DOCKER_NAME}[/dim]"
                )
                sys.exit(1)

            status.update(
                f"[{PLANO_COLOR}]Starting Plano...[/{PLANO_COLOR}] [dim]({elapsed:.1f}s)[/dim]"
            )
            time.sleep(0.5)

    elapsed = time.time() - start_time
    console.print(
        f"[green]✓[/green] [bold]Plano is running and healthy![/bold] [dim]({elapsed:.1f}s)[/dim]"
    )

    console.print(f"\n[bold]Listening on:[/bold]")
    for port in gateway_ports:
        console.print(f"  [bold cyan]http://localhost:{port}[/bold cyan]")
    console.print()

    if foreground:
        console.print(f"[dim]Streaming logs (Ctrl+C to stop)...[/dim]\n")
        stream_gateway_logs(follow=True)


@click.command()
def down():
    """Stops Plano."""
    import time
    from rich.console import Console
    from planoai.docker_cli import (
        docker_container_status as get_container_status,
        docker_stop_container as docker_stop,
        docker_remove_container,
    )

    console = Console()

    # Print header
    console.print(
        f"\n[bold {PLANO_COLOR}]Plano CLI[/bold {PLANO_COLOR}] [dim]v{get_version()}[/dim]\n"
    )

    # Check if running
    plano_status = get_container_status(PLANO_DOCKER_NAME)

    if plano_status == "not found":
        console.print(f"[yellow]![/yellow] Plano is not running\n")
        return

    start_time = time.time()

    with console.status(
        f"[{PLANO_COLOR}]Shutting down Plano...[/{PLANO_COLOR}]", spinner="dots"
    ):
        docker_stop(PLANO_DOCKER_NAME)
        docker_remove_container(PLANO_DOCKER_NAME)

    elapsed = time.time() - start_time
    console.print(
        f"[green]✓[/green] Successfully shut down Plano! [dim]({elapsed:.1f}s)[/dim]\n"
    )


@click.command()
@click.option(
    "--f",
    "--file",
    type=click.Path(exists=True),
    required=True,
    help="Path to the Python file",
)
def generate_prompt_targets(file):
    """Generats prompt_targets from python methods.
    Note: This works for simple data types like ['int', 'float', 'bool', 'str', 'list', 'tuple', 'set', 'dict']:
    If you have a complex pydantic data type, you will have to flatten those manually until we add support for it.
    """

    print(f"Processing file: {file}")
    if not file.endswith(".py"):
        print("Error: Input file must be a .py file")
        sys.exit(1)

    targets.generate_prompt_targets(file)


@click.command()
@click.option(
    "--debug",
    help="For detailed debug logs to trace calls from plano <> api_server, etc",
    is_flag=True,
)
@click.option("--follow", help="Follow the logs", is_flag=True)
def logs(debug, follow):
    """Stream logs from access logs services."""

    archgw_process = None
    try:
        if debug:
            archgw_process = multiprocessing.Process(
                target=stream_gateway_logs, args=(follow,)
            )
            archgw_process.start()

        archgw_access_logs_process = multiprocessing.Process(
            target=stream_access_logs, args=(follow,)
        )
        archgw_access_logs_process.start()
        archgw_access_logs_process.join()

        if archgw_process:
            archgw_process.join()
    except KeyboardInterrupt:
        log.info("KeyboardInterrupt detected. Exiting.")
        if archgw_access_logs_process.is_alive():
            archgw_access_logs_process.terminate()
        if archgw_process and archgw_process.is_alive():
            archgw_process.terminate()


@click.command()
@click.argument("type", type=click.Choice(["claude"]), required=True)
@click.argument("file", required=False)  # Optional file argument
@click.option(
    "--path", default=".", help="Path to the directory containing arch_config.yaml"
)
@click.option(
    "--settings",
    default="{}",
    help="Additional settings as JSON string for the CLI agent.",
)
def cli_agent(type, file, path, settings):
    """Start a CLI agent connected to Plano.

    CLI_AGENT: The type of CLI agent to start (currently only 'claude' is supported)
    """

    # Check if plano docker container is running
    archgw_status = docker_container_status(PLANO_DOCKER_NAME)
    if archgw_status != "running":
        log.error(f"plano docker container is not running (status: {archgw_status})")
        log.error("Please start plano using the 'planoai up' command.")
        sys.exit(1)

    # Determine arch_config.yaml path
    arch_config_file = find_config_file(path, file)
    if not os.path.exists(arch_config_file):
        log.error(f"Config file not found: {arch_config_file}")
        sys.exit(1)

    try:
        start_cli_agent(arch_config_file, settings)
    except SystemExit:
        # Re-raise SystemExit to preserve exit codes
        raise
    except Exception as e:
        click.echo(f"Error: {e}")
        sys.exit(1)


def validate_config_file(config_path: str) -> dict:
    """Validate a Plano config file and return validation results.

    Args:
        config_path: Path to the config file

    Returns:
        dict with keys: valid, errors, warnings, config, summary
    """
    import yaml
    from jsonschema import validate as json_validate, ValidationError

    result = {
        "valid": True,
        "errors": [],
        "warnings": [],
        "config": None,
        "summary": {
            "model_providers": [],
            "listeners": [],
            "env_vars_required": [],
        },
    }

    # Check file exists
    if not os.path.exists(config_path):
        result["valid"] = False
        result["errors"].append(f"Config file not found: {config_path}")
        return result

    # Try to load YAML
    try:
        with open(config_path, "r") as f:
            config = yaml.safe_load(f)
        result["config"] = config
    except yaml.YAMLError as e:
        result["valid"] = False
        result["errors"].append(f"Invalid YAML syntax: {e}")
        return result

    # Find schema file
    schema_path = find_repo_root()
    if schema_path:
        schema_file = os.path.join(schema_path, "config", "arch_config_schema.yaml")
    else:
        # Fallback - try relative paths
        schema_file = None
        for possible_path in [
            "../config/arch_config_schema.yaml",
            "config/arch_config_schema.yaml",
        ]:
            if os.path.exists(possible_path):
                schema_file = possible_path
                break

    # Schema validation
    if schema_file and os.path.exists(schema_file):
        try:
            with open(schema_file, "r") as f:
                schema = yaml.safe_load(f)
            json_validate(config, schema)
        except ValidationError as e:
            result["valid"] = False
            result["errors"].append(f"Schema validation failed: {e.message}")
        except Exception as e:
            result["warnings"].append(f"Could not validate schema: {e}")
    else:
        result["warnings"].append("Schema file not found, skipping schema validation")

    # Extract model providers
    model_providers = config.get("model_providers", config.get("llm_providers", []))
    for provider in model_providers:
        model = provider.get("model", "unknown")
        is_default = provider.get("default", False)
        result["summary"]["model_providers"].append(
            {
                "model": model,
                "default": is_default,
                "name": provider.get("name", model),
            }
        )

        # Check for env vars
        access_key = provider.get("access_key", "")
        if access_key.startswith("$"):
            env_var = access_key[1:]
            result["summary"]["env_vars_required"].append(env_var)

    # Extract listeners
    listeners = config.get("listeners", {})
    if isinstance(listeners, dict):
        # Legacy format
        if "egress_traffic" in listeners:
            result["summary"]["listeners"].append(
                {
                    "name": "egress_traffic (LLM Gateway)",
                    "port": listeners["egress_traffic"].get("port", 12000),
                    "type": "model",
                }
            )
        if "ingress_traffic" in listeners:
            result["summary"]["listeners"].append(
                {
                    "name": "ingress_traffic (Prompt Gateway)",
                    "port": listeners["ingress_traffic"].get("port", 10000),
                    "type": "prompt",
                }
            )
    elif isinstance(listeners, list):
        for listener in listeners:
            result["summary"]["listeners"].append(
                {
                    "name": listener.get("name", "unnamed"),
                    "port": listener.get("port", "unknown"),
                    "type": listener.get("type", "unknown"),
                }
            )

    # Remove duplicates from env vars first
    result["summary"]["env_vars_required"] = list(
        set(result["summary"]["env_vars_required"])
    )

    # Check environment variables (after deduplication)
    for env_var in result["summary"]["env_vars_required"]:
        if not os.environ.get(env_var):
            result["warnings"].append(f"Environment variable ${env_var} is not set")

    return result


@click.command()
@click.argument("config_file", required=False, type=click.Path())
@click.option(
    "--path", "-p", default=".", help="Path to directory containing config.yaml"
)
@click.option("--quiet", "-q", is_flag=True, help="Only show errors, no summary")
def validate(config_file, path, quiet):
    """Validate a Plano configuration file.

    If no CONFIG_FILE is provided, looks for config.yaml in the current directory
    or the directory specified by --path.
    """
    from rich.console import Console
    from rich.table import Table
    from rich.panel import Panel

    console = Console()

    # Determine config file path
    if config_file:
        config_path = os.path.abspath(config_file)
    else:
        # Look for config.yaml in path
        config_path = os.path.join(os.path.abspath(path), "config.yaml")
        if not os.path.exists(config_path):
            # Try arch_config.yaml as fallback
            config_path = os.path.join(os.path.abspath(path), "arch_config.yaml")

    # Show what we're validating
    console.print(f"\n[bold]Validating:[/bold] [dim]{config_path}[/dim]\n")

    # Run validation
    result = validate_config_file(config_path)

    # Display results
    if result["valid"]:
        console.print(
            f"[bold green]✓[/bold green] [green]Configuration is valid[/green]\n"
        )
    else:
        console.print(f"[bold red]✗[/bold red] [red]Configuration is invalid[/red]\n")

    # Show errors
    if result["errors"]:
        for error in result["errors"]:
            console.print(f"  [red]✗ {error}[/red]")
        console.print()

    # Show warnings
    if result["warnings"]:
        for warning in result["warnings"]:
            console.print(f"  [yellow]⚠ {warning}[/yellow]")
        console.print()

    # Show summary (unless quiet mode)
    if not quiet and result["config"]:
        summary = result["summary"]

        # Model Providers table
        if summary["model_providers"]:
            table = Table(
                title=f"[bold {PLANO_COLOR}]Model Providers[/bold {PLANO_COLOR}]",
                border_style="dim",
                show_header=True,
                header_style=f"bold {PLANO_COLOR}",
            )
            table.add_column("Model", style="cyan")
            table.add_column("Default", style="green", justify="center")

            for provider in summary["model_providers"]:
                default_marker = "●" if provider["default"] else ""
                table.add_row(provider["model"], default_marker)

            console.print(table)
            console.print()

        # Listeners table
        if summary["listeners"]:
            table = Table(
                title=f"[bold {PLANO_COLOR}]Listeners[/bold {PLANO_COLOR}]",
                border_style="dim",
                show_header=True,
                header_style=f"bold {PLANO_COLOR}",
            )
            table.add_column("Name", style="cyan")
            table.add_column("Type", style="magenta")
            table.add_column("Port", style="yellow", justify="right")

            for listener in summary["listeners"]:
                table.add_row(listener["name"], listener["type"], str(listener["port"]))

            console.print(table)
            console.print()

        # Environment variables
        if summary["env_vars_required"]:
            env_status = []
            for env_var in sorted(summary["env_vars_required"]):
                is_set = os.environ.get(env_var) is not None
                status = f"[green]✓[/green]" if is_set else f"[yellow]○[/yellow]"
                env_status.append(f"  {status} [dim]${env_var}[/dim]")

            console.print(
                f"[bold {PLANO_COLOR}]Environment Variables[/bold {PLANO_COLOR}]"
            )
            for line in env_status:
                console.print(line)
            console.print()

    # Exit with appropriate code
    if not result["valid"]:
        sys.exit(1)


main.add_command(up)
main.add_command(down)
main.add_command(build)
main.add_command(logs)
main.add_command(cli_agent)
main.add_command(generate_prompt_targets)
main.add_command(validate)
main.add_command(init_cmd, name="init")

if __name__ == "__main__":
    main()
