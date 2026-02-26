import contextlib
import io
import json
import os
import signal
import subprocess
import sys
import time

from planoai.consts import (
    NATIVE_PID_FILE,
    PLANO_RUN_DIR,
)
from planoai.docker_cli import health_check_endpoint
from planoai.native_binaries import (
    ensure_envoy_binary,
    find_brightstaff_binary,
    find_wasm_plugins,
)
from planoai.utils import find_repo_root, getLogger

log = getLogger(__name__)


@contextlib.contextmanager
def _temporary_env(overrides):
    """Context manager that sets env vars from *overrides* and restores originals on exit."""
    saved = {}
    for key, value in overrides.items():
        saved[key] = os.environ.get(key)
        os.environ[key] = value
    try:
        yield
    finally:
        for key, original in saved.items():
            if original is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = original


def render_native_config(plano_config_file, env, with_tracing=False):
    """Render envoy and plano configs for native mode. Returns (envoy_config_path, plano_config_rendered_path)."""
    import yaml

    repo_root = find_repo_root()
    if not repo_root:
        print(
            "Error: Could not find repository root. "
            "Make sure you're inside the plano repository."
        )
        sys.exit(1)

    os.makedirs(PLANO_RUN_DIR, exist_ok=True)

    prompt_gw_path, llm_gw_path = find_wasm_plugins()

    # If --with-tracing, inject tracing config if not already present
    effective_config_file = os.path.abspath(plano_config_file)
    if with_tracing:
        with open(plano_config_file, "r") as f:
            config_data = yaml.safe_load(f)
        tracing = config_data.get("tracing", {})
        if not tracing.get("random_sampling"):
            tracing["random_sampling"] = 100
            config_data["tracing"] = tracing
            effective_config_file = os.path.join(
                PLANO_RUN_DIR, "config_with_tracing.yaml"
            )
            with open(effective_config_file, "w") as f:
                yaml.dump(config_data, f, default_flow_style=False)

    envoy_config_path = os.path.join(PLANO_RUN_DIR, "envoy.yaml")
    plano_config_rendered_path = os.path.join(
        PLANO_RUN_DIR, "plano_config_rendered.yaml"
    )

    # Set environment variables that config_generator.validate_and_render_schema() reads
    config_dir = os.path.join(repo_root, "config")
    overrides = {
        "PLANO_CONFIG_FILE": effective_config_file,
        "PLANO_CONFIG_SCHEMA_FILE": os.path.join(
            config_dir, "plano_config_schema.yaml"
        ),
        "TEMPLATE_ROOT": config_dir,
        "ENVOY_CONFIG_TEMPLATE_FILE": "envoy.template.yaml",
        "PLANO_CONFIG_FILE_RENDERED": plano_config_rendered_path,
        "ENVOY_CONFIG_FILE_RENDERED": envoy_config_path,
    }

    # Also propagate caller env vars (API keys, OTEL endpoint, etc.)
    for key, value in env.items():
        if key not in overrides:
            overrides[key] = value

    with _temporary_env(overrides):
        from planoai.config_generator import validate_and_render_schema

        # Suppress verbose print output from config_generator
        with contextlib.redirect_stdout(io.StringIO()):
            validate_and_render_schema()

    # Post-process envoy.yaml: replace Docker WASM plugin paths with local paths
    with open(envoy_config_path, "r") as f:
        envoy_content = f.read()

    envoy_content = envoy_content.replace(
        "/etc/envoy/proxy-wasm-plugins/prompt_gateway.wasm", prompt_gw_path
    )
    envoy_content = envoy_content.replace(
        "/etc/envoy/proxy-wasm-plugins/llm_gateway.wasm", llm_gw_path
    )

    # Replace /var/log/ paths with local log directory (non-root friendly)
    log_dir = os.path.join(PLANO_RUN_DIR, "logs")
    os.makedirs(log_dir, exist_ok=True)
    envoy_content = envoy_content.replace("/var/log/", log_dir + "/")

    with open(envoy_config_path, "w") as f:
        f.write(envoy_content)

    # Run envsubst-equivalent on both rendered files using the caller's env
    with _temporary_env(env):
        for filepath in [envoy_config_path, plano_config_rendered_path]:
            with open(filepath, "r") as f:
                content = f.read()
            content = os.path.expandvars(content)
            with open(filepath, "w") as f:
                f.write(content)

    return envoy_config_path, plano_config_rendered_path


