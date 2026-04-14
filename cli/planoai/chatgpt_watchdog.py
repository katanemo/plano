"""
Background daemon that monitors ChatGPT OAuth token expiry and restarts
Plano processes with a fresh token before the old one expires.

The watchdog is spawned by start_native() when ChatGPT providers are present.
It runs as a fully daemonized process (double-fork) and exits after triggering
a restart (a fresh watchdog is spawned by the new start_native() call).
"""

import json
import os
import time
from typing import Optional

from planoai.consts import (
    NATIVE_PID_FILE,
    PLANO_RUN_DIR,
    PLANO_WATCHDOG_LOG_FILE,
    PLANO_WATCHDOG_STATE_FILE,
)

# Wake up this many seconds before token expiry to refresh
WATCHDOG_REFRESH_LEAD_SECONDS = 5 * 60  # 5 minutes

# How often the watchdog polls for expiry (in seconds)
WATCHDOG_POLL_INTERVAL_SECONDS = 30

# Env var sentinel: if set, spawn_watchdog() is a no-op (prevents recursive spawning)
_NO_WATCHDOG_ENV_VAR = "PLANO_NO_WATCHDOG"


def _log(msg: str) -> None:
    print(f"[{time.strftime('%Y-%m-%d %H:%M:%S')}] watchdog: {msg}", flush=True)


def _seconds_until_expiry() -> Optional[float]:
    """Return seconds until the ChatGPT access token expires, or None if unknown."""
    try:
        from planoai.chatgpt_auth import load_auth

        auth = load_auth()
        if not auth:
            return None
        expires_at = auth.get("expires_at")
        if not expires_at:
            return None
        return float(expires_at) - time.time()
    except Exception:
        return None


def _stop_services(skip_pids: set) -> None:
    """Stop envoy and brightstaff without killing the watchdog (self)."""
    from planoai.native_runner import stop_native

    stop_native(skip_pids=skip_pids)


def _do_restart(plano_config_file: str) -> bool:
    """
    Refresh the ChatGPT token, stop Envoy+brightstaff, restart with the new token.
    Returns True on success, False if the refresh failed.
    """
    # 1. Load saved state (env dict + metadata)
    if not os.path.exists(PLANO_WATCHDOG_STATE_FILE):
        _log("Watchdog state file missing — cannot restart services")
        return False
    with open(PLANO_WATCHDOG_STATE_FILE) as f:
        state = json.load(f)
    env = state["env"]
    with_tracing = state.get("with_tracing", False)

    # 2. Refresh the token
    try:
        from planoai.chatgpt_auth import get_access_token

        access_token, account_id = get_access_token()
    except Exception as exc:
        _log(
            f"Token refresh failed: {exc} — "
            "run 'planoai chatgpt login' to re-authenticate"
        )
        return False

    env["CHATGPT_ACCESS_TOKEN"] = access_token
    if account_id:
        env["CHATGPT_ACCOUNT_ID"] = account_id

    # 3. Stop envoy + brightstaff (skip self so we don't self-terminate)
    _stop_services(skip_pids={os.getpid()})

    # 4. Unset the sentinel so start_native() spawns a fresh watchdog
    os.environ.pop(_NO_WATCHDOG_ENV_VAR, None)

    # 5. Restart with the fresh token; this also spawns the next watchdog
    from planoai.native_runner import start_native

    start_native(
        plano_config_file,
        env,
        with_tracing=with_tracing,
        spawn_watchdog=True,
    )
    return True


def _watchdog_main(plano_config_file: str) -> None:
    """Main loop running inside the watchdog daemon process."""
    _log(f"Watchdog started (PID {os.getpid()})")

    while True:
        time.sleep(WATCHDOG_POLL_INTERVAL_SECONDS)

        secs = _seconds_until_expiry()
        if secs is None:
            _log("Cannot read token expiry — will retry next cycle")
            continue

        if secs > WATCHDOG_REFRESH_LEAD_SECONDS:
            continue  # Token still healthy

        _log(f"Token expires in {secs:.0f}s — refreshing and restarting services")
        success = _do_restart(plano_config_file)
        if not success:
            _log(
                "Restart failed — exiting watchdog. "
                "Services will continue until the token expires, "
                "then requests will fail. Run 'planoai chatgpt login' to fix."
            )
        # Either _do_restart spawned a new watchdog, or it failed.
        # Either way, this watchdog's job is done.
        return


def spawn_watchdog(plano_config_file: str) -> int:
    """
    Spawn a background watchdog daemon to monitor ChatGPT token expiry.

    Called from start_native() after services are healthy. Returns the watchdog
    daemon PID, or 0 if no watchdog was spawned (no ChatGPT providers, or
    recursive spawn was prevented by _NO_WATCHDOG_ENV_VAR).
    """
    # Prevent recursive spawning (watchdog calls start_native which calls us)
    if os.environ.get(_NO_WATCHDOG_ENV_VAR):
        return 0

    # Only spawn if the config has ChatGPT providers
    try:
        import yaml

        with open(plano_config_file) as f:
            config = yaml.safe_load(f)
        providers = config.get("model_providers") or config.get("llm_providers") or []
        has_chatgpt = any(
            str(p.get("model", "")).startswith("chatgpt/") for p in providers
        )
        if not has_chatgpt:
            return 0
    except Exception:
        return 0

    os.makedirs(PLANO_RUN_DIR, exist_ok=True)
    log_fd = os.open(
        PLANO_WATCHDOG_LOG_FILE, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644
    )

    # Double-fork to daemonize (mirrors _daemon_exec in native_runner.py)
    pid = os.fork()
    if pid > 0:
        # Parent: close log fd, wait for first child, read back grandchild PID
        os.close(log_fd)
        os.waitpid(pid, 0)
        grandchild_pid_path = os.path.join(PLANO_RUN_DIR, f".daemon_pid_{pid}")
        deadline = time.time() + 5
        while time.time() < deadline:
            if os.path.exists(grandchild_pid_path):
                with open(grandchild_pid_path) as f:
                    grandchild_pid = int(f.read().strip())
                os.unlink(grandchild_pid_path)
                return grandchild_pid
            time.sleep(0.05)
        os.close(log_fd) if False else None  # already closed above
        return 0  # Timed out — watchdog did not start

    # First child: create new session, fork again
    os.setsid()
    grandchild_pid = os.fork()
    if grandchild_pid > 0:
        # Intermediate child: write grandchild PID and exit
        pid_path = os.path.join(PLANO_RUN_DIR, f".daemon_pid_{os.getpid()}")
        with open(pid_path, "w") as f:
            f.write(str(grandchild_pid))
        os._exit(0)

    # Grandchild: the actual daemon
    os.dup2(log_fd, 1)  # stdout -> watchdog log
    os.dup2(log_fd, 2)  # stderr -> watchdog log
    os.close(log_fd)
    devnull = os.open(os.devnull, os.O_RDONLY)
    os.dup2(devnull, 0)
    os.close(devnull)

    # Set sentinel so any start_native() we call doesn't spawn another watchdog
    os.environ[_NO_WATCHDOG_ENV_VAR] = "1"

    try:
        _watchdog_main(plano_config_file)
    except Exception as exc:
        _log(f"Watchdog crashed: {exc}")
    finally:
        os._exit(0)
