"""Configure Claude Desktop to use the local Plano gateway.

Python port of Ollama's `cmd/launch/claude_desktop.go` tailored for Plano. The
flow is intentionally simpler than Ollama's:

1. Detect Claude Desktop on macOS / Windows.
2. Pick a string to put in Claude's ``inferenceGatewayApiKey`` slot (Claude
   Desktop requires the field; Plano's local gateway does not enforce bearer
   auth, so a placeholder is fine — see ``_resolve_api_key`` for precedence).
3. Rewrite Claude Desktop config JSON files with ``.bak`` backups to switch
   Claude into 3rd-party gateway mode pointed at Plano.
4. Optionally restart Claude Desktop so the changes take effect immediately.

Restoring flips ``deploymentMode`` back to ``1p`` and removes the Plano gateway
profile + meta entry.

The Claude Desktop ``deploymentMode = "3p"`` profile structure used here is
defined by Anthropic / observed via the Ollama integration; we do not control
it. We re-use the same JSON shape so Claude Desktop happily accepts the Plano
profile alongside any other third-party profile the user may have.
"""

from __future__ import annotations

import glob as _glob
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass, field
from typing import Callable, Optional

from planoai.utils import getLogger

log = getLogger(__name__)


# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

INTEGRATION_NAME = "claude-desktop"
PROFILE_NAME = "Plano"
# Deterministic UUID-v4 distinct from Ollama's `…0114`. The trailing bytes
# spell "PlanO" in ASCII to make it easy to identify the profile in
# `_meta.json`.
PROFILE_ID = "00000000-0000-4000-8000-0000506C616E"
DEFAULT_BASE_URL = "http://localhost:12000"
SUCCESS_MESSAGE = "Claude Desktop profile changed to Plano."
RESTORE_HINT = (
    "To restore the usual Claude profile, run: "
    "planoai launch claude-desktop --restore"
)
RESTORED_MESSAGE = "Claude Desktop restored to the usual Claude profile."

# Placeholder Claude Desktop writes into the gateway profile when the user
# hasn't overridden it. Plano's local gateway does not enforce a bearer
# token; this string only exists so Claude Desktop has a non-empty value to
# attach to outbound requests.
DEFAULT_API_KEY = "plano"

# How long we wait for Claude Desktop to fully exit on restart.
_QUIT_TIMEOUT_SECONDS = 30


# ---------------------------------------------------------------------------
# Test seams: replace these in tests instead of monkey-patching os/subprocess.
# ---------------------------------------------------------------------------


# Platform identifier. ``"darwin"``, ``"windows"``, or anything else (which
# is treated as unsupported). Module-level so tests can override it.
def _detect_goos() -> str:
    if os.name == "nt":
        return "windows"
    if sys.platform == "darwin":
        return "darwin"
    return sys.platform


_GOOS: str = _detect_goos()

_user_home: Callable[[], str] = os.path.expanduser  # called as _user_home("~")


def _is_running() -> bool:
    """Return True if Claude Desktop is currently running."""
    if _GOOS == "darwin":
        try:
            out = subprocess.run(
                ["pgrep", "-f", "Claude.app/Contents/MacOS/Claude"],
                capture_output=True,
                text=True,
                check=False,
            )
            return out.returncode == 0 and out.stdout.strip() != ""
        except FileNotFoundError:
            return False
    if _GOOS == "windows":
        script = (
            "(Get-Process claude -ErrorAction SilentlyContinue "
            "| Where-Object { $_.MainWindowHandle -ne 0 } "
            "| Select-Object -First 1).Id"
        )
        try:
            out = subprocess.run(
                ["powershell.exe", "-NoProfile", "-Command", script],
                capture_output=True,
                text=True,
                check=False,
            )
            return out.returncode == 0 and out.stdout.strip() != ""
        except FileNotFoundError:
            return False
    return False


def _quit() -> None:
    """Ask Claude Desktop to quit gracefully."""
    if _GOOS == "darwin":
        subprocess.run(
            ["osascript", "-e", 'tell application "Claude" to quit'],
            check=False,
        )
        return
    if _GOOS == "windows":
        script = (
            "Get-Process claude -ErrorAction SilentlyContinue "
            "| Where-Object { $_.MainWindowHandle -ne 0 } "
            "| ForEach-Object { [void]$_.CloseMainWindow() }"
        )
        subprocess.run(
            ["powershell.exe", "-NoProfile", "-Command", script],
            check=False,
        )


