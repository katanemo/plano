//! HTTP server fronting the claude-cli bridge. Speaks Anthropic Messages API
//! (`POST /v1/messages`) on a localhost port; everything inside this module
//! delegates to `hermesllm::apis::claude_cli` for translation and to
//! `super::session::SessionManager` for subprocess lifecycle.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use futures::stream;
use hermesllm::apis::anthropic::MessagesRequest;
use hermesllm::apis::claude_cli::{
    cli_error_to_anthropic_error_body, cli_event_to_messages_stream_event,
    collect_to_messages_response, extract_system_prompt, messages_request_to_stdin_payload,
    synthetic_message_start, ClaudeCliEvent,
};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::header::{self, HeaderValue};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, error, info, warn};

use super::session::{SessionManager, SESSION_HEADER};

/// Spawn the claude-cli bridge listener. The returned `JoinHandle` resolves
/// when the listener loop exits (either via the provided shutdown signal or a
/// fatal accept error). On shutdown the manager drains all active sessions.
pub async fn run_listener<F>(
    addr: SocketAddr,
    manager: Arc<SessionManager>,
    shutdown: F,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let listener = TcpListener::bind(addr).await?;
    info!(%addr, "claude-cli bridge listening");

    let manager_for_shutdown = Arc::clone(&manager);
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            accept = listener.accept() => {
                let (stream, peer) = match accept {
                    Ok(s) => s,
                    Err(err) => {
                        warn!(error = ?err, "claude-cli accept error");
                        continue;
                    }
                };
                debug!(peer = ?peer, "claude-cli accepted connection");
                let manager = Arc::clone(&manager);
                let io = TokioIo::new(stream);
                tokio::task::spawn(async move {
                    let svc = service_fn(move |req| {
                        let manager = Arc::clone(&manager);
                        async move { handle(req, manager).await }
                    });
                    if let Err(err) = http1::Builder::new().serve_connection(io, svc).await {
                        warn!(error = ?err, "claude-cli connection error");
                    }
                });
            }
            _ = &mut shutdown => {
                info!("claude-cli bridge shutting down");
                manager_for_shutdown.shutdown_all().await;
                return Ok(());
            }
        }
    }
}

async fn handle(
    req: Request<Incoming>,
    manager: Arc<SessionManager>,
) -> Result<Response<BoxBody<Bytes, Infallible>>, hyper::Error> {
    let path = req.uri().path();
    let method = req.method();
    if method == Method::GET && path == "/healthz" {
        return Ok(text_response(StatusCode::OK, "ok"));
    }
    if method != Method::POST || path != "/v1/messages" {
        return Ok(text_response(StatusCode::NOT_FOUND, "not found"));
    }

    // Pull out the optional session header up front so we can drop the
    // request after consuming the body.
    let session_header = req
        .headers()
        .get(SESSION_HEADER)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());

    let body_bytes = match req.collect().await {
        Ok(c) => c.to_bytes(),
        Err(err) => {
            warn!(error = %err, "failed to read claude-cli request body");
            return Ok(json_error(StatusCode::BAD_REQUEST, "failed to read body"));
        }
    };

    let parsed: MessagesRequest = match serde_json::from_slice(&body_bytes) {
        Ok(p) => p,
        Err(err) => {
            warn!(error = %err, "failed to parse Anthropic MessagesRequest");
            return Ok(json_error(
                StatusCode::BAD_REQUEST,
                &format!("invalid Anthropic MessagesRequest: {err}"),
            ));
        }
    };

    let session_id = SessionManager::resolve_session_id(session_header.as_deref(), &parsed);
    let system_prompt = extract_system_prompt(&parsed);

    let process = match manager
        .get_or_spawn(&session_id, &parsed.model, system_prompt.as_deref(), None)
        .await
    {
        Ok(p) => p,
        Err(err) => {
            error!(session = %session_id, error = %err, "failed to spawn claude-cli");
            return Ok(json_error(
                StatusCode::BAD_GATEWAY,
                &format!("failed to spawn claude-cli: {err}"),
            ));
        }
    };

    let stdin_payload = match messages_request_to_stdin_payload(&parsed, Some(&session_id)) {
        Ok(p) => p,
        Err(err) => {
            warn!(error = %err, "failed to build claude-cli stdin payload");
            return Ok(json_error(
                StatusCode::BAD_REQUEST,
                &format!("failed to build claude-cli stdin payload: {err}"),
            ));
        }
    };

    let streaming = parsed.stream.unwrap_or(false);
    let model = parsed.model.clone();

    let mut turn = match process.send_user_turn(&stdin_payload).await {
        Ok(t) => t,
        Err(err) => {
            error!(session = %session_id, error = %err, "failed to send user turn");
            return Ok(json_error(
                StatusCode::BAD_GATEWAY,
                &format!("failed to send user turn: {err}"),
            ));
        }
    };

    if streaming {
        Ok(stream_response(turn, model, session_id))
    } else {
        // Drain the entire turn before answering.
        let mut events: Vec<ClaudeCliEvent> = Vec::new();
        loop {
            match turn.next().await {
                Ok(Some(ev)) => events.push(ev),
                Ok(None) => break,
                Err(err) => {
                    warn!(session = %session_id, error = %err, "claude-cli turn failed");
                    let body = cli_error_to_anthropic_error_body(&err.to_string());
                    return Ok(json_response(StatusCode::BAD_GATEWAY, &body));
                }
            }
        }
        match collect_to_messages_response(&model, events) {
            Ok(resp) => Ok(json_response(StatusCode::OK, &resp)),
            Err(err) => {
                let body = cli_error_to_anthropic_error_body(&err.to_string());
                Ok(json_response(StatusCode::BAD_GATEWAY, &body))
            }
        }
    }
}

