import os
import multiprocessing
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
    start_plano,
    stop_docker_container,
    start_cli_agent,
)
from planoai.init_cmd import init as init_cmd
from planoai.trace_cmd import trace as trace_cmd, start_trace_listener_background
from planoai.consts import (
    DEFAULT_OTEL_TRACING_GRPC_ENDPOINT,
    PLANO_DOCKER_IMAGE,
    PLANO_DOCKER_NAME,
)
from planoai.rich_click_config import configure_rich_click
from planoai.versioning import check_version_status, get_latest_version, get_version

log = getLogger(__name__)


def _is_port_in_use(port: int) -> bool:
    """Check if a TCP port is already bound on localhost."""
    import socket

    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        try:
            s.bind(("0.0.0.0", port))  # noqa: S104
            return False
        except OSError:
            return True


# ref https://patorjk.com/software/taag/#p=display&f=Doom&t=Plano&x=none&v=4&h=4&w=80&we=false
LOGO = f"""[bold {PLANO_COLOR}]
 ______ _
 | ___ \\ |
 | |_/ / | __ _ _ __   ___
 |  __/| |/ _` | '_ \\ / _ \\
 | |   | | (_| | | | | (_) |
 \\_|   |_|\\__,_|_| |_|\\___/
[/bold {PLANO_COLOR}]"""


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
        console.print("[dim]Run: uv pip install --upgrade planoai[/dim]")
    elif latest_version:
        console.print(f"[dim]✓ You're up to date[/dim]")


configure_rich_click(PLANO_COLOR)


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
@click.option(
    "--with-tracing",
    default=False,
    help="Start a local OTLP trace collector on port 4317.",
    is_flag=True,
)
@click.option(
    "--tracing-port",
    default=4317,
    type=int,
    help="Port for the OTLP trace collector (default: 4317).",
    show_default=True,
)
def up(file, path, foreground, with_tracing, tracing_port):
    """Starts Plano."""
    from rich.status import Status

    console = _console()
    _print_cli_header(console)

    # Use the utility function to find config file
    plano_config_file = find_config_file(path, file)

    # Check if the file exists
    if not os.path.exists(plano_config_file):
        console.print(
            f"[red]✗[/red] Config file not found: [dim]{plano_config_file}[/dim]"
        )
        sys.exit(1)

    with Status(
        "[dim]Validating configuration[/dim]", spinner="dots", spinner_style="dim"
    ):
        (
            validation_return_code,
            _,
            validation_stderr,
        ) = docker_validate_plano_schema(plano_config_file)

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
    access_keys = get_llm_provider_access_keys(plano_config_file=plano_config_file)
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

    # Start the local OTLP trace collector if --with-tracing is set
    trace_server = None
    if with_tracing:
        if _is_port_in_use(tracing_port):
            # A listener is already running (e.g. `planoai trace listen`)
            console.print(
                f"[green]✓[/green] Trace collector already running on port [cyan]{tracing_port}[/cyan]"
            )
        else:
            try:
                trace_server = start_trace_listener_background(grpc_port=tracing_port)
                console.print(
                    f"[green]✓[/green] Trace collector listening on [cyan]0.0.0.0:{tracing_port}[/cyan]"
                )
            except Exception as e:
                console.print(
                    f"[red]✗[/red] Failed to start trace collector on port {tracing_port}: {e}"
                )
                console.print(
                    f"\n[dim]Check if another process is using port {tracing_port}:[/dim]"
                )
                console.print(f"  [cyan]lsof -i :{tracing_port}[/cyan]")
                console.print(f"\n[dim]Or use a different port:[/dim]")
                console.print(
                    f"  [cyan]planoai up --with-tracing --tracing-port 4318[/cyan]\n"
                )
                sys.exit(1)

        # Update the OTEL endpoint so the gateway sends traces to the right port
        env_stage[
            "OTEL_TRACING_GRPC_ENDPOINT"
        ] = f"http://host.docker.internal:{tracing_port}"

    env.update(env_stage)
    try:
        start_plano(plano_config_file, env, foreground=foreground)

        # When tracing is enabled but --foreground is not, keep the process
        # alive so the OTLP collector continues to receive spans.
        if trace_server is not None and not foreground:
            console.print(
                f"[dim]Plano is running. Trace collector active on port {tracing_port}. Press Ctrl+C to stop.[/dim]"
            )
            trace_server.wait_for_termination()
    except KeyboardInterrupt:
        if trace_server is not None:
            console.print(f"\n[dim]Stopping trace collector...[/dim]")
    finally:
        if trace_server is not None:
            trace_server.stop(grace=2)


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

    plano_process = None
    try:
        if debug:
            plano_process = multiprocessing.Process(
                target=stream_gateway_logs, args=(follow,)
            )
            plano_process.start()

        plano_access_logs_process = multiprocessing.Process(
            target=stream_access_logs, args=(follow,)
        )
        plano_access_logs_process.start()
        plano_access_logs_process.join()

        if plano_process:
            plano_process.join()
    except KeyboardInterrupt:
        log.info("KeyboardInterrupt detected. Exiting.")
        if plano_access_logs_process.is_alive():
            plano_access_logs_process.terminate()
        if plano_process and plano_process.is_alive():
            plano_process.terminate()


@click.command()
@click.argument("type", type=click.Choice(["claude"]), required=True)
@click.argument("file", required=False)  # Optional file argument
@click.option(
    "--path", default=".", help="Path to the directory containing plano_config.yaml"
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
    plano_status = docker_container_status(PLANO_DOCKER_NAME)
    if plano_status != "running":
        log.error(f"plano docker container is not running (status: {plano_status})")
        log.error("Please start plano using the 'planoai up' command.")
        sys.exit(1)

    # Determine plano_config.yaml path
    plano_config_file = find_config_file(path, file)
    if not os.path.exists(plano_config_file):
        log.error(f"Config file not found: {plano_config_file}")
        sys.exit(1)

    try:
        start_cli_agent(plano_config_file, settings)
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
