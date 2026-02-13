import contextlib
import io
import json
import os
import platform
import signal
import subprocess
import sys
import tarfile
import time
import tempfile

from planoai.consts import (
    ENVOY_VERSION,
    NATIVE_PID_FILE,
    PLANO_BIN_DIR,
    PLANO_RUN_DIR,
)
from planoai.docker_cli import health_check_endpoint
from planoai.utils import find_repo_root, getLogger

log = getLogger(__name__)


def _get_platform_slug():
    """Return the platform slug for Envoy binary downloads."""
    system = platform.system().lower()
    machine = platform.machine().lower()

    mapping = {
        ("linux", "x86_64"): "linux-amd64",
        ("linux", "aarch64"): "linux-arm64",
        ("darwin", "arm64"): "darwin-arm64",
    }

    slug = mapping.get((system, machine))
    if slug is None:
        if system == "darwin" and machine == "x86_64":
            print(
                "Error: macOS x86_64 (Intel) is not supported. "
                "Pre-built Envoy binaries are only available for Apple Silicon (arm64)."
            )
            sys.exit(1)
        print(
            f"Error: Unsupported platform {system}/{machine}. "
            "Supported platforms: linux-amd64, linux-arm64, darwin-arm64"
        )
        sys.exit(1)

    return slug


def ensure_envoy_binary():
    """Download Envoy binary if not already present or version changed. Returns path to binary."""
    envoy_path = os.path.join(PLANO_BIN_DIR, "envoy")
    version_path = os.path.join(PLANO_BIN_DIR, "envoy.version")

    if os.path.exists(envoy_path) and os.access(envoy_path, os.X_OK):
        # Check if cached binary matches the pinned version
        if os.path.exists(version_path):
            with open(version_path, "r") as f:
                cached_version = f.read().strip()
            if cached_version == ENVOY_VERSION:
                log.info(f"Envoy {ENVOY_VERSION} found at {envoy_path}")
                return envoy_path
            print(
                f"Envoy version changed ({cached_version} → {ENVOY_VERSION}), re-downloading..."
            )
        else:
            log.info(
                f"Envoy binary found at {envoy_path} (unknown version, re-downloading...)"
            )

    slug = _get_platform_slug()
    url = (
        f"https://github.com/tetratelabs/archive-envoy/releases/download/"
        f"{ENVOY_VERSION}/envoy-{ENVOY_VERSION}-{slug}.tar.xz"
    )

    os.makedirs(PLANO_BIN_DIR, exist_ok=True)

    print(f"Downloading Envoy {ENVOY_VERSION} for {slug}...")
    print(f"  URL: {url}")

    with tempfile.NamedTemporaryFile(suffix=".tar.xz", delete=False) as tmp:
        tmp_path = tmp.name

    try:
        subprocess.run(
            ["curl", "-fSL", "-o", tmp_path, url],
            check=True,
        )

        print("Extracting Envoy binary...")
        with tarfile.open(tmp_path, "r:xz") as tar:
            # Find the envoy binary inside the archive
            envoy_member = None
            for member in tar.getmembers():
                if member.name.endswith("/bin/envoy") or member.name == "bin/envoy":
                    envoy_member = member
                    break

            if envoy_member is None:
                print("Error: Could not find envoy binary in the downloaded archive.")
                print("Archive contents:")
                for member in tar.getmembers():
                    print(f"  {member.name}")
                sys.exit(1)

            # Extract just the binary
            f = tar.extractfile(envoy_member)
            if f is None:
                print("Error: Could not extract envoy binary from archive.")
                sys.exit(1)

            with open(envoy_path, "wb") as out:
                out.write(f.read())

        os.chmod(envoy_path, 0o755)
        with open(version_path, "w") as f:
            f.write(ENVOY_VERSION)
        print(f"Envoy {ENVOY_VERSION} installed at {envoy_path}")
        return envoy_path

    except subprocess.CalledProcessError as e:
        print(f"Error downloading Envoy: {e}")
        print(f"URL: {url}")
        print("Please check your internet connection and try again.")
        sys.exit(1)
    finally:
        if os.path.exists(tmp_path):
            os.unlink(tmp_path)


def find_wasm_plugins():
    """Find WASM plugin files built from source. Returns (prompt_gateway_path, llm_gateway_path)."""
    repo_root = find_repo_root()
    if not repo_root:
        print(
            "Error: Could not find repository root. "
            "Make sure you're inside the plano repository."
        )
        sys.exit(1)

    wasm_dir = os.path.join(repo_root, "crates", "target", "wasm32-wasip1", "release")
    prompt_gw = os.path.join(wasm_dir, "prompt_gateway.wasm")
    llm_gw = os.path.join(wasm_dir, "llm_gateway.wasm")

    missing = []
    if not os.path.exists(prompt_gw):
        missing.append("prompt_gateway.wasm")
    if not os.path.exists(llm_gw):
        missing.append("llm_gateway.wasm")

    if missing:
        print(f"Error: WASM plugins not found: {', '.join(missing)}")
        print(f"  Expected at: {wasm_dir}/")
        print("  Run 'planoai build --native' first to build them.")
        sys.exit(1)

    return prompt_gw, llm_gw


