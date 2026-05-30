//! Bridge that exposes the local `claude` CLI as an Anthropic Messages API
//! endpoint on a localhost port, allowing it to be used as just another
//! `model_provider` in Plano.
//!
//! Wire-up:
//! - `process` — spawns and manages the `claude -p --output-format stream-json
//!   --input-format stream-json` subprocess.
//! - `session` — keys long-lived processes by session id (header or hash) and
//!   enforces idle TTL / cap.
//! - `server` — hyper listener that speaks `POST /v1/messages` and bridges
//!   between Anthropic SSE and the CLI's NDJSON.
//!
//! Translation between the two wire formats lives in
//! `hermesllm::apis::claude_cli`; this module only owns runtime concerns.

pub mod process;
pub mod server;
pub mod session;

pub use process::{ClaudeCliConfig, ClaudeProcess, ProcessError};
pub use server::run_listener;
pub use session::{SessionManager, SessionManagerConfig, SESSION_HEADER};
