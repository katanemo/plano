//! Manages the lifetime of one `claude -p` child process for a single
//! conversation session. Spawning, env scrubbing, NDJSON line reading and the
//! per-line watchdog all live here. Translation between Anthropic Messages
//! and stream-json lives in `hermesllm::apis::claude_cli`.

use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use hermesllm::apis::claude_cli::{parse_ndjson_line, ClaudeCliEvent, ClaudeCliInputEvent};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, Mutex, OwnedMutexGuard};
use tokio::time::{self, Instant};
use tracing::{debug, info, warn};

/// Tunables for one `ClaudeProcess`. Defaults match the OpenClaw reference
/// configuration: `bypassPermissions`, ~120 s watchdog window, ~10 min idle TTL.
#[derive(Debug, Clone)]
pub struct ClaudeCliConfig {
    /// Path or name of the `claude` binary (looked up via `$PATH`).
    pub binary: String,
    /// Value passed to `--permission-mode`. The CLI accepts `default`,
    /// `acceptEdits`, `plan`, `auto`, `dontAsk`, `bypassPermissions`.
    pub permission_mode: String,
    /// Idle session TTL — after this many seconds without a request the
    /// session manager kills the child.
    pub session_ttl: Duration,
    /// Per-line watchdog: if no NDJSON line arrives for this long during a
    /// turn, kill the child. Reset on every line (not every byte).
    pub watchdog: Duration,
}

impl Default for ClaudeCliConfig {
    fn default() -> Self {
        Self {
            binary: "claude".to_string(),
            permission_mode: "bypassPermissions".to_string(),
            session_ttl: Duration::from_secs(600),
            watchdog: Duration::from_secs(120),
        }
    }
}

