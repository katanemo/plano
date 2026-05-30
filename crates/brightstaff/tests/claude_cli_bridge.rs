//! Integration test for the claude-cli bridge. Spins up the listener with a
//! fake `claude` shell script that emits a canned NDJSON sequence, then
//! verifies both the streaming SSE and non-streaming JSON code paths produce
//! the expected Anthropic Messages output.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use brightstaff::handlers::claude_cli::{
    self, ClaudeCliConfig, SessionManager, SessionManagerConfig,
};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

fn fake_claude_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("fake_claude.sh")
}

async fn pick_free_addr() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    addr
}

struct BridgeFixture {
    addr: std::net::SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl BridgeFixture {
    async fn start() -> Self {
        let addr = pick_free_addr().await;
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        let manager = SessionManager::new(SessionManagerConfig {
            max_sessions: 4,
            process: ClaudeCliConfig {
                binary: fake_claude_path().to_string_lossy().to_string(),
                permission_mode: "bypassPermissions".to_string(),
                session_ttl: Duration::from_secs(60),
                watchdog: Duration::from_secs(5),
            },
        });

        let manager_for_listener = Arc::clone(&manager);
        let handle = tokio::spawn(async move {
            let shutdown = async move {
                let _ = shutdown_rx.await;
            };
            if let Err(err) = claude_cli::run_listener(addr, manager_for_listener, shutdown).await {
                eprintln!("listener exited with error: {err}");
            }
        });

        // Wait for the listener to bind. Loop until we can connect.
        for _ in 0..50 {
            if tokio::net::TcpStream::connect(addr).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        Self {
            addr,
            shutdown: Some(shutdown_tx),
            handle: Some(handle),
        }
    }

    async fn stop(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(h) = self.handle.take() {
            let _ = tokio::time::timeout(Duration::from_secs(3), h).await;
        }
    }
}

/// Best-effort cleanup if a test panics before `stop().await`. We can't
/// `.await` from `Drop`, so we just abort the listener task; that's enough to
/// keep the runtime from leaking the spawned future.
impl Drop for BridgeFixture {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(h) = self.handle.take() {
            h.abort();
        }
    }
}

fn anthropic_request(stream: bool) -> Value {
    json!({
        "model": "claude-cli/sonnet",
        "max_tokens": 64,
        "stream": stream,
        "messages": [
            {"role": "user", "content": "say hi"}
        ]
    })
}

#[tokio::test]
async fn streaming_request_emits_anthropic_sse() {
    let fixture = BridgeFixture::start().await;
    let url = format!("http://{}/v1/messages", fixture.addr);

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .json(&anthropic_request(true))
        .send()
        .await
        .expect("send request");
    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        ct.starts_with("text/event-stream"),
        "expected text/event-stream, got {ct}"
    );
    let body = resp.text().await.expect("read body");

    // SSE event names should mirror Anthropic's wire format, in order.
    let events: Vec<&str> = body
        .lines()
        .filter_map(|l| l.strip_prefix("event: "))
        .collect();
    assert_eq!(
        events,
        vec![
            "message_start",
            "content_block_start",
            "content_block_delta",
            "content_block_delta",
            "content_block_stop",
            "message_delta",
            "message_stop",
        ],
        "unexpected SSE event sequence:\n{body}"
    );

    // The two text deltas should reconstruct "Hello, world!".
    let mut combined = String::new();
    for line in body.lines() {
        if let Some(payload) = line.strip_prefix("data: ") {
            if let Ok(v) = serde_json::from_str::<Value>(payload) {
                if v.get("type").and_then(|t| t.as_str()) == Some("content_block_delta") {
                    if let Some(text) = v
                        .get("delta")
                        .and_then(|d| d.get("text"))
                        .and_then(|t| t.as_str())
                    {
                        combined.push_str(text);
                    }
                }
            }
        }
    }
    assert_eq!(combined, "Hello, world!");

    fixture.stop().await;
}

#[tokio::test]
async fn non_streaming_request_returns_messages_response() {
    let fixture = BridgeFixture::start().await;
    let url = format!("http://{}/v1/messages", fixture.addr);

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .json(&anthropic_request(false))
        .send()
        .await
        .expect("send request");
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.expect("parse json");

    assert_eq!(body["type"], "message");
    assert_eq!(body["role"], "assistant");
    assert_eq!(body["stop_reason"], "end_turn");
    assert_eq!(body["usage"]["input_tokens"], 3);
    assert_eq!(body["usage"]["output_tokens"], 4);
    let content = body["content"].as_array().expect("content array");
    assert_eq!(content.len(), 1);
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[0]["text"], "Hello, world!");

    fixture.stop().await;
}
