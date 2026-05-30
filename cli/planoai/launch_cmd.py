"""``planoai launch`` command group.

Launches CLI agents (Claude Code, Codex) or the Claude Desktop app against the
local Plano gateway. This replaces the old ``planoai cli-agent`` command.
"""

from __future__ import annotations

import json
import os
import sys
from typing import Optional

import rich_click as click
import yaml

from planoai import claude_desktop as _cd
from planoai.consts import NATIVE_PID_FILE, PLANO_DOCKER_NAME
from planoai.core import _resolve_cli_agent_endpoint, start_cli_agent
from planoai.docker_cli import docker_container_status
from planoai.defaults import DEFAULT_LLM_LISTENER_PORT
from planoai.utils import find_config_file, getLogger

log = getLogger(__name__)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


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


def _is_plano_running() -> bool:
    if _is_native_plano_running():
        return True
    return docker_container_status(PLANO_DOCKER_NAME) == "running"


def _require_plano_running(console) -> None:
    if _is_plano_running():
        return
    console.print("[red]✗[/red] Plano is not running.")
    console.print(
        "[dim]Start Plano first using 'planoai up <config.yaml>' "
        "(native or --docker mode).[/dim]"
    )
    sys.exit(1)


def _start_plano_with_config(config_path: str, console) -> None:
    """Invoke `planoai up` against the given config and wait for it to be healthy.

    Reuses the click ``up`` command's callback so we get the same validation,
    env loading, and native runner behavior as a top-level invocation. ``up``
    runs in detached/background mode by default and only returns once Plano is
    healthy, so we can safely continue with the Claude Desktop config flow
    after it returns.
    """
    # Lazy import: ``planoai.main`` pulls in heavy modules (rich, native runner,
    # etc.) and would create a circular import at module-load time.
    from planoai.main import up

    abs_path = os.path.abspath(config_path)
    if not os.path.exists(abs_path):
        console.print(f"[red]✗[/red] Config file not found: {abs_path}")
        sys.exit(1)

    console.print(
        f"[dim]Starting Plano with config " f"[cyan]{abs_path}[/cyan]...[/dim]"
    )
    up.callback(
        file=abs_path,
        path=".",
        foreground=False,
        with_tracing=False,
        tracing_port=4317,
        docker=False,
        verbose=False,
        listener_port=DEFAULT_LLM_LISTENER_PORT,
    )


def _base_url_from_config_file(config_path: str) -> Optional[str]:
    try:
        with open(config_path, "r") as f:
            cfg = yaml.safe_load(f) or {}
    except (OSError, yaml.YAMLError):
        return None
    _host, port = _resolve_cli_agent_endpoint(cfg)
    return f"http://localhost:{port}"


def _resolve_plano_config(file: Optional[str], path: str, console) -> str:
    plano_config_file = find_config_file(path, file)
    if not os.path.exists(plano_config_file):
        console.print(f"[red]✗[/red] Config file not found: {plano_config_file}")
        sys.exit(1)
    return plano_config_file


def _run_cli_agent(agent_type: str, file, path, settings) -> None:
    from rich.console import Console

    console = Console()
    _require_plano_running(console)
    plano_config_file = _resolve_plano_config(file, path, console)
    try:
        start_cli_agent(plano_config_file, agent_type, settings)
    except SystemExit:
        raise
    except Exception as e:
        click.echo(f"Error: {e}")
        sys.exit(1)


# ---------------------------------------------------------------------------
# Group + subcommands
# ---------------------------------------------------------------------------


@click.group()
def launch():
    """Launch a CLI agent or desktop app against the local Plano gateway."""


@launch.command("claude-cli")
@click.argument("file", required=False)
@click.option(
    "--path", default=".", help="Path to the directory containing plano_config.yaml"
)
@click.option(
    "--settings",
    default="{}",
    help="Additional settings as JSON string for the CLI agent.",
)
def claude_cli(file, path, settings):
    """Launch the Claude Code CLI connected to Plano."""
    _run_cli_agent("claude", file, path, settings)


@launch.command("codex")
@click.argument("file", required=False)
@click.option(
    "--path", default=".", help="Path to the directory containing plano_config.yaml"
)
@click.option(
    "--settings",
    default="{}",
    help="Additional settings as JSON string for the CLI agent.",
)
def codex(file, path, settings):
    """Launch the Codex CLI connected to Plano."""
    _run_cli_agent("codex", file, path, settings)


