"""Detect local-agent provider entries in a Plano config and warn the
operator that the host is about to spawn a local CLI binary with the same
filesystem, shell, and network capabilities as the user running planoai.

Local-agent providers (e.g. ``claude-cli``) are an entirely different
trust class from stateless network LLM providers (``openai``,
``anthropic``, ``gemini``, ...): the bridge runs inside brightstaff and
shells out to a local binary for every request, so a misconfigured
production deployment would expose the host to whatever the spawned
agent can do — which, for tools like Claude Code, is "anything the
operator can do at the shell".

This module is intentionally additive and side-effect free until the
caller invokes :func:`maybe_warn_local_agent_providers`. The set of
known local-agent provider interfaces lives in
:data:`LOCAL_AGENT_PROVIDER_INTERFACES`; adding a future entry (codex,
chatgpt-cli, opencode, hermes, ...) is a one-line change.
"""

from __future__ import annotations

import json
import os
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Iterable

from rich.console import Console
from rich.panel import Panel

from planoai.consts import PLANO_STATE_DIR

# Provider interfaces whose runtime spawns a local CLI subprocess with
# host filesystem / shell access. The string here is matched against the
# config's ``provider_interface`` field AND against the ``<prefix>/...``
# in ``model:`` and ``name:`` fields, so configs that rely on the
# Python-side autofill (``model: claude-cli/*`` only) are still detected
# before that autofill runs.
#
# Add new entries here as additional local-agent bridges are implemented
# (e.g. a future ``codex-cli`` or ``chatgpt-cli`` bridge that spawns the
# Codex CLI). This is the *only* line that needs to change to extend the
# warning's coverage.
LOCAL_AGENT_PROVIDER_INTERFACES: tuple[str, ...] = ("claude-cli",)

# Persistent ack lives next to the rest of the per-user planoai state
# (run/, bin/, plugins/, ...). Operators can ``rm`` this file to undo.
ACK_FILE_PATH = os.path.join(PLANO_STATE_DIR, "local_agent_ack.json")

# Env-var fallback for the ``--ack-local-agents`` CLI flag. Truthy values
# are 1/true/yes (case-insensitive); everything else is treated as unset.
ACK_ENV_VAR = "PLANO_ACK_LOCAL_AGENTS"

# Where the docs page lives. Printed verbatim in the warning panel — the
# relative path resolves cleanly when an operator opens it from the repo
# root, and the GitHub URL is a valid fallback for users running planoai
# outside a clone.
DOCS_RELATIVE_PATH = "docs/source/resources/local_agent_providers.rst"
DOCS_LEARN_MORE = (
    "https://github.com/katanemo/plano/blob/main/docs/source/resources/"
    "local_agent_providers.rst"
)


@dataclass(frozen=True)
class LocalAgentProvider:
    """A single ``model_providers`` entry that resolves to a local-agent
    bridge. ``name`` and ``model`` come straight from the config, while
    ``interface`` is the canonical key used for ack persistence."""

    interface: str
    name: str
    model: str


def _truthy_env(value: str | None) -> bool:
    if not value:
        return False
    return value.strip().lower() in {"1", "true", "yes", "on"}


def _interface_for_entry(entry: dict) -> str | None:
    """Return the canonical local-agent interface name for ``entry``, or
    ``None`` if the entry isn't a local-agent provider.

    Matching is intentionally permissive so that minimally-configured
    entries — i.e. just ``model: claude-cli/*`` before the Python
    autofill runs — are still detected. The first match wins and is
    returned; multiple matches against the same interface collapse.
    """

    if not isinstance(entry, dict):
        return None

    provider_interface = (entry.get("provider_interface") or "").strip()
    provider = (entry.get("provider") or "").strip()
    model = str(entry.get("model") or "").strip()
    name = str(entry.get("name") or "").strip()

    for interface in LOCAL_AGENT_PROVIDER_INTERFACES:
        if provider_interface == interface or provider == interface:
            return interface
        prefix = f"{interface}/"
        if model.startswith(prefix) or name.startswith(prefix):
            return interface

    return None


def detect_local_agent_providers(config: dict) -> list[LocalAgentProvider]:
    """Walk ``config`` and return every ``model_providers`` entry whose
    ``provider_interface`` falls in :data:`LOCAL_AGENT_PROVIDER_INTERFACES`.

    Order is preserved so the warning lists providers in declaration
    order. Both the new ``model_providers`` key and the legacy
    ``llm_providers`` key are consulted, mirroring the rest of the CLI.
    """

    if not isinstance(config, dict):
        return []

    providers = config.get("model_providers")
    if not isinstance(providers, list):
        providers = config.get("llm_providers") or []

    found: list[LocalAgentProvider] = []
    for entry in providers:
        interface = _interface_for_entry(entry)
        if interface is None:
            continue
        model = str(entry.get("model") or "").strip()
        name = str(entry.get("name") or "").strip() or model or interface
        found.append(LocalAgentProvider(interface=interface, name=name, model=model))
    return found


def _interfaces_in(providers: Iterable[LocalAgentProvider]) -> set[str]:
    return {p.interface for p in providers}


def load_acknowledged_interfaces(ack_path: str = ACK_FILE_PATH) -> set[str]:
    """Read the ack file and return the set of acknowledged provider
    interfaces. Missing or malformed files are treated as "no ack",
    never as a hard error, so a half-written ack file degrades to "warn
    again" instead of crashing ``planoai up``."""

    try:
        with open(ack_path, "r", encoding="utf-8") as f:
            data = json.load(f)
    except (OSError, json.JSONDecodeError):
        return set()

    if not isinstance(data, dict):
        return set()
    raw = data.get("acknowledged")
    if not isinstance(raw, list):
        return set()
    return {str(item) for item in raw if isinstance(item, str)}