/// Errors produced while interacting with the child process.
#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("failed to spawn `{binary}`: {source}")]
    Spawn {
        binary: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write to claude stdin: {0}")]
    StdinWrite(#[source] std::io::Error),
    #[error("claude process exited unexpectedly")]
    ExitedEarly,
    /// `Command::spawn` succeeded but a piped stdio handle was already taken
    /// by the time we asked for it. Should be unreachable given we set
    /// `Stdio::piped()` immediately before spawn; surfaced as its own variant
    /// so callers can tell it apart from a real "exited early".
    #[error("claude child is missing piped {which} after spawn")]
    MissingStdio { which: &'static str },
    #[error("claude watchdog fired after {0:?} of silence")]
    WatchdogTimeout(Duration),
    #[error("failed to serialize stdin payload: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("turn already in progress for this session")]
    TurnInProgress,
}

/// Strip down to the model alias / id the CLI's `--model` flag accepts.
/// Models registered via the wildcard `claude-cli/*` arrive prefixed with
/// `claude-cli/` (or just bare, e.g. `sonnet`); both forms are normalized
/// here.
pub fn normalize_model_arg(model: &str) -> &str {
    model.strip_prefix("claude-cli/").unwrap_or(model)
}

/// Environment variables that must be removed before exec'ing `claude` so the
/// child uses its own login keychain rather than picking up server-side
/// credentials. The list mirrors the OpenClaw scrub list.
const SCRUB_ENV_PREFIXES: &[&str] = &["ANTHROPIC_", "CLAUDE_CODE_", "OTEL_"];

fn scrubbed_env_for_spawn() -> Vec<(String, String)> {
    std::env::vars()
        .filter(|(k, _)| !SCRUB_ENV_PREFIXES.iter().any(|p| k.starts_with(p)))
        .collect()
}

/// One running `claude -p` subprocess plus the channels we use to talk to it.
/// Each `ClaudeProcess` is owned by exactly one session.
pub struct ClaudeProcess {
    child: Mutex<Option<Child>>,
    stdin: Mutex<Option<ChildStdin>>,
    /// The receiver of `ClaudeCliEvent`s parsed from the child's stdout.
    /// Wrapped in `Arc<Mutex>` so a `TurnStream` can hold an owned guard for
    /// the duration of one turn (which serializes turns within a session).
    event_rx: Arc<Mutex<mpsc::Receiver<ClaudeCliEvent>>>,
    config: ClaudeCliConfig,
    /// Last time a request was served on this session — used by the session
    /// manager to enforce the idle TTL. Held under a sync mutex because the
    /// critical section is one read/write of a `Copy` value with no `.await`,
    /// which keeps `SessionManager` callers from holding the session-map lock
    /// across an async hop.
    last_used: StdMutex<Instant>,
    /// Brightstaff-internal identifier — a deterministic UUID v5 derived from
    /// the conversation prefix (or supplied by the client header). Stable
    /// across retries so the manager can route follow-up turns to this same
    /// child. NEVER passed to `claude` itself.
    pub session_id: String,
    /// Per-spawn random UUID v4 passed to `claude --session-id`. Always fresh
    /// so we never collide with on-disk state (`~/.claude/projects/...`)
    /// from a previous run of the same conversation. Also stamped onto every
    /// stdin JSONL event so the CLI can verify the turn matches its session.
    cli_session_id: String,
}

impl ClaudeProcess {
    /// Spawn a new child for `session_id`. The first turn for a new session
    /// should be the user's Anthropic request body — see
    /// [`ClaudeProcess::send_user_turn`] for that.
    pub async fn spawn(
        session_id: String,
        model: &str,
        system_prompt: Option<&str>,
        cwd: Option<&std::path::Path>,
        config: ClaudeCliConfig,
    ) -> Result<Arc<Self>, ProcessError> {
        // Always hand the CLI a brand-new UUID. `--no-session-persistence`
        // does NOT actually prevent Claude Code from writing
        // `~/.claude/projects/<workspace>/<id>.jsonl` — it only blocks
        // resumability — so re-using our deterministic `session_id` would
        // collide with any prior run of the same conversation and the CLI
        // would exit with `Session ID ... is already in use`.
        let cli_session_id = uuid::Uuid::new_v4().to_string();

        let mut cmd = Command::new(&config.binary);
        cmd.arg("-p")
            .arg("--output-format")
            .arg("stream-json")
            .arg("--input-format")
            .arg("stream-json")
            .arg("--verbose")
            .arg("--include-partial-messages")
            .arg("--permission-mode")
            .arg(&config.permission_mode)
            .arg("--model")
            .arg(normalize_model_arg(model))
            .arg("--session-id")
            .arg(&cli_session_id)
            .arg("--no-session-persistence");

        if let Some(prompt) = system_prompt {
            // Append (don't replace) so Claude Code's built-in system prompt
            // — which carries tool definitions — is preserved.
            cmd.arg("--append-system-prompt").arg(prompt);
        }
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        cmd.env_clear();
        for (k, v) in scrubbed_env_for_spawn() {
            cmd.env(k, v);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|e| ProcessError::Spawn {
            binary: config.binary.clone(),
            source: e,
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or(ProcessError::MissingStdio { which: "stdin" })?;
        let stdout = child
            .stdout
            .take()
            .ok_or(ProcessError::MissingStdio { which: "stdout" })?;
        let stderr = child
            .stderr
            .take()
            .ok_or(ProcessError::MissingStdio { which: "stderr" })?;

        // Bounded channel — backpressure if the consumer is slow, but large
        // enough that bursts of small text deltas do not block stdout drain.
        let (tx, rx) = mpsc::channel::<ClaudeCliEvent>(256);

        let session_for_log = session_id.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            loop {
                match reader.next_line().await {
                    Ok(Some(line)) => {
                        if let Some(parsed) = parse_ndjson_line(&line) {
                            match parsed {
                                Ok(ev) => {
                                    if tx.send(ev).await.is_err() {
                                        break;
                                    }
                                }
                                Err(err) => {
                                    warn!(
                                        session = %session_for_log,
                                        error = %err,
                                        line = %line,
                                        "failed to parse claude NDJSON line"
                                    );
                                }
                            }
                        }
                    }
                    Ok(None) => {
                        debug!(session = %session_for_log, "claude stdout closed");
                        break;
                    }
                    Err(err) => {
                        warn!(
                            session = %session_for_log,
                            error = %err,
                            "claude stdout read error"
                        );
                        break;
                    }
                }
            }
        });

        let session_for_stderr = session_id.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if !line.trim().is_empty() {
                    warn!(session = %session_for_stderr, line = %line, "claude stderr");
                }
            }
        });

        info!(
            session = %session_id,
            cli_session = %cli_session_id,
            model = %normalize_model_arg(model),
            "spawned claude-cli"
        );

        Ok(Arc::new(Self {
            child: Mutex::new(Some(child)),
            stdin: Mutex::new(Some(stdin)),
            event_rx: Arc::new(Mutex::new(rx)),
            config,
            last_used: StdMutex::new(Instant::now()),
            session_id,
            cli_session_id,
        }))
    }

    /// The UUID that `claude --session-id` was launched with. The bridge has
    /// to stamp every stdin JSONL event with this id so the CLI accepts the
    /// turn as belonging to its current session — see
    /// [`Self::session_id`] for why this is distinct from the brightstaff
    /// session id.
    pub fn cli_session_id(&self) -> &str {
        &self.cli_session_id
    }

    /// Write the user-turn JSONL events to the child's stdin and return a
    /// stream that yields parsed CLI events for this turn until the terminal
    /// `result` event (or watchdog) ends it.
    ///
    /// Holds an exclusive lock on the event receiver for the duration of the
    /// turn, so concurrent calls return [`ProcessError::TurnInProgress`].
    pub async fn send_user_turn(
        &self,
        events: &[ClaudeCliInputEvent],
    ) -> Result<TurnStream, ProcessError> {
        // Sync lock + Copy value; never held across an `.await`.
        if let Ok(mut last) = self.last_used.lock() {
            *last = Instant::now();
        }

        // Claim the event receiver for the lifetime of this turn.
        let rx_guard = Arc::clone(&self.event_rx)
            .try_lock_owned()
            .map_err(|_| ProcessError::TurnInProgress)?;

        let mut stdin_guard = self.stdin.lock().await;
        let stdin = stdin_guard.as_mut().ok_or(ProcessError::ExitedEarly)?;
        for ev in events {
            let mut bytes = serde_json::to_vec(ev)?;
            bytes.push(b'\n');
            stdin
                .write_all(&bytes)
                .await
                .map_err(ProcessError::StdinWrite)?;
        }
        stdin.flush().await.map_err(ProcessError::StdinWrite)?;

        Ok(TurnStream {
            rx: rx_guard,
            watchdog: self.config.watchdog,
            done: false,
        })
    }

    /// Most-recent activity timestamp; used by the session manager's reaper.
    /// Sync because the lock guards a single `Instant` with no `.await` in
    /// the critical section — keeps callers from holding async locks across
    /// an await point.
    pub fn last_used(&self) -> Instant {
        // Poisoning is impossible here (the only writer is `send_user_turn`
        // which never panics while holding the lock), but if it ever happens
        // we degrade gracefully rather than aborting.
        self.last_used
            .lock()
            .map(|g| *g)
            .unwrap_or_else(|p| *p.into_inner())
    }

    /// Forcefully terminate the child. Safe to call multiple times.
    pub async fn shutdown(&self) {
        if let Some(mut child) = self.child.lock().await.take() {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
        // Dropping stdin signals the child if it survived `start_kill`.
        let _ = self.stdin.lock().await.take();
    }
}