@launch.command("claude-desktop")
@click.option(
    "--config",
    "config_path",
    type=click.Path(dir_okay=False),
    default=None,
    help="Path to a Plano config; if Plano isn't already running, "
    "`planoai up <config>` is invoked first so the gateway is ready before "
    "Claude Desktop is configured.",
)
@click.option(
    "--no-launch",
    "no_launch",
    is_flag=True,
    default=False,
    help="Configure Claude Desktop but do not (re)open the app afterwards.",
)
@click.option(
    "--restore",
    "restore_flag",
    is_flag=True,
    default=False,
    help="Switch Claude Desktop back to its usual Anthropic Claude profile.",
)
@click.option(
    "--yes",
    "-y",
    "yes_flag",
    is_flag=True,
    default=False,
    help="Auto-approve restart prompts.",
)
@click.option(
    "--base-url",
    default=None,
    help="Plano LLM listener URL (default: derived from --config or running Plano, falling back to http://localhost:12000).",
)
def claude_desktop_cmd(config_path, no_launch, restore_flag, yes_flag, base_url):
    """Configure Claude Desktop to use the local Plano gateway.

    Mirrors `ollama launch claude-desktop`: rewrites Claude Desktop's profile
    JSONs (with `.bak` backups) to switch into third-party gateway mode pointed
    at Plano, then optionally restarts Claude Desktop so the change takes
    effect. When `--config <path>` is supplied and Plano is not already
    running, this command also starts Plano with that config first, so the
    end-to-end flow is a single command.
    """
    from rich.console import Console

    console = Console()

    err = _cd.supported()
    if err is not None:
        console.print(f"[red]✗[/red] {err}")
        sys.exit(1)

    if restore_flag:
        if config_path is not None:
            console.print(
                "[yellow]⚠[/yellow] --config is ignored when --restore is set."
            )
        try:
            _cd.restore()
        except Exception as e:
            console.print(f"[red]✗[/red] Failed to restore Claude Desktop: {e}")
            sys.exit(1)
        console.print(f"[green]✓[/green] {_cd.RESTORED_MESSAGE}")
        if no_launch:
            return
        try:
            _cd.launch_or_restart(
                "Restart Claude Desktop to use the usual Claude profile?",
                yes_flag,
            )
        except Exception as e:
            console.print(f"[yellow]⚠[/yellow] Could not restart Claude Desktop: {e}")
        return

    # Auto-start Plano if --config was provided and nothing is running yet.
    if config_path is not None:
        abs_config = os.path.abspath(config_path)
        if not os.path.exists(abs_config):
            console.print(f"[red]✗[/red] Config file not found: {abs_config}")
            sys.exit(1)
        if _is_plano_running():
            console.print(
                "[dim]Plano already running; skipping startup. Using listener "
                "from [cyan]"
                f"{abs_config}[/cyan] for the gateway URL.[/dim]"
            )
        else:
            _start_plano_with_config(abs_config, console)

    # Resolve base URL precedence: --base-url > --config file > running Plano > default.
    resolved_url = (
        base_url
        or (
            _base_url_from_config_file(os.path.abspath(config_path))
            if config_path is not None
            else None
        )
        or _resolve_base_url_from_running_plano()
        or _cd.DEFAULT_BASE_URL
    )

    if not _is_plano_running():
        console.print(
            "[yellow]⚠[/yellow] Plano does not appear to be running. "
            "Start it with [cyan]planoai up[/cyan] (or pass [cyan]--config "
            "<path>[/cyan]) before using Claude Desktop."
        )

    console.print(
        f"[dim]Configuring Claude Desktop to use Plano at "
        f"[cyan]{resolved_url}[/cyan][/dim]"
    )
    try:
        _cd.configure(resolved_url)
    except Exception as e:
        console.print(f"[red]✗[/red] Failed to configure Claude Desktop: {e}")
        sys.exit(1)

    console.print(f"[green]✓[/green] {_cd.SUCCESS_MESSAGE}")
    console.print(f"[dim]{_cd.RESTORE_HINT}[/dim]")

    if no_launch:
        return

    try:
        _cd.launch_or_restart("Restart Claude Desktop to use Plano?", yes_flag)
    except Exception as e:
        console.print(f"[yellow]⚠[/yellow] Could not restart Claude Desktop: {e}")


def _resolve_base_url_from_running_plano() -> Optional[str]:
    """Return ``http://localhost:<port>`` for the active Plano LLM listener.

    Best-effort: if no config can be located, return ``None`` so the caller
    falls back to ``DEFAULT_BASE_URL``.
    """
    try:
        plano_config_file = find_config_file(".", None)
    except Exception:
        return None
    if not plano_config_file or not os.path.exists(plano_config_file):
        return None
    try:
        with open(plano_config_file, "r") as f:
            cfg = yaml.safe_load(f) or {}
    except (OSError, yaml.YAMLError):
        return None
    _host, port = _resolve_cli_agent_endpoint(cfg)
    return f"http://localhost:{port}"