def _open() -> None:
    """Launch Claude Desktop."""
    if _GOOS == "darwin":
        subprocess.run(["open", "-a", "Claude"], check=False)
        return
    if _GOOS == "windows":
        path = _claude_app_path()
        if not path:
            raise RuntimeError(
                "Claude Desktop executable was not found; open Claude Desktop "
                "manually once and re-run 'planoai launch claude-desktop'"
            )
        ps_path = "'" + path.replace("'", "''") + "'"
        subprocess.run(
            [
                "powershell.exe",
                "-NoProfile",
                "-Command",
                f"Start-Process -FilePath {ps_path}",
            ],
            check=False,
        )


def _sleep(seconds: float) -> None:
    time.sleep(seconds)


# ---------------------------------------------------------------------------
# Path discovery
# ---------------------------------------------------------------------------


@dataclass
class _ThirdPartyPaths:
    desktop_config: str
    meta: str
    profile: str


@dataclass
class _Targets:
    normal_configs: list[str] = field(default_factory=list)
    third_party_profiles: list[_ThirdPartyPaths] = field(default_factory=list)


def supported() -> Optional[str]:
    """Return ``None`` if the platform is supported, else an error message."""
    if _GOOS in ("darwin", "windows"):
        return None
    return "Claude Desktop launch is only supported on macOS and Windows"


def _home() -> str:
    home = _user_home("~")
    if home == "~" or not home:
        raise RuntimeError("could not resolve user home directory")
    return home


def _local_app_data() -> str:
    val = (os.environ.get("LOCALAPPDATA") or "").strip()
    if val:
        return val
    user = (os.environ.get("USERPROFILE") or "").strip()
    if user:
        return os.path.join(user, "AppData", "Local")
    return os.path.join(_home(), "AppData", "Local")


def _darwin_profile_roots() -> tuple[list[str], list[str]]:
    base = os.path.join(_home(), "Library", "Application Support")
    return ([os.path.join(base, "Claude")], [os.path.join(base, "Claude-3p")])


def _windows_profile_roots() -> tuple[list[str], list[str]]:
    local = _local_app_data()
    normal = [
        os.path.join(local, "Claude"),
        os.path.join(local, "Claude Nest"),
    ]
    third_party = [
        os.path.join(local, "Claude-3p"),
        os.path.join(local, "Claude Nest-3p"),
    ]
    return normal, third_party


def _dedupe_paths(paths: list[str]) -> list[str]:
    out: list[str] = []
    seen: set[str] = set()
    for path in paths:
        if not path or not path.strip():
            continue
        key = path.lower()
        if key in seen:
            continue
        seen.add(key)
        out.append(path)
    return out


def _target_paths() -> _Targets:
    err = supported()
    if err is not None:
        raise RuntimeError(err)

    if _GOOS == "darwin":
        normal, third = _darwin_profile_roots()
    else:
        normal, third = _windows_profile_roots()

    targets = _Targets()
    for root in _dedupe_paths(normal):
        targets.normal_configs.append(os.path.join(root, "claude_desktop_config.json"))
    for root in _dedupe_paths(third):
        targets.third_party_profiles.append(
            _ThirdPartyPaths(
                desktop_config=os.path.join(root, "claude_desktop_config.json"),
                meta=os.path.join(root, "configLibrary", "_meta.json"),
                profile=os.path.join(root, "configLibrary", f"{PROFILE_ID}.json"),
            )
        )
    return targets


def _claude_app_path() -> str:
    """Return path to the Claude Desktop executable, or ``""`` if unknown."""
    if _GOOS == "darwin":
        candidates = ["/Applications/Claude.app"]
        candidates.append(os.path.join(_home(), "Applications", "Claude.app"))
        for path in candidates:
            if os.path.exists(path):
                return path
        return ""
    if _GOOS == "windows":
        local = _local_app_data()
        candidates = [
            os.path.join(local, "Programs", "Claude", "Claude.exe"),
            os.path.join(local, "Programs", "Claude Desktop", "Claude.exe"),
            os.path.join(local, "Claude", "Claude.exe"),
            os.path.join(local, "Claude Nest", "Claude.exe"),
            os.path.join(local, "Claude Desktop", "Claude.exe"),
            os.path.join(local, "AnthropicClaude", "Claude.exe"),
        ]
        for pattern in (
            os.path.join(local, "AnthropicClaude", "app-*", "Claude.exe"),
            os.path.join(local, "Programs", "Claude", "app-*", "Claude.exe"),
            os.path.join(local, "Programs", "Claude Desktop", "app-*", "Claude.exe"),
        ):
            candidates.extend(_glob.glob(pattern))
        for path in _dedupe_paths(candidates):
            if os.path.exists(path):
                return path
        return ""
    return ""


def is_installed() -> bool:
    """Best-effort check: app binary or any profile dir is present."""
    if _claude_app_path():
        return True
    if _GOOS == "windows" and _is_running():
        return True
    if _GOOS == "darwin":
        normal, third = _darwin_profile_roots()
    elif _GOOS == "windows":
        normal, third = _windows_profile_roots()
    else:
        return False
    for path in normal + third:
        if os.path.isdir(path):
            return True
    return False