def start_native(plano_config_file, env, foreground=False, with_tracing=False):
    """Start Envoy and brightstaff natively."""
    from planoai.core import _get_gateway_ports

    console = None
    try:
        from rich.console import Console

        console = Console()
    except ImportError:
        pass

    def status_print(msg):
        if console:
            console.print(msg)
        else:
            print(msg)

    envoy_path = ensure_envoy_binary()
    find_wasm_plugins()  # validate they exist
    brightstaff_path = find_brightstaff_binary()
    envoy_config_path, plano_config_rendered_path = render_native_config(
        plano_config_file, env, with_tracing=with_tracing
    )

    status_print(f"[green]✓[/green] Configuration rendered")

    log_dir = os.path.join(PLANO_RUN_DIR, "logs")
    os.makedirs(log_dir, exist_ok=True)

    log_level = env.get("LOG_LEVEL", "info")

    # Start brightstaff
    brightstaff_env = os.environ.copy()
    brightstaff_env["RUST_LOG"] = log_level
    brightstaff_env["PLANO_CONFIG_PATH_RENDERED"] = plano_config_rendered_path
    # Propagate API keys and other env vars
    for key, value in env.items():
        brightstaff_env[key] = value

    brightstaff_pid = _daemon_exec(
        [brightstaff_path],
        brightstaff_env,
        os.path.join(log_dir, "brightstaff.log"),
    )
    log.info(f"Started brightstaff (PID {brightstaff_pid})")

    # Start envoy
    envoy_pid = _daemon_exec(
        [
            envoy_path,
            "-c",
            envoy_config_path,
            "--component-log-level",
            f"wasm:{log_level}",
            "--log-format",
            "[%Y-%m-%d %T.%e][%l] %v",
        ],
        brightstaff_env,
        os.path.join(log_dir, "envoy.log"),
    )
    log.info(f"Started envoy (PID {envoy_pid})")

    # Save PIDs
    os.makedirs(PLANO_RUN_DIR, exist_ok=True)
    with open(NATIVE_PID_FILE, "w") as f:
        json.dump(
            {
                "envoy_pid": envoy_pid,
                "brightstaff_pid": brightstaff_pid,
            },
            f,
        )

    # Health check
    gateway_ports = _get_gateway_ports(plano_config_file)
    status_print(f"[dim]Waiting for listeners to become healthy...[/dim]")

    start_time = time.time()
    timeout = 60
    while True:
        all_healthy = True
        for port in gateway_ports:
            if not health_check_endpoint(f"http://localhost:{port}/healthz"):
                all_healthy = False

        if all_healthy:
            status_print(f"[green]✓[/green] Plano is running (native mode)")
            for port in gateway_ports:
                status_print(f"  [cyan]http://localhost:{port}[/cyan]")
            break

        # Check if processes are still alive
        if not _is_pid_alive(brightstaff_pid):
            status_print("[red]✗[/red] brightstaff exited unexpectedly")
            status_print(f"  Check logs: {os.path.join(log_dir, 'brightstaff.log')}")
            _kill_pid(envoy_pid)
            sys.exit(1)

        if not _is_pid_alive(envoy_pid):
            status_print("[red]✗[/red] envoy exited unexpectedly")
            status_print(f"  Check logs: {os.path.join(log_dir, 'envoy.log')}")
            _kill_pid(brightstaff_pid)
            sys.exit(1)

        if time.time() - start_time > timeout:
            status_print(f"[red]✗[/red] Health check timed out after {timeout}s")
            status_print(f"  Check logs in: {log_dir}")
            stop_native()
            sys.exit(1)

        time.sleep(1)

    if foreground:
        status_print(f"[dim]Running in foreground. Press Ctrl+C to stop.[/dim]")
        status_print(f"[dim]Logs: {log_dir}[/dim]")
        try:
            # Tail both log files
            tail_proc = subprocess.Popen(
                [
                    "tail",
                    "-f",
                    os.path.join(log_dir, "envoy.log"),
                    os.path.join(log_dir, "brightstaff.log"),
                ],
                stdout=sys.stdout,
                stderr=sys.stderr,
            )
            tail_proc.wait()
        except KeyboardInterrupt:
            status_print(f"\n[dim]Stopping Plano...[/dim]")
            if tail_proc.poll() is None:
                tail_proc.terminate()
            stop_native()
    else:
        status_print(f"[dim]Logs: {log_dir}[/dim]")
        status_print(f"[dim]Run 'planoai down --native' to stop.[/dim]")


