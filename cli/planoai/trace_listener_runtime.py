"""
Trace listener process runtime utilities.
"""

import os
import signal
import time
import logging
from collections.abc import Callable

# Canonical PID file used by `planoai trace listen/down`.
TRACE_LISTENER_PID_PATH = os.path.expanduser("~/.plano/run/trace_listener.pid")
TRACE_LISTENER_LOG_PATH = os.path.expanduser("~/.plano/run/trace_listener.log")
LOGGER = logging.getLogger(__name__)


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
        LOGGER.warning(
            "Removing stale or malformed trace listener PID file at %s",
            TRACE_LISTENER_PID_PATH,
        )
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

    # Redirect stdin to /dev/null and stdout/stderr to a persistent log file.
    # This keeps the daemon terminal-independent while preserving diagnostics.
    os.makedirs(os.path.dirname(TRACE_LISTENER_LOG_PATH), exist_ok=True)
    devnull_in = os.open(os.devnull, os.O_RDONLY)
    try:
        log_fd = os.open(
            TRACE_LISTENER_LOG_PATH,
            os.O_WRONLY | os.O_CREAT | os.O_APPEND,
            0o644,
        )
    except OSError:
        # If logging cannot be initialized, keep running with output discarded.
        log_fd = os.open(os.devnull, os.O_WRONLY)
    os.dup2(devnull_in, 0)  # stdin
    os.dup2(log_fd, 1)  # stdout
    os.dup2(log_fd, 2)  # stderr
    if devnull_in > 2:
        os.close(devnull_in)
    if log_fd > 2:
        os.close(log_fd)

    # Run the daemon main loop (expected to block until process termination).
    run_forever()

    # If callback unexpectedly returns, exit cleanly to avoid returning to parent context.
    os._exit(0)