# ---------------------------------------------------------------------------
# JSON IO with atomic write + .bak backup
# ---------------------------------------------------------------------------


def _read_json(path: str) -> dict:
    with open(path, "r", encoding="utf-8") as f:
        data = f.read()
    if not data.strip():
        return {}
    parsed = json.loads(data)
    return parsed if isinstance(parsed, dict) else {}


def _read_json_allow_missing(path: str) -> dict:
    try:
        return _read_json(path)
    except FileNotFoundError:
        return {}


def _atomic_write_with_backup(path: str, payload: bytes) -> None:
    """Write ``payload`` to ``path`` atomically, keeping a ``.bak`` copy."""
    parent = os.path.dirname(path)
    if parent:
        os.makedirs(parent, exist_ok=True)
    if os.path.exists(path):
        try:
            shutil.copy2(path, path + ".bak")
        except OSError as e:
            log.debug("could not write backup for %s: %s", path, e)

    fd, tmp_path = tempfile.mkstemp(prefix=".plano_", suffix=".tmp", dir=parent or None)
    try:
        with os.fdopen(fd, "wb") as f:
            f.write(payload)
        os.replace(tmp_path, path)
    except Exception:
        try:
            os.unlink(tmp_path)
        except OSError:
            pass
        raise


def _write_json(path: str, value: dict) -> None:
    payload = (json.dumps(value, indent=2) + "\n").encode("utf-8")
    _atomic_write_with_backup(path, payload)


# ---------------------------------------------------------------------------
# JSON shape mutators (1:1 with Ollama)
# ---------------------------------------------------------------------------


def _write_deployment_mode(path: str, mode: str) -> None:
    cfg = _read_json_allow_missing(path)
    cfg["deploymentMode"] = mode
    _write_json(path, cfg)


def _write_meta(path: str, profile_id: str, name: str) -> None:
    meta = _read_json_allow_missing(path)
    meta["appliedId"] = profile_id

    raw_entries = meta.get("entries")
    entries: list = []
    if isinstance(raw_entries, list):
        for entry in raw_entries:
            if isinstance(entry, dict) and entry.get("id") == profile_id:
                continue
            entries.append(entry)
    entries.append({"id": profile_id, "name": name})
    meta["entries"] = entries
    _write_json(path, meta)


def _write_gateway_profile(
    path: str, api_key: str, base_url: str, force_chooser: bool
) -> None:
    cfg = _read_json_allow_missing(path)
    cfg["inferenceProvider"] = "gateway"
    cfg["inferenceGatewayBaseUrl"] = base_url
    cfg["inferenceGatewayApiKey"] = api_key
    cfg["inferenceGatewayAuthScheme"] = "bearer"
    cfg.pop("inferenceModels", None)
    cfg["disableDeploymentModeChooser"] = force_chooser
    _write_json(path, cfg)


def _restore_meta(path: str) -> None:
    meta = _read_json_allow_missing(path)
    if not meta:
        return
    changed = False
    if meta.get("appliedId") == PROFILE_ID:
        meta.pop("appliedId", None)
        changed = True

    raw_entries = meta.get("entries")
    if isinstance(raw_entries, list):
        filtered: list = []
        for entry in raw_entries:
            if isinstance(entry, dict) and entry.get("id") == PROFILE_ID:
                changed = True
                continue
            filtered.append(entry)
        meta["entries"] = filtered

    if changed:
        _write_json(path, meta)


def _restore_profile(path: str) -> None:
    cfg = _read_json_allow_missing(path)
    if not cfg:
        return
    cfg["disableDeploymentModeChooser"] = False
    for key in (
        "inferenceProvider",
        "inferenceGatewayBaseUrl",
        "inferenceGatewayAuthScheme",
        "inferenceModels",
    ):
        cfg.pop(key, None)
    _write_json(path, cfg)


def _read_applied_id(path: str) -> str:
    try:
        meta = _read_json(path)
    except (FileNotFoundError, json.JSONDecodeError):
        return ""
    val = meta.get("appliedId")
    return val if isinstance(val, str) else ""


def _read_deployment_mode(path: str) -> str:
    try:
        cfg = _read_json(path)
    except (FileNotFoundError, json.JSONDecodeError):
        return ""
    val = cfg.get("deploymentMode")
    return val if isinstance(val, str) else ""


def _read_gateway_api_key(path: str) -> str:
    try:
        cfg = _read_json(path)
    except (FileNotFoundError, json.JSONDecodeError):
        return ""
    val = cfg.get("inferenceGatewayApiKey")
    return val.strip() if isinstance(val, str) else ""