fn stream_response(
    mut turn: super::process::TurnStream,
    model: String,
    session_id: String,
) -> Response<BoxBody<Bytes, Infallible>> {
    let (tx, rx) = mpsc::channel::<Result<Frame<Bytes>, Infallible>>(64);

    tokio::spawn(async move {
        // Some short turns skip MessageStart; emit a synthetic one so the
        // client always sees a complete stream.
        let mut emitted_message_start = false;

        loop {
            let ev = match turn.next().await {
                Ok(Some(ev)) => ev,
                Ok(None) => break,
                Err(err) => {
                    warn!(session = %session_id, error = %err, "claude-cli streaming turn failed");
                    let body = cli_error_to_anthropic_error_body(&err.to_string());
                    let frame =
                        Frame::data(format_sse("error", &serde_json::to_string(&body).unwrap()));
                    let _ = tx.send(Ok(frame)).await;
                    break;
                }
            };

            if !emitted_message_start {
                if let ClaudeCliEvent::StreamEvent {
                    event: hermesllm::apis::anthropic::MessagesStreamEvent::MessageStart { .. },
                } = &ev
                {
                    emitted_message_start = true;
                } else if matches!(&ev, ClaudeCliEvent::Result { .. }) {
                    // No actual content was streamed; synthesize a
                    // MessageStart so the SSE stream is well-formed.
                    let synthetic = synthetic_message_start(&model, Some(&session_id));
                    if let Some(frame) = sse_frame_for_event(&synthetic) {
                        let _ = tx.send(Ok(frame)).await;
                    }
                    emitted_message_start = true;
                }
            }

            if let Some(translated) = cli_event_to_messages_stream_event(&ev) {
                if let Some(frame) = sse_frame_for_event(&translated) {
                    if tx.send(Ok(frame)).await.is_err() {
                        break;
                    }
                }
            }

            if let ClaudeCliEvent::Result {
                is_error, result, ..
            } = &ev
            {
                if *is_error {
                    let msg = result
                        .clone()
                        .unwrap_or_else(|| "claude-cli returned an error".to_string());
                    let body = cli_error_to_anthropic_error_body(&msg);
                    let frame =
                        Frame::data(format_sse("error", &serde_json::to_string(&body).unwrap()));
                    let _ = tx.send(Ok(frame)).await;
                }
                break;
            }
        }
    });

    let body = StreamBody::new(ReceiverStream::new(rx));
    let mut resp = Response::new(body.boxed());
    *resp.status_mut() = StatusCode::OK;
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    resp.headers_mut()
        .insert("X-Accel-Buffering", HeaderValue::from_static("no"));
    resp
}

fn sse_frame_for_event(
    event: &hermesllm::apis::anthropic::MessagesStreamEvent,
) -> Option<Frame<Bytes>> {
    use hermesllm::apis::anthropic::MessagesStreamEvent;
    let event_name = match event {
        MessagesStreamEvent::MessageStart { .. } => "message_start",
        MessagesStreamEvent::ContentBlockStart { .. } => "content_block_start",
        MessagesStreamEvent::ContentBlockDelta { .. } => "content_block_delta",
        MessagesStreamEvent::ContentBlockStop { .. } => "content_block_stop",
        MessagesStreamEvent::MessageDelta { .. } => "message_delta",
        MessagesStreamEvent::MessageStop => "message_stop",
        MessagesStreamEvent::Ping => "ping",
    };
    let data = serde_json::to_string(event).ok()?;
    Some(Frame::data(format_sse(event_name, &data)))
}

fn format_sse(event: &str, data: &str) -> Bytes {
    Bytes::from(format!("event: {event}\ndata: {data}\n\n"))
}

fn json_response<T: serde::Serialize>(
    status: StatusCode,
    body: &T,
) -> Response<BoxBody<Bytes, Infallible>> {
    let bytes = serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec());
    let body = Full::new(Bytes::from(bytes))
        .map_err(|e| match e {})
        .boxed();
    let mut resp = Response::new(body);
    *resp.status_mut() = status;
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    resp
}

fn json_error(status: StatusCode, message: &str) -> Response<BoxBody<Bytes, Infallible>> {
    let body = cli_error_to_anthropic_error_body(message);
    json_response(status, &body)
}

fn text_response(
    status: StatusCode,
    message: &'static str,
) -> Response<BoxBody<Bytes, Infallible>> {
    let body = Full::new(Bytes::from_static(message.as_bytes()))
        .map_err(|e| match e {})
        .boxed();
    let mut resp = Response::new(body);
    *resp.status_mut() = status;
    resp.headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
    resp
}

// Ensure a no-op import so that `stream` (re-exported from futures) is
// considered used in case future expansion needs it. Avoids accidental
// deletion when running `cargo fix`.
#[allow(dead_code)]
fn _touch_stream_module() {
    let _: stream::Empty<u32> = stream::empty();
}