/// One-shot stream of CLI events for a single user turn. Yields events until
/// the terminal `result` event is observed (or the watchdog fires). Drops the
/// owned receiver lock when finished, allowing the next turn to start.
pub struct TurnStream {
    rx: OwnedMutexGuard<mpsc::Receiver<ClaudeCliEvent>>,
    watchdog: Duration,
    done: bool,
}

impl TurnStream {
    /// Pull the next CLI event from the child, applying the per-line
    /// watchdog. Returns `Ok(None)` when the turn's terminal `result` event
    /// has been delivered.
    pub async fn next(&mut self) -> Result<Option<ClaudeCliEvent>, ProcessError> {
        if self.done {
            return Ok(None);
        }
        match time::timeout(self.watchdog, self.rx.recv()).await {
            Ok(Some(ev)) => {
                if matches!(ev, ClaudeCliEvent::Result { .. }) {
                    self.done = true;
                }
                Ok(Some(ev))
            }
            Ok(None) => {
                self.done = true;
                Err(ProcessError::ExitedEarly)
            }
            Err(_) => {
                self.done = true;
                Err(ProcessError::WatchdogTimeout(self.watchdog))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_model_arg_strips_prefix() {
        assert_eq!(normalize_model_arg("claude-cli/sonnet"), "sonnet");
        assert_eq!(
            normalize_model_arg("claude-cli/claude-opus-4-7"),
            "claude-opus-4-7"
        );
        assert_eq!(normalize_model_arg("sonnet"), "sonnet");
    }

    // Note: cannot mutate process env in unit tests safely since tests run
    // in parallel; spawn integration tests cover env behavior end-to-end via
    // the fake_claude.sh fixture.
}
