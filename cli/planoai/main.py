import json
import os
import multiprocessing
import subprocess
import sys
import contextlib
import logging
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
from planoai.chatgpt_cmd import chatgpt as chatgpt_cmd
from planoai.consts import (
    DEFAULT_OTEL_TRACING_GRPC_ENDPOINT,
    DEFAULT_NATIVE_OTEL_TRACING_GRPC_ENDPOINT,
    NATIVE_PID_FILE,
    PLANO_RUN_DIR,
    PLANO_DOCKER_IMAGE,
    PLANO_DOCKER_NAME,
)
from planoai.rich_click_config import configure_rich_click
from planoai.versioning import check_version_status, get_latest_version, get_version

log = getLogger(__name__)


def _is_native_plano_running() -> bool:
    if not os.path.exists(NATIVE_PID_FILE):
        return False
    try:
        with open(NATIVE_PID_FILE, "r") as f:
            pids = json.load(f)
    except (OSError, json.JSONDecodeError):
        return False

    envoy_pid = pids.get("envoy_pid")
    brightstaff_pid = pids.get("brightstaff_pid")
    if not isinstance(envoy_pid, int) or not isinstance(brightstaff_pid, int):
        return False

    for pid in (envoy_pid, brightstaff_pid):
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return False
        except PermissionError:
            continue
    return True


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


@contextlib.contextmanager
def _temporary_cli_log_level(level: str | None):
    if level is None:
        yield
        return

    current_level = logging.getLevelName(logging.getLogger().level).lower()
    set_log_level(level)
    try:
        yield
    finally:
        set_log_level(current_level)