def _third_party_profile_ok(t: _ThirdPartyPaths) -> bool:
    if _read_applied_id(t.meta) != PROFILE_ID:
        return False
    try:
        cfg = _read_json(t.profile)
    except (FileNotFoundError, json.JSONDecodeError):
        return False
    if cfg.get("inferenceProvider") != "gateway":
        return False
    base_url = cfg.get("inferenceGatewayBaseUrl")
    if not isinstance(base_url, str) or not base_url.strip():
        return False
    api_key = cfg.get("inferenceGatewayApiKey")
    if not isinstance(api_key, str) or not api_key.strip():
        return False
    return True


def is_configured() -> bool:
    try:
        targets = _target_paths()
    except RuntimeError:
        return False
    if not targets.normal_configs or not targets.third_party_profiles:
        return False
    for path in targets.normal_configs:
        if _read_deployment_mode(path) != "3p":
            return False
    for t in targets.third_party_profiles:
        if _read_deployment_mode(t.desktop_config) != "3p":
            return False
        if not _third_party_profile_ok(t):
            return False
    return True


# ---------------------------------------------------------------------------
# API key resolution
# ---------------------------------------------------------------------------
#
# Plano's local gateway does not enforce bearer auth — there's no such thing
# as a "Plano API key". Claude Desktop's third-party profile schema, however,
# requires ``inferenceGatewayApiKey`` to be a non-empty string before it will
# treat the profile as configured. We therefore pick *some* string to write
# into that slot, with the following precedence so users running Plano behind
# their own auth proxy can opt-in:
#
#   1. ``$PLANO_API_KEY`` — explicit override (e.g. an internal auth token).
#   2. The existing ``inferenceGatewayApiKey`` already in Claude's 3p profile,
#      so re-running ``planoai launch claude-desktop`` does not clobber a
#      value the user manually set.
#   3. The fixed placeholder ``DEFAULT_API_KEY`` ("plano").
#
# We do not validate this string against the gateway. The gateway's
# reachability is already surfaced by ``launch_cmd._is_plano_running()``
# before this module is invoked.


def _resolve_api_key(profile_paths: list[str]) -> str:
    env_key = (os.environ.get("PLANO_API_KEY") or "").strip()
    if env_key:
        return env_key

    for path in profile_paths:
        existing = _read_gateway_api_key(path)
        if existing:
            return existing

    return DEFAULT_API_KEY


# ---------------------------------------------------------------------------
# Public configure / restore / launch
# ---------------------------------------------------------------------------


def configure(base_url: str = DEFAULT_BASE_URL, *, force_chooser: bool = True) -> None:
    """Switch Claude Desktop into 3p mode pointed at the local Plano gateway."""
    err = supported()
    if err is not None:
        raise RuntimeError(err)

    targets = _target_paths()
    profile_paths = [t.profile for t in targets.third_party_profiles]
    api_key = _resolve_api_key(profile_paths)

    for path in targets.normal_configs:
        _write_deployment_mode(path, "3p")
    for t in targets.third_party_profiles:
        _write_deployment_mode(t.desktop_config, "3p")
        _write_meta(t.meta, PROFILE_ID, PROFILE_NAME)
        _write_gateway_profile(t.profile, api_key, base_url, force_chooser)


def restore() -> None:
    """Flip Claude Desktop back to the default Anthropic profile."""
    err = supported()
    if err is not None:
        raise RuntimeError(err)

    targets = _target_paths()
    for path in targets.normal_configs:
        _write_deployment_mode(path, "1p")
    for t in targets.third_party_profiles:
        _write_deployment_mode(t.desktop_config, "1p")
        _restore_meta(t.meta)
        _restore_profile(t.profile)


def _can_prompt() -> bool:
    return sys.stdin.isatty() and sys.stderr.isatty()


def _confirm(prompt: str, yes: bool) -> bool:
    if yes:
        return True
    if not _can_prompt():
        return False
    try:
        answer = input(f"{prompt} [Y/n] ").strip().lower()
    except (EOFError, KeyboardInterrupt):
        sys.stderr.write("\n")
        return False
    return answer in ("", "y", "yes")


def launch_or_restart(prompt: str, yes: bool) -> None:
    """Open Claude Desktop, restarting it first if it is already running."""
    err = supported()
    if err is not None:
        raise RuntimeError(err)

    if not _is_running():
        _open()
        return

    if not _confirm(prompt, yes):
        sys.stderr.write(
            "Quit and reopen Claude Desktop when you're ready for the "
            "profile change to take effect.\n"
        )
        return

    _quit()
    deadline = time.time() + _QUIT_TIMEOUT_SECONDS
    while time.time() < deadline:
        if not _is_running():
            break
        _sleep(0.2)
    else:
        raise RuntimeError(
            "Claude Desktop did not quit; quit it manually and re-run " "the command"
        )
    _open()