def _daemon_exec(args, env, log_path):
    """Start a fully daemonized process via double-fork. Returns the child PID."""
    log_fd = os.open(log_path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644)

    pid = os.fork()
    if pid > 0:
        # Parent: close our copy of the log fd and wait for intermediate child
        os.close(log_fd)
        os.waitpid(pid, 0)
        # Read the grandchild PID from the pipe
        grandchild_pid_path = os.path.join(PLANO_RUN_DIR, f".daemon_pid_{pid}")
        deadline = time.time() + 5
        while time.time() < deadline:
            if os.path.exists(grandchild_pid_path):
                with open(grandchild_pid_path, "r") as f:
                    grandchild_pid = int(f.read().strip())
                os.unlink(grandchild_pid_path)
                return grandchild_pid
            time.sleep(0.05)
        raise RuntimeError(f"Timed out waiting for daemon PID from {args[0]}")

    # First child: create new session and fork again
    os.setsid()
    grandchild_pid = os.fork()
    if grandchild_pid > 0:
        # Intermediate child: write grandchild PID and exit
        pid_path = os.path.join(PLANO_RUN_DIR, f".daemon_pid_{os.getpid()}")
        with open(pid_path, "w") as f:
            f.write(str(grandchild_pid))
        os._exit(0)

    # Grandchild: this is the actual daemon
    os.dup2(log_fd, 1)  # stdout -> log
    os.dup2(log_fd, 2)  # stderr -> log
    os.close(log_fd)
    # Close stdin
    devnull = os.open(os.devnull, os.O_RDONLY)
    os.dup2(devnull, 0)
    os.close(devnull)

    os.execve(args[0], args, env)


def _is_pid_alive(pid):
    """Check if a process with the given PID is still running."""
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True  # Process exists but we can't signal it


def _kill_pid(pid):
    """Send SIGTERM to a PID, ignoring errors."""
    try:
        os.kill(pid, signal.SIGTERM)
    except (ProcessLookupError, PermissionError):
        pass


def stop_native():
    """Stop natively-running Envoy and brightstaff processes."""
    if not os.path.exists(NATIVE_PID_FILE):
        print("No native Plano instance found (PID file missing).")
        return

    with open(NATIVE_PID_FILE, "r") as f:
        pids = json.load(f)

    envoy_pid = pids.get("envoy_pid")
    brightstaff_pid = pids.get("brightstaff_pid")

    for name, pid in [("envoy", envoy_pid), ("brightstaff", brightstaff_pid)]:
        if pid is None:
            continue
        try:
            os.kill(pid, signal.SIGTERM)
            log.info(f"Sent SIGTERM to {name} (PID {pid})")
        except ProcessLookupError:
            log.info(f"{name} (PID {pid}) already stopped")
            continue
        except PermissionError:
            log.info(f"Permission denied stopping {name} (PID {pid})")
            continue

        # Wait for graceful shutdown
        deadline = time.time() + 10
        while time.time() < deadline:
            try:
                os.kill(pid, 0)  # Check if still alive
                time.sleep(0.5)
            except ProcessLookupError:
                break
        else:
            # Still alive after timeout, force kill
            try:
                os.kill(pid, signal.SIGKILL)
                log.info(f"Sent SIGKILL to {name} (PID {pid})")
            except ProcessLookupError:
                pass

    os.unlink(NATIVE_PID_FILE)
    print("Plano stopped (native mode).")


def native_validate_config(plano_config_file):
    """Validate config in-process without Docker."""
    repo_root = find_repo_root()
    if not repo_root:
        print(
            "Error: Could not find repository root. "
            "Make sure you're inside the plano repository."
        )
        sys.exit(1)

    config_dir = os.path.join(repo_root, "config")

    # Create temp dir for rendered output (we just want validation)
    os.makedirs(PLANO_RUN_DIR, exist_ok=True)

    overrides = {
        "PLANO_CONFIG_FILE": os.path.abspath(plano_config_file),
        "PLANO_CONFIG_SCHEMA_FILE": os.path.join(
            config_dir, "plano_config_schema.yaml"
        ),
        "TEMPLATE_ROOT": config_dir,
        "ENVOY_CONFIG_TEMPLATE_FILE": "envoy.template.yaml",
        "PLANO_CONFIG_FILE_RENDERED": os.path.join(
            PLANO_RUN_DIR, "plano_config_rendered.yaml"
        ),
        "ENVOY_CONFIG_FILE_RENDERED": os.path.join(PLANO_RUN_DIR, "envoy.yaml"),
    }

    with _temporary_env(overrides):
        from planoai.config_generator import validate_and_render_schema

        # Suppress verbose print output from config_generator
        with contextlib.redirect_stdout(io.StringIO()):
            validate_and_render_schema()