def write_acknowledgement(
    interfaces: Iterable[str],
    ack_path: str = ACK_FILE_PATH,
) -> set[str]:
    """Persist ``interfaces`` (merged with anything already on disk) to
    the ack file. Returns the full acknowledged set after the write so
    callers can render an "acknowledged: X, Y" line."""

    merged = load_acknowledged_interfaces(ack_path) | set(interfaces)
    payload = {
        "acknowledged": sorted(merged),
        "ack_at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
    }
    os.makedirs(os.path.dirname(ack_path), exist_ok=True)
    with open(ack_path, "w", encoding="utf-8") as f:
        json.dump(payload, f, indent=2, sort_keys=True)
        f.write("\n")
    return merged


def _render_panel(
    console: Console,
    pending: list[LocalAgentProvider],
) -> None:
    """Render the single warning panel for ``pending``. Callers must
    ensure ``pending`` is non-empty; the caller decides whether to skip
    based on the ack set."""

    listed = "\n".join(
        f"  • [bold]{p.name}[/bold]"
        + (f" [dim]({p.model})[/dim]" if p.model and p.model != p.name else "")
        + f"  [dim]→ provider_interface=[/dim][cyan]{p.interface}[/cyan]"
        for p in pending
    )

    interfaces_csv = ", ".join(sorted({p.interface for p in pending}))
    body_lines = [
        "[bold yellow]This config wires up a local-agent provider.[/bold yellow]",
        "",
        listed,
        "",
        (
            "Unlike stateless network providers ([cyan]openai[/cyan], "
            "[cyan]anthropic[/cyan], [cyan]gemini[/cyan], ...), these entries "
            "spawn a local CLI binary as a subprocess of brightstaff. The "
            "subprocess inherits the operator's permissions and can:"
        ),
        "  • read and write any file the operator can touch",
        "  • execute arbitrary shell commands as the operator's user",
        "  • use the host's auth keychain / login session",
        "  • make outbound network calls from the host's IP",
        "",
        (
            "[bold]Intended for local development only — not production.[/bold] "
            "Treat this as the same trust class as OpenClaw / OpenCode / "
            "Hermes (agent integrations), not a stateless LLM provider."
        ),
        "",
        f"[dim]Learn more:[/dim] [bold]{DOCS_LEARN_MORE}[/bold]",
        f"[dim]Or in this repo:[/dim] [bold]{DOCS_RELATIVE_PATH}[/bold]",
        "",
        "[dim]Dismiss permanently:[/dim]",
        f"  [cyan]planoai up --ack-local-agents[/cyan]   [dim]# writes {ACK_FILE_PATH}[/dim]",
        f"  [dim]or:[/dim] [cyan]{ACK_ENV_VAR}=1 planoai up[/cyan]",
        f"[dim]Undo with:[/dim] [cyan]rm {ACK_FILE_PATH}[/cyan]",
    ]

    console.print(
        Panel(
            "\n".join(body_lines),
            title=f"⚠  Local-agent provider detected ({interfaces_csv})",
            title_align="left",
            border_style="yellow",
            padding=(1, 2),
        )
    )


def maybe_warn_local_agent_providers(
    config: dict,
    console: Console,
    *,
    ack_flag: bool = False,
    ack_path: str = ACK_FILE_PATH,
    env: dict | None = None,
) -> bool:
    """Show the local-agent warning panel if appropriate and return
    ``True`` iff the panel was rendered.

    Resolution order, top to bottom:

    1. No local-agent providers in config → no-op.
    2. ``ack_flag`` (the ``--ack-local-agents`` CLI flag) **or** the
       :data:`ACK_ENV_VAR` env var truthy → write/update the ack file
       so it covers every triggering interface, print one ✓ confirmation
       line, suppress the panel.
    3. Existing ack file already covers every triggering interface →
       print a single dim INFO line and suppress the panel.
    4. Otherwise → render the panel for the *un-acked* interfaces only
       (e.g. acknowledged ``claude-cli`` doesn't suppress a fresh
       warning when the operator later adds a hypothetical ``codex``).
    """

    env = env if env is not None else os.environ
    detected = detect_local_agent_providers(config)
    if not detected:
        return False

    ack_via_env = _truthy_env(env.get(ACK_ENV_VAR))
    if ack_flag or ack_via_env:
        new_set = _interfaces_in(detected)
        merged = write_acknowledgement(new_set, ack_path=ack_path)
        ack_csv = ", ".join(sorted(new_set))
        console.print(
            f"[green]✓[/green] Acknowledged local-agent provider(s): "
            f"[bold]{ack_csv}[/bold] [dim]→ {ack_path}[/dim]"
        )
        return False

    acknowledged = load_acknowledged_interfaces(ack_path)
    pending = [p for p in detected if p.interface not in acknowledged]
    if not pending:
        ack_csv = ", ".join(sorted(_interfaces_in(detected)))
        console.print(
            f"[dim]Local-agent providers acknowledged: {ack_csv}. "
            f"Remove {ack_path} to undo.[/dim]"
        )
        return False

    _render_panel(console, pending)
    return True


__all__ = [
    "ACK_ENV_VAR",
    "ACK_FILE_PATH",
    "DOCS_LEARN_MORE",
    "DOCS_RELATIVE_PATH",
    "LOCAL_AGENT_PROVIDER_INTERFACES",
    "LocalAgentProvider",
    "detect_local_agent_providers",
    "load_acknowledged_interfaces",
    "maybe_warn_local_agent_providers",
    "write_acknowledgement",
]