def find_brightstaff_binary():
    """Find the brightstaff binary built from source. Returns path."""
    repo_root = find_repo_root()
    if not repo_root:
        print(
            "Error: Could not find repository root. "
            "Make sure you're inside the plano repository."
        )
        sys.exit(1)

    brightstaff_path = os.path.join(
        repo_root, "crates", "target", "release", "brightstaff"
    )
    if not os.path.exists(brightstaff_path):
        print(f"Error: brightstaff binary not found at {brightstaff_path}")
        print("  Run 'planoai build --native' first to build it.")
        sys.exit(1)

    return brightstaff_path


def render_native_config(arch_config_file, env, with_tracing=False):
    """Render envoy and arch configs for native mode. Returns (envoy_config_path, arch_config_rendered_path)."""
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
    effective_config_file = os.path.abspath(arch_config_file)
    if with_tracing:
        with open(arch_config_file, "r") as f:
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
    arch_config_rendered_path = os.path.join(PLANO_RUN_DIR, "arch_config_rendered.yaml")

    # Set environment variables that config_generator.validate_and_render_schema() reads
    config_dir = os.path.join(repo_root, "config")
    saved_env = {}
    overrides = {
        "ARCH_CONFIG_FILE": effective_config_file,
        "ARCH_CONFIG_SCHEMA_FILE": os.path.join(config_dir, "arch_config_schema.yaml"),
        "TEMPLATE_ROOT": config_dir,
        "ENVOY_CONFIG_TEMPLATE_FILE": "envoy.template.yaml",
        "ARCH_CONFIG_FILE_RENDERED": arch_config_rendered_path,
        "ENVOY_CONFIG_FILE_RENDERED": envoy_config_path,
    }

    # Also propagate caller env vars (API keys, OTEL endpoint, etc.)
    for key, value in env.items():
        if key not in overrides:
            overrides[key] = value

    for key, value in overrides.items():
        saved_env[key] = os.environ.get(key)
        os.environ[key] = value

    try:
        from planoai.config_generator import validate_and_render_schema

        # Suppress verbose print output from config_generator
        with contextlib.redirect_stdout(io.StringIO()):
            validate_and_render_schema()
    finally:
        # Restore original environment
        for key, original in saved_env.items():
            if original is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = original

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
    # (os.environ was already restored, so temporarily inject the env vars)
    env_to_restore = {}
    for key, value in env.items():
        env_to_restore[key] = os.environ.get(key)
        os.environ[key] = value
    try:
        for filepath in [envoy_config_path, arch_config_rendered_path]:
            with open(filepath, "r") as f:
                content = f.read()
            content = os.path.expandvars(content)
            with open(filepath, "w") as f:
                f.write(content)
    finally:
        for key, original in env_to_restore.items():
            if original is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = original

    return envoy_config_path, arch_config_rendered_path


def start_native(arch_config_file, env, foreground=False, with_tracing=False):
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
    envoy_config_path, arch_config_rendered_path = render_native_config(
        arch_config_file, env, with_tracing=with_tracing
    )

    status_print(f"[green]✓[/green] Configuration rendered")

    log_dir = os.path.join(PLANO_RUN_DIR, "logs")
    os.makedirs(log_dir, exist_ok=True)

    log_level = env.get("LOG_LEVEL", "info")

    # Start brightstaff
    brightstaff_env = os.environ.copy()
    brightstaff_env["RUST_LOG"] = log_level
    brightstaff_env["ARCH_CONFIG_PATH_RENDERED"] = arch_config_rendered_path
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
    gateway_ports = _get_gateway_ports(arch_config_file)
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


def native_validate_config(arch_config_file):
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

    saved_env = {}
    overrides = {
        "ARCH_CONFIG_FILE": os.path.abspath(arch_config_file),
        "ARCH_CONFIG_SCHEMA_FILE": os.path.join(config_dir, "arch_config_schema.yaml"),
        "TEMPLATE_ROOT": config_dir,
        "ENVOY_CONFIG_TEMPLATE_FILE": "envoy.template.yaml",
        "ARCH_CONFIG_FILE_RENDERED": os.path.join(
            PLANO_RUN_DIR, "arch_config_rendered.yaml"
        ),
        "ENVOY_CONFIG_FILE_RENDERED": os.path.join(PLANO_RUN_DIR, "envoy.yaml"),
    }

    for key, value in overrides.items():
        saved_env[key] = os.environ.get(key)
        os.environ[key] = value

    try:
        from planoai.config_generator import validate_and_render_schema

        # Suppress verbose print output from config_generator
        with contextlib.redirect_stdout(io.StringIO()):
            validate_and_render_schema()
    finally:
        for key, original in saved_env.items():
            if original is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = original
