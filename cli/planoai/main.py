import os
import multiprocessing
import importlib.metadata
import subprocess
import sys
import rich_click as click
from planoai import targets

# Brand color - Plano purple
PLANO_COLOR = "#969FF4"
from planoai.docker_cli import (
    docker_validate_plano_schema,
    stream_gateway_logs,
    docker_container_status,
)
from planoai.utils import (
    getLogger,
    get_llm_provider_access_keys,
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
from planoai.trace_cmd import trace as trace_cmd
from planoai.consts import (
    DEFAULT_OTEL_TRACING_GRPC_ENDPOINT,
    PLANO_DOCKER_IMAGE,
    PLANO_DOCKER_NAME,
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

# PyPI package name for version checking
PYPI_PACKAGE_NAME = "planoai"
PYPI_URL = f"https://pypi.org/pypi/{PYPI_PACKAGE_NAME}/json"


def _configure_rich_click() -> None:
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
    click.rich_click.STYLE_HELPTEXT_FIRST_LINE = "white italic"
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
                "name": "Observability",
                "commands": ["trace"],
            },
            {
                "name": "Utilities",
                "commands": ["generate-prompt-targets"],
            },
        ],
    }


def _console():
    from rich.console import Console

    return Console()


def _print_cli_header(console) -> None:
    console.print(
        f"\n[bold {PLANO_COLOR}]Plano CLI[/bold {PLANO_COLOR}] [dim]v{get_version()}[/dim]\n"
    )


def _print_missing_keys(console, missing_keys: list[str]) -> None:
    console.print(f"\n[red]✗[/red] [red]Missing API keys![/red]\n")
    for key in missing_keys:
        console.print(f"  [red]•[/red] [bold]{key}[/bold] not found")
    console.print(f"\n[dim]Set the environment variable(s):[/dim]")
    for key in missing_keys:
        console.print(f'  [cyan]export {key}="your-api-key"[/cyan]')
    console.print(f"\n[dim]Or create a .env file in the config directory.[/dim]\n")


def _print_version(console, current_version: str) -> None:
    console.print(
        f"[bold {PLANO_COLOR}]plano[/bold {PLANO_COLOR}] version [cyan]{current_version}[/cyan]"
    )


def _maybe_check_updates(console, current_version: str) -> None:
    if os.environ.get("PLANO_SKIP_VERSION_CHECK"):
        return
    latest_version = get_latest_version()
    status = check_version_status(current_version, latest_version)

    if status["is_outdated"]:
        console.print(
            f"\n[yellow]⚠ Update available:[/yellow] [bold]{status['latest']}[/bold]"
        )
        console.print(f"[dim]Run: uv pip install --upgrade {PYPI_PACKAGE_NAME}[/dim]")
    elif latest_version:
        console.print(f"[dim]✓ You're up to date[/dim]")


def _build_table(title: str):
    from rich.table import Table

    return Table(
        title=title,
        border_style="dim",
        show_header=True,
        header_style=f"bold {PLANO_COLOR}",
    )


def _print_messages(console, items: list[str], template: str) -> None:
    for item in items:
        console.print(template.format(item=item))
    console.print()


def _print_section(console, title: str, lines: list[str]) -> None:
    console.print(title)
    for line in lines:
        console.print(line)
    console.print()


_configure_rich_click()


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
    console = _console()

    if version:
        current_version = get_version()
        _print_version(console, current_version)
        _maybe_check_updates(console, current_version)

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
    from rich.status import Status

    console = _console()
    _print_cli_header(console)

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
    ):
        (
            validation_return_code,
            _,
            validation_stderr,
        ) = docker_validate_plano_schema(arch_config_file)

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
        _print_missing_keys(console, missing_keys)
        sys.exit(1)

    # Pass log level to the Docker container — supervisord uses LOG_LEVEL
    # to set RUST_LOG (brightstaff) and envoy component log levels
    env_stage["LOG_LEVEL"] = os.environ.get("LOG_LEVEL", "info")

    env.update(env_stage)
    start_arch(arch_config_file, env, foreground=foreground)


@click.command()
def down():
    """Stops Plano."""
    console = _console()
    _print_cli_header(console)

    with console.status(
        f"[{PLANO_COLOR}]Shutting down Plano...[/{PLANO_COLOR}]", spinner="dots"
    ):
        stop_docker_container()


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


# add commands to the main group
main.add_command(up)
main.add_command(down)
main.add_command(build)
main.add_command(logs)
main.add_command(cli_agent)
main.add_command(generate_prompt_targets)
main.add_command(init_cmd, name="init")
main.add_command(trace_cmd, name="trace")

if __name__ == "__main__":
    main()
