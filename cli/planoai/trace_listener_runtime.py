"""
Trace listener process runtime utilities.
"""

import os
import signal
import time
from collections.abc import Callable

# Canonical PID file used by `planoai trace listen/down`.
TRACE_LISTENER_PID_PATH = os.path.expanduser("~/.plano/run/trace_listener.pid")


def write_listener_pid(pid: int) -> None:
    """Persist listener PID for later management commands."""
    # Ensure parent directory exists for first-time installs.
    os.makedirs(os.path.dirname(TRACE_LISTENER_PID_PATH), exist_ok=True)
    with open(TRACE_LISTENER_PID_PATH, "w") as f:
        f.write(str(pid))


def remove_listener_pid() -> None:
    """Remove persisted listener PID file if present."""
    # Best-effort cleanup; missing file is not an error.
    if os.path.exists(TRACE_LISTENER_PID_PATH):
        os.remove(TRACE_LISTENER_PID_PATH)


def get_listener_pid() -> int | None:
    """Return listener PID if present and process is alive."""
    if not os.path.exists(TRACE_LISTENER_PID_PATH):
        return None

    try:
        # Parse persisted PID.
        with open(TRACE_LISTENER_PID_PATH, "r") as f:
            pid = int(f.read().strip())
        # Signal 0 performs liveness check without sending a real signal.
        os.kill(pid, 0)
        return pid
    except (ValueError, ProcessLookupError, OSError):
        # Stale or malformed PID file: clean it up to prevent repeated confusion.
        remove_listener_pid()
        return None


def stop_listener_process(grace_seconds: float = 0.5) -> bool:
    """Stop persisted listener process, returning True if one was stopped."""
    pid = get_listener_pid()
    if pid is None:
        return False

    try:
        # Try graceful shutdown first.
        os.kill(pid, signal.SIGTERM)
        # Allow the process a short window to exit cleanly.
        time.sleep(grace_seconds)
        try:
            # If still alive, force terminate.
            os.kill(pid, 0)
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            # Already exited after SIGTERM.
            pass
        remove_listener_pid()
        return True
    except ProcessLookupError:
        # Process disappeared between checks; treat as already stopped.
        remove_listener_pid()
        return False


def daemonize_and_run(run_forever: Callable[[], None]) -> int | None:
    """
    Fork and detach process to create a Unix daemon.

    Returns:
    - Parent process: child PID (> 0), allowing caller to report startup.
    - Child process: never returns; runs callback in daemon context until termination.

    Raises:
    - OSError: if fork fails (e.g., resource limits exceeded).
    """
    # Duplicate current process. Raises OSError if fork fails.
    pid = os.fork()
    if pid > 0:
        # Parent returns child PID to caller.
        return pid

    # Child: detach from controlling terminal/session.
    # This prevents SIGHUP when parent terminal closes and ensures
    # the daemon cannot reacquire a controlling terminal.
    os.setsid()

    # Redirect stdin/stdout/stderr to /dev/null so daemon is terminal-independent.
    # This prevents broken pipe errors and ensures no output leaks to the parent terminal.
    devnull = os.open(os.devnull, os.O_RDWR)
    os.dup2(devnull, 0)  # stdin
    os.dup2(devnull, 1)  # stdout
    os.dup2(devnull, 2)  # stderr
    if devnull > 2:
        os.close(devnull)

    # Run the daemon main loop (expected to block until process termination).
    run_forever()

    # If callback unexpectedly returns, exit cleanly to avoid returning to parent context.
    os._exit(0)