def _inject_chatgpt_tokens_if_needed(config, env, console):
    """If config uses chatgpt providers, resolve tokens from ~/.plano/chatgpt/auth.json."""
    providers = config.get("model_providers") or config.get("llm_providers") or []
    has_chatgpt = any(str(p.get("model", "")).startswith("chatgpt/") for p in providers)
    if not has_chatgpt:
        return

    try:
        from planoai.chatgpt_auth import get_access_token

        access_token, account_id = get_access_token()
        env["CHATGPT_ACCESS_TOKEN"] = access_token
        if account_id:
            env["CHATGPT_ACCOUNT_ID"] = account_id
    except Exception as e:
        console.print(
            f"\n[red]ChatGPT auth error:[/red] {e}\n"
            f"[dim]Run 'planoai chatgpt login' to authenticate.[/dim]\n"
        )
        sys.exit(1)


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
@click.option(
    "--docker",
    default=False,
    help="Build the Docker image instead of native binaries.",
    is_flag=True,
)
def build(docker):
    """Build Plano from source. Works from any directory within the repo."""

    # Find the repo root
    repo_root = find_repo_root()
    if not repo_root:
        click.echo(
            "Error: Could not find repository root. Make sure you're inside the plano repository."
        )
        sys.exit(1)

    if not docker:
        import shutil

        crates_dir = os.path.join(repo_root, "crates")
        console = _console()
        _print_cli_header(console)

        if not shutil.which("cargo"):
            console.print(
                "[red]✗[/red] [bold]cargo[/bold] not found. "
                "Install Rust: [cyan]https://rustup.rs[/cyan]"
            )
            sys.exit(1)

        console.print("[dim]Building WASM plugins (wasm32-wasip1)...[/dim]")
        try:
            subprocess.run(
                [
                    "cargo",
                    "build",
                    "--release",
                    "--target",
                    "wasm32-wasip1",
                    "-p",
                    "llm_gateway",
                    "-p",
                    "prompt_gateway",
                ],
                cwd=crates_dir,
                check=True,
            )
            log.info("WASM plugins built")
        except subprocess.CalledProcessError as e:
            console.print(f"[red]✗[/red] WASM build failed: {e}")
            sys.exit(1)

        console.print("[dim]Building brightstaff (native)...[/dim]")
        try:
            subprocess.run(
                [
                    "cargo",
                    "build",
                    "--release",
                    "-p",
                    "brightstaff",
                ],
                cwd=crates_dir,
                check=True,
            )
            log.info("brightstaff built")
        except subprocess.CalledProcessError as e:
            console.print(f"[red]✗[/red] brightstaff build failed: {e}")
            sys.exit(1)

        wasm_dir = os.path.join(crates_dir, "target", "wasm32-wasip1", "release")
        native_dir = os.path.join(crates_dir, "target", "release")
        console.print(f"\n[bold]Build artifacts:[/bold]")
        console.print(f"  {os.path.join(wasm_dir, 'prompt_gateway.wasm')}")
        console.print(f"  {os.path.join(wasm_dir, 'llm_gateway.wasm')}")
        console.print(f"  {os.path.join(native_dir, 'brightstaff')}")
        return

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
@click.option(
    "--docker",
    default=False,
    help="Run Plano inside Docker instead of natively.",
    is_flag=True,
)
@click.option(
    "--verbose",
    "-v",
    default=False,
    help="Show detailed startup logs with timestamps.",
    is_flag=True,
)
def up(file, path, foreground, with_tracing, tracing_port, docker, verbose):
    """Starts Plano."""
    from rich.status import Status

    console = _console()
    _print_cli_header(console)

    with _temporary_cli_log_level("warning" if not verbose else None):
        # Use the utility function to find config file
        plano_config_file = find_config_file(path, file)

        # Check if the file exists
        if not os.path.exists(plano_config_file):
            console.print(
                f"[red]✗[/red] Config file not found: [dim]{plano_config_file}[/dim]"
            )
            sys.exit(1)

        if not docker:
            from planoai.native_runner import native_validate_config

            with Status(
                "[dim]Validating configuration[/dim]",
                spinner="dots",
                spinner_style="dim",
            ):
                try:
                    native_validate_config(plano_config_file)
                except SystemExit:
                    console.print(f"[red]✗[/red] Validation failed")
                    sys.exit(1)
                except Exception as e:
                    console.print(f"[red]✗[/red] Validation failed")
                    console.print(f"  [dim]{str(e).strip()}[/dim]")
                    sys.exit(1)
        else:
            with Status(
                "[dim]Validating configuration (Docker)[/dim]",
                spinner="dots",
                spinner_style="dim",
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

        log.info("Configuration valid")

        # Set up environment
        default_otel = (
            DEFAULT_OTEL_TRACING_GRPC_ENDPOINT
            if docker
            else DEFAULT_NATIVE_OTEL_TRACING_GRPC_ENDPOINT
        )
        env_stage = {
            "OTEL_TRACING_GRPC_ENDPOINT": default_otel,
        }
        env = os.environ.copy()
        env.pop("PATH", None)

        import yaml

        with open(plano_config_file, "r") as f:
            plano_config = yaml.safe_load(f)

        # Inject ChatGPT tokens from ~/.plano/chatgpt/auth.json if any provider needs them
        _inject_chatgpt_tokens_if_needed(plano_config, env, console)

        # Check access keys
        access_keys = get_llm_provider_access_keys(plano_config_file=plano_config_file)
        access_keys = set(access_keys)
        access_keys = [
            item[1:] if item.startswith("$") else item for item in access_keys
        ]

        missing_keys = []
        if access_keys:
            if file:
                app_env_file = os.path.join(
                    os.path.dirname(os.path.abspath(file)), ".env"
                )
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
                    trace_server = start_trace_listener_background(
                        grpc_port=tracing_port
                    )
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
            tracing_host = "host.docker.internal" if docker else "localhost"
            otel_endpoint = f"http://{tracing_host}:{tracing_port}"
            env_stage["OTEL_TRACING_GRPC_ENDPOINT"] = otel_endpoint

        env.update(env_stage)
        try:
            if not docker:
                from planoai.native_runner import start_native

                if not verbose:
                    with Status(
                        f"[{PLANO_COLOR}]Starting Plano...[/{PLANO_COLOR}]",
                        spinner="dots",
                    ) as status:

                        def _update_progress(message: str) -> None:
                            console.print(f"[dim]{message}[/dim]")

                        start_native(
                            plano_config_file,
                            env,
                            foreground=foreground,
                            with_tracing=with_tracing,
                            progress_callback=_update_progress,
                        )
                    console.print("")
                    console.print(
                        "[green]✓[/green] Plano is running! [dim](native mode)[/dim]"
                    )
                    log_dir = os.path.join(PLANO_RUN_DIR, "logs")
                    console.print(f"Logs: {log_dir}")
                    console.print("Run 'planoai down' to stop.")
                else:
                    start_native(
                        plano_config_file,
                        env,
                        foreground=foreground,
                        with_tracing=with_tracing,
                    )
            else:
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
@click.option(
    "--docker",
    default=False,
    help="Stop a Docker-based Plano instance.",
    is_flag=True,
)
@click.option(
    "--verbose",
    "-v",
    default=False,
    help="Show detailed shutdown logs with timestamps.",
    is_flag=True,
)
def down(docker, verbose):
    """Stops Plano."""
    console = _console()
    _print_cli_header(console)

    with _temporary_cli_log_level("warning" if not verbose else None):
        if not docker:
            from planoai.native_runner import stop_native

            with console.status(
                f"[{PLANO_COLOR}]Shutting down Plano...[/{PLANO_COLOR}]",
                spinner="dots",
            ):
                stopped = stop_native()
            if not verbose:
                if stopped:
                    console.print(
                        "[green]✓[/green] Plano stopped! [dim](native mode)[/dim]"
                    )
                else:
                    console.print(
                        "[dim]No Plano instance was running (native mode)[/dim]"
                    )
        else:
            with console.status(
                f"[{PLANO_COLOR}]Shutting down Plano (Docker)...[/{PLANO_COLOR}]",
                spinner="dots",
            ):
                stop_docker_container()
            if not verbose:
                console.print(
                    "[green]✓[/green] Plano stopped! [dim](docker mode)[/dim]"
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
@click.option(
    "--docker",
    default=False,
    help="Stream logs from a Docker-based Plano instance.",
    is_flag=True,
)
def logs(debug, follow, docker):
    """Stream logs from access logs services."""

    if not docker:
        from planoai.native_runner import native_logs

        native_logs(debug=debug, follow=follow)
        return

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
@click.argument("type", type=click.Choice(["claude", "codex"]), required=True)
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

    CLI_AGENT: The type of CLI agent to start ('claude' or 'codex')
    """

    native_running = _is_native_plano_running()
    docker_running = False
    if not native_running:
        docker_running = docker_container_status(PLANO_DOCKER_NAME) == "running"

    if not (native_running or docker_running):
        log.error("Plano is not running.")
        log.error(
            "Start Plano first using 'planoai up <config.yaml>' (native or --docker mode)."
        )
        sys.exit(1)

    # Determine plano_config.yaml path
    plano_config_file = find_config_file(path, file)
    if not os.path.exists(plano_config_file):
        log.error(f"Config file not found: {plano_config_file}")
        sys.exit(1)

    try:
        start_cli_agent(plano_config_file, type, settings)
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
main.add_command(chatgpt_cmd, name="chatgpt")

if __name__ == "__main__":
    main()
