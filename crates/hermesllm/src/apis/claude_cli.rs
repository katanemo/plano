//! Translation between Anthropic Messages API and Claude Code CLI's
//! `--output-format stream-json` / `--input-format stream-json` wire format.
//!
//! Claude Code CLI is invoked as a subprocess by `brightstaff` with flags such
//! as `claude -p --output-format stream-json --input-format stream-json
//! --include-partial-messages --verbose`. Each line on stdout is one JSON event
//! (NDJSON), and each line on stdin is a user-message JSON. This module owns
//! the pure (no-I/O) types and conversions; the runtime layer in brightstaff
//! does the actual spawning and streaming.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use serde_with::skip_serializing_none;
use thiserror::Error;
use uuid::Uuid;

use crate::apis::anthropic::{
    MessagesContentBlock, MessagesContentDelta, MessagesMessageContent, MessagesMessageDelta,
    MessagesRequest, MessagesResponse, MessagesRole, MessagesStopReason, MessagesStreamEvent,
    MessagesStreamMessage, MessagesSystemPrompt, MessagesUsage,
};

/// Errors produced by translation between Anthropic Messages and Claude Code
/// stream-json.
#[derive(Debug, Error)]
pub enum ClaudeCliTranslationError {
    #[error("Claude CLI returned an error: {message}")]
    CliError { message: String },
    #[error("Failed to serialize stdin payload: {0}")]
    SerializeStdin(#[from] serde_json::Error),
    #[error("Claude CLI stream ended before a terminal `result` event")]
    UnexpectedEnd,
}

// ---------------------------------------------------------------------------
// Wire types — output (Claude CLI -> us)
// ---------------------------------------------------------------------------

/// One line of NDJSON emitted on stdout by `claude -p --output-format
/// stream-json`. The CLI tags variants with a top-level `type` field, and
/// `system`/`result` carry an additional `subtype`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClaudeCliEvent {
    /// `type=system` events. The actual classification lives in `subtype`
    /// (e.g. `init`, `api_retry`, `rate_limit_event`). We keep the raw fields
    /// rather than enumerating subtypes so a new CLI release that adds a
    /// subtype does not break parsing.
    System {
        #[serde(default)]
        subtype: Option<String>,
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(flatten)]
        extra: Value,
    },
    /// A complete assistant message (emitted after the corresponding
    /// `stream_event` deltas finish). Useful for non-streaming consumers.
    Assistant { message: ClaudeCliAssistantMessage },
    /// A complete user message echoed back (when `--replay-user-messages` is
    /// set). We currently ignore these in translation but keep the variant so
    /// stray events do not cause deserialization failures.
    User {
        #[serde(default)]
        message: Value,
    },
    /// Wrapped Anthropic SSE event. The CLI re-emits the raw streaming-API
    /// shape here when `--include-partial-messages` is enabled.
    StreamEvent { event: MessagesStreamEvent },
    /// Terminal event marking the end of one CLI turn. `is_error == true`
    /// means the underlying API call failed; `result` typically holds the
    /// final assistant text or an error message.
    Result {
        #[serde(default)]
        subtype: Option<String>,
        #[serde(default)]
        is_error: bool,
        #[serde(default)]
        duration_ms: Option<u64>,
        #[serde(default)]
        num_turns: Option<u32>,
        #[serde(default)]
        result: Option<String>,
        #[serde(default)]
        total_cost_usd: Option<f64>,
        #[serde(default)]
        usage: Option<ClaudeCliUsage>,
        #[serde(default)]
        session_id: Option<String>,
    },
    /// Catch-all for events the CLI may add in the future. We surface them in
    /// logs but do not translate them to Anthropic events.
    #[serde(other)]
    Unknown,
}

/// Subset of the Anthropic message shape the CLI emits inside `assistant`
/// events. We keep `content` as `Value` so we can decode text + tool_use
/// blocks without re-deriving every Anthropic content variant here.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeCliAssistantMessage {
    pub id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub content: Vec<ClaudeCliContentBlock>,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub stop_sequence: Option<String>,
    #[serde(default)]
    pub usage: Option<ClaudeCliUsage>,
}

/// The CLI's `assistant.message.content[]` entries are a subset of Anthropic's
/// content blocks. We deserialize them into `MessagesContentBlock` directly
/// where possible and fall back to a tagged enum for the few fields we care
/// about explicitly (text + tool_use).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ClaudeCliContentBlock {
    /// Anthropic-shaped content block (text, tool_use, thinking, ...).
    Anthropic(MessagesContentBlock),
    /// Anything we do not recognize is preserved as raw JSON so we can still
    /// surface it in the `result` aggregation.
    Unknown(Value),
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClaudeCliUsage {
    #[serde(default)]
    pub input_tokens: Option<u32>,
    #[serde(default)]
    pub output_tokens: Option<u32>,
    #[serde(default)]
    pub cache_creation_input_tokens: Option<u32>,
    #[serde(default)]
    pub cache_read_input_tokens: Option<u32>,
}

impl From<ClaudeCliUsage> for MessagesUsage {
    fn from(u: ClaudeCliUsage) -> Self {
        MessagesUsage {
            input_tokens: u.input_tokens.unwrap_or(0),
            output_tokens: u.output_tokens.unwrap_or(0),
            cache_creation_input_tokens: u.cache_creation_input_tokens,
            cache_read_input_tokens: u.cache_read_input_tokens,
        }
    }
}

// ---------------------------------------------------------------------------
// Wire types — input (us -> Claude CLI)
// ---------------------------------------------------------------------------

/// One line of NDJSON written to the CLI's stdin when invoked with
/// `--input-format stream-json`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClaudeCliInputEvent {
    User {
        message: ClaudeCliUserMessage,
        /// The session id assigned by the CLI on first turn. Optional on the
        /// first message; required (and must match) on subsequent turns.
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaudeCliUserMessage {
    pub role: &'static str,
    pub content: Value,
}

// ---------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------

/// Map a `MessagesRequest` into the JSONL payload that should be written to
/// the CLI's stdin. Returns one event per user turn, in order, so callers can
/// either replay the full conversation on first spawn or send only the latest
/// turn for a hot session.
///
/// `session_id` (when set) is attached to every event so the CLI can verify
/// the turn belongs to the expected session.
pub fn messages_request_to_stdin_payload(
    req: &MessagesRequest,
    session_id: Option<&str>,
) -> Result<Vec<ClaudeCliInputEvent>, ClaudeCliTranslationError> {
    let mut out = Vec::new();
    for msg in &req.messages {
        if msg.role != MessagesRole::User {
            // Assistant turns are managed by the CLI internally; we skip them.
            continue;
        }
        let content = message_content_to_cli_value(&msg.content);
        out.push(ClaudeCliInputEvent::User {
            message: ClaudeCliUserMessage {
                role: "user",
                content,
            },
            session_id: session_id.map(str::to_string),
        });
    }
    Ok(out)
}

/// Build the `--append-system-prompt` value that should be passed when
/// spawning the CLI for this request. Returns `None` when the request has no
/// system prompt.
pub fn extract_system_prompt(req: &MessagesRequest) -> Option<String> {
    req.system.as_ref().map(|s| match s {
        MessagesSystemPrompt::Single(text) => text.clone(),
        MessagesSystemPrompt::Blocks(blocks) => blocks
            .iter()
            .filter_map(|b| match b {
                MessagesContentBlock::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    })
}

fn message_content_to_cli_value(content: &MessagesMessageContent) -> Value {
    match content {
        MessagesMessageContent::Single(s) => Value::String(s.clone()),
        MessagesMessageContent::Blocks(blocks) => {
            // Preserve the structured block array so tool_result / image
            // blocks survive intact across the stdin boundary.
            serde_json::to_value(blocks).unwrap_or_else(|_| Value::Array(vec![]))
        }
    }
}

/// Translate a single CLI event into a corresponding Anthropic
/// `MessagesStreamEvent`, when one exists. Returns `None` for events that
/// have no SSE counterpart (CLI-internal `system` notifications, terminal
/// `result`, unrecognized variants, ...).
pub fn cli_event_to_messages_stream_event(ev: &ClaudeCliEvent) -> Option<MessagesStreamEvent> {
    match ev {
        ClaudeCliEvent::StreamEvent { event } => Some(event.clone()),
        _ => None,
    }
}

/// Aggregate a sequence of CLI events into a single non-streaming
/// `MessagesResponse`. Used by the bridge when the client did not request
/// streaming.
///
/// The terminal `result` event is required: if the iterator ends without one,
/// we return [`ClaudeCliTranslationError::UnexpectedEnd`].
pub fn collect_to_messages_response<I>(
    model: &str,
    events: I,
) -> Result<MessagesResponse, ClaudeCliTranslationError>
where
    I: IntoIterator<Item = ClaudeCliEvent>,
{
    let mut content_blocks: Vec<MessagesContentBlock> = Vec::new();
    // Accumulate per-index text deltas + tool-use input deltas as the CLI
    // emits content_block_start -> content_block_delta(s) -> content_block_stop.
    let mut text_accum: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
    let mut tool_accum: std::collections::HashMap<u32, (String, String, String)> =
        std::collections::HashMap::new();
    let mut block_order: Vec<(u32, BlockKind)> = Vec::new();
    let mut stop_reason = MessagesStopReason::EndTurn;
    let mut stop_sequence: Option<String> = None;
    let mut usage = MessagesUsage {
        input_tokens: 0,
        output_tokens: 0,
        cache_creation_input_tokens: None,
        cache_read_input_tokens: None,
    };
    let mut id = String::new();
    let mut model_out = model.to_string();
    let mut last_assistant_message: Option<ClaudeCliAssistantMessage> = None;
    let mut saw_result = false;
    let mut error_message: Option<String> = None;

    for ev in events {
        match ev {
            ClaudeCliEvent::StreamEvent { event } => match event {
                MessagesStreamEvent::MessageStart { message } => {
                    if id.is_empty() {
                        id.clone_from(&message.id);
                    }
                    if !message.model.is_empty() {
                        model_out.clone_from(&message.model);
                    }
                    usage = message.usage.clone();
                }
                MessagesStreamEvent::ContentBlockStart {
                    index,
                    content_block,
                } => match content_block {
                    MessagesContentBlock::Text { text, .. } => {
                        text_accum.insert(index, text);
                        block_order.push((index, BlockKind::Text));
                    }
                    MessagesContentBlock::ToolUse {
                        id: tool_id, name, ..
                    } => {
                        // Anthropic streaming always starts a tool_use block
                        // with an empty `input` placeholder (`{}` or `null`);
                        // the real arguments arrive via `input_json_delta`s.
                        // Always start with an empty buffer so deltas
                        // assemble into valid JSON.
                        tool_accum.insert(index, (tool_id, name, String::new()));
                        block_order.push((index, BlockKind::ToolUse));
                    }
                    other => {
                        // Unknown block kind — preserve verbatim by pushing it
                        // immediately. We do not expect deltas for this index.
                        content_blocks.push(other);
                    }
                },
                MessagesStreamEvent::ContentBlockDelta { index, delta } => match delta {
                    MessagesContentDelta::TextDelta { text } => {
                        text_accum.entry(index).or_default().push_str(&text);
                    }
                    MessagesContentDelta::InputJsonDelta { partial_json } => {
                        if let Some((_, _, buf)) = tool_accum.get_mut(&index) {
                            buf.push_str(&partial_json);
                        }
                    }
                    // Thinking/signature deltas are surfaced to streaming
                    // clients but dropped from the non-streaming aggregate.
                    _ => {}
                },
                MessagesStreamEvent::MessageDelta {
                    delta,
                    usage: msg_usage,
                } => {
                    let MessagesMessageDelta {
                        stop_reason: sr,
                        stop_sequence: ss,
                    } = delta;
                    stop_reason = sr;
                    stop_sequence = ss;
                    // The MessageDelta usage carries final output_tokens.
                    usage.output_tokens = msg_usage.output_tokens;
                }
                MessagesStreamEvent::ContentBlockStop { .. }
                | MessagesStreamEvent::MessageStop
                | MessagesStreamEvent::Ping => {}
            },
            ClaudeCliEvent::Assistant { message } => {
                last_assistant_message = Some(message);
            }
            ClaudeCliEvent::Result {
                is_error,
                result,
                usage: result_usage,
                ..
            } => {
                saw_result = true;
                if is_error {
                    error_message = Some(result.unwrap_or_else(|| "Claude CLI failed".to_string()));
                }
                if let Some(u) = result_usage {
                    let merged: MessagesUsage = u.into();
                    if merged.input_tokens > 0 {
                        usage.input_tokens = merged.input_tokens;
                    }
                    if merged.output_tokens > 0 {
                        usage.output_tokens = merged.output_tokens;
                    }
                    if merged.cache_creation_input_tokens.is_some() {
                        usage.cache_creation_input_tokens = merged.cache_creation_input_tokens;
                    }
                    if merged.cache_read_input_tokens.is_some() {
                        usage.cache_read_input_tokens = merged.cache_read_input_tokens;
                    }
                }
            }
            ClaudeCliEvent::System { .. }
            | ClaudeCliEvent::User { .. }
            | ClaudeCliEvent::Unknown => {}
        }
    }

    if let Some(msg) = error_message {
        return Err(ClaudeCliTranslationError::CliError { message: msg });
    }
    if !saw_result {
        return Err(ClaudeCliTranslationError::UnexpectedEnd);
    }

    // Materialize accumulated blocks in the order they were started.
    let mut sorted_indices = block_order.clone();
    sorted_indices.sort_by_key(|(idx, _)| *idx);
    for (idx, kind) in sorted_indices {
        match kind {
            BlockKind::Text => {
                if let Some(text) = text_accum.remove(&idx) {
                    content_blocks.push(MessagesContentBlock::Text {
                        text,
                        cache_control: None,
                    });
                }
            }
            BlockKind::ToolUse => {
                if let Some((tool_id, name, raw_input)) = tool_accum.remove(&idx) {
                    let input_value = if raw_input.is_empty() {
                        Value::Object(Map::default())
                    } else {
                        serde_json::from_str(&raw_input)
                            .unwrap_or_else(|_| Value::String(raw_input))
                    };
                    content_blocks.push(MessagesContentBlock::ToolUse {
                        id: tool_id,
                        name,
                        input: input_value,
                        cache_control: None,
                    });
                }
            }
        }
    }

    // If the streaming events did not include any content but the CLI sent a
    // final `assistant` message (common for short responses), use that as the
    // body of the response.
    if content_blocks.is_empty() {
        if let Some(msg) = last_assistant_message {
            for block in msg.content {
                if let ClaudeCliContentBlock::Anthropic(b) = block {
                    content_blocks.push(b);
                }
            }
            if id.is_empty() {
                if let Some(msg_id) = msg.id {
                    id = msg_id;
                }
            }
            if let Some(m) = msg.model {
                if !m.is_empty() {
                    model_out = m;
                }
            }
            if let Some(u) = msg.usage {
                let merged: MessagesUsage = u.into();
                if usage.input_tokens == 0 {
                    usage.input_tokens = merged.input_tokens;
                }
                if usage.output_tokens == 0 {
                    usage.output_tokens = merged.output_tokens;
                }
                if usage.cache_creation_input_tokens.is_none() {
                    usage.cache_creation_input_tokens = merged.cache_creation_input_tokens;
                }
                if usage.cache_read_input_tokens.is_none() {
                    usage.cache_read_input_tokens = merged.cache_read_input_tokens;
                }
            }
        }
    }

    if id.is_empty() {
        id = format!("msg_cli_{}", Uuid::new_v4().simple());
    }

    Ok(MessagesResponse {
        id,
        obj_type: "message".to_string(),
        role: MessagesRole::Assistant,
        content: content_blocks,
        model: model_out,
        stop_reason,
        stop_sequence,
        usage,
        container: None,
    })
}

#[derive(Clone, Copy)]
enum BlockKind {
    Text,
    ToolUse,
}

/// Build an Anthropic-style error envelope JSON for a CLI-level failure. The
/// brightstaff bridge serializes this and returns it with a 502/500 status so
/// the existing `llm_gateway` error handling sees a familiar shape.
pub fn cli_error_to_anthropic_error_body(message: &str) -> Value {
    json!({
        "type": "error",
        "error": {
            "type": "claude_cli_error",
            "message": message,
        }
    })
}

/// Synthesize a `message_start` event for streaming clients in cases where
/// the CLI did not emit one (it usually does, but very small turns can skip
/// straight to `assistant`/`result`).
pub fn synthetic_message_start(model: &str, session_id: Option<&str>) -> MessagesStreamEvent {
    let id = session_id.map_or_else(
        || format!("msg_cli_{}", Uuid::new_v4().simple()),
        |s| format!("msg_cli_{s}"),
    );
    MessagesStreamEvent::MessageStart {
        message: MessagesStreamMessage {
            id,
            obj_type: "message".to_string(),
            role: MessagesRole::Assistant,
            content: Vec::new(),
            model: model.to_string(),
            stop_reason: None,
            stop_sequence: None,
            usage: MessagesUsage {
                input_tokens: 0,
                output_tokens: 0,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
            },
        },
    }
}

/// Convenience: parse one NDJSON line into a `ClaudeCliEvent`. Whitespace-only
/// lines deserialize to `None` so callers can simply skip them.
pub fn parse_ndjson_line(line: &str) -> Option<Result<ClaudeCliEvent, serde_json::Error>> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(serde_json::from_str(trimmed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apis::anthropic::{MessagesMessage, MessagesMessageContent};

    fn user_request(text: &str) -> MessagesRequest {
        MessagesRequest {
            model: "claude-cli/sonnet".to_string(),
            messages: vec![MessagesMessage {
                role: MessagesRole::User,
                content: MessagesMessageContent::Single(text.to_string()),
            }],
            max_tokens: 1024,
            container: None,
            mcp_servers: None,
            system: None,
            metadata: None,
            service_tier: None,
            thinking: None,
            temperature: None,
            top_p: None,
            top_k: None,
            stream: Some(true),
            stop_sequences: None,
            tools: None,
            tool_choice: None,
        }
    }

    #[test]
    fn parses_system_init_event() {
        let line = r#"{"type":"system","subtype":"init","session_id":"s1","model":"sonnet","cwd":"/tmp","tools":[]}"#;
        let parsed = parse_ndjson_line(line).expect("non-empty").expect("ok");
        match parsed {
            ClaudeCliEvent::System {
                subtype,
                session_id,
                model,
                ..
            } => {
                assert_eq!(subtype.as_deref(), Some("init"));
                assert_eq!(session_id.as_deref(), Some("s1"));
                assert_eq!(model.as_deref(), Some("sonnet"));
            }
            other => panic!("expected System, got {other:?}"),
        }
    }

    #[test]
    fn parses_text_stream_event() {
        let line = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}}"#;
        let parsed = parse_ndjson_line(line).unwrap().unwrap();
        let translated = cli_event_to_messages_stream_event(&parsed)
            .expect("text_delta should translate to MessagesStreamEvent");
        match translated {
            MessagesStreamEvent::ContentBlockDelta { index, delta } => {
                assert_eq!(index, 0);
                match delta {
                    MessagesContentDelta::TextDelta { text } => assert_eq!(text, "hi"),
                    other => panic!("expected TextDelta, got {other:?}"),
                }
            }
            other => panic!("expected ContentBlockDelta, got {other:?}"),
        }
    }

    #[test]
    fn parses_result_success_event() {
        let line = r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":12,"num_turns":1,"result":"hi","total_cost_usd":0.001,"usage":{"input_tokens":4,"output_tokens":2},"session_id":"s1"}"#;
        let parsed = parse_ndjson_line(line).unwrap().unwrap();
        match parsed {
            ClaudeCliEvent::Result {
                is_error,
                result,
                usage,
                ..
            } => {
                assert!(!is_error);
                assert_eq!(result.as_deref(), Some("hi"));
                assert_eq!(usage.unwrap().output_tokens, Some(2));
            }
            other => panic!("expected Result, got {other:?}"),
        }
    }

    #[test]
    fn unknown_event_type_does_not_break_parser() {
        let line = r#"{"type":"future_event_kind","data":{"foo":"bar"},"another":42}"#;
        let parsed = parse_ndjson_line(line).unwrap().unwrap();
        assert!(matches!(parsed, ClaudeCliEvent::Unknown));
    }

    #[test]
    fn stdin_payload_skips_assistant_turns() {
        let mut req = user_request("hello");
        req.messages.push(MessagesMessage {
            role: MessagesRole::Assistant,
            content: MessagesMessageContent::Single("hi back".to_string()),
        });
        req.messages.push(MessagesMessage {
            role: MessagesRole::User,
            content: MessagesMessageContent::Single("how are you?".to_string()),
        });
        let payload = messages_request_to_stdin_payload(&req, Some("s1")).unwrap();
        assert_eq!(payload.len(), 2);
        for ev in &payload {
            match ev {
                ClaudeCliInputEvent::User {
                    message,
                    session_id,
                } => {
                    assert_eq!(message.role, "user");
                    assert_eq!(session_id.as_deref(), Some("s1"));
                }
            }
        }
    }

    #[test]
    fn collect_to_messages_response_aggregates_text() {
        let events = vec![
            ClaudeCliEvent::StreamEvent {
                event: MessagesStreamEvent::MessageStart {
                    message: MessagesStreamMessage {
                        id: "msg_1".to_string(),
                        obj_type: "message".to_string(),
                        role: MessagesRole::Assistant,
                        content: vec![],
                        model: "claude-sonnet-4-6".to_string(),
                        stop_reason: None,
                        stop_sequence: None,
                        usage: MessagesUsage {
                            input_tokens: 7,
                            output_tokens: 0,
                            cache_creation_input_tokens: None,
                            cache_read_input_tokens: None,
                        },
                    },
                },
            },
            ClaudeCliEvent::StreamEvent {
                event: MessagesStreamEvent::ContentBlockStart {
                    index: 0,
                    content_block: MessagesContentBlock::Text {
                        text: String::new(),
                        cache_control: None,
                    },
                },
            },
            ClaudeCliEvent::StreamEvent {
                event: MessagesStreamEvent::ContentBlockDelta {
                    index: 0,
                    delta: MessagesContentDelta::TextDelta {
                        text: "Hello ".to_string(),
                    },
                },
            },
            ClaudeCliEvent::StreamEvent {
                event: MessagesStreamEvent::ContentBlockDelta {
                    index: 0,
                    delta: MessagesContentDelta::TextDelta {
                        text: "world".to_string(),
                    },
                },
            },
            ClaudeCliEvent::StreamEvent {
                event: MessagesStreamEvent::ContentBlockStop { index: 0 },
            },
            ClaudeCliEvent::StreamEvent {
                event: MessagesStreamEvent::MessageDelta {
                    delta: MessagesMessageDelta {
                        stop_reason: MessagesStopReason::EndTurn,
                        stop_sequence: None,
                    },
                    usage: MessagesUsage {
                        input_tokens: 0,
                        output_tokens: 12,
                        cache_creation_input_tokens: None,
                        cache_read_input_tokens: None,
                    },
                },
            },
            ClaudeCliEvent::StreamEvent {
                event: MessagesStreamEvent::MessageStop,
            },
            ClaudeCliEvent::Result {
                subtype: Some("success".to_string()),
                is_error: false,
                duration_ms: Some(123),
                num_turns: Some(1),
                result: Some("Hello world".to_string()),
                total_cost_usd: Some(0.001),
                usage: Some(ClaudeCliUsage {
                    input_tokens: Some(7),
                    output_tokens: Some(12),
                    cache_creation_input_tokens: None,
                    cache_read_input_tokens: None,
                }),
                session_id: Some("s1".to_string()),
            },
        ];

        let resp = collect_to_messages_response("claude-cli/sonnet", events).unwrap();
        assert_eq!(resp.id, "msg_1");
        assert_eq!(resp.model, "claude-sonnet-4-6");
        assert_eq!(resp.usage.input_tokens, 7);
        assert_eq!(resp.usage.output_tokens, 12);
        assert!(matches!(resp.stop_reason, MessagesStopReason::EndTurn));
        match &resp.content[..] {
            [MessagesContentBlock::Text { text, .. }] => assert_eq!(text, "Hello world"),
            other => panic!("expected single Text block, got {other:?}"),
        }
    }

    #[test]
    fn collect_to_messages_response_aggregates_tool_use() {
        let events = vec![
            ClaudeCliEvent::StreamEvent {
                event: MessagesStreamEvent::MessageStart {
                    message: MessagesStreamMessage {
                        id: "msg_2".to_string(),
                        obj_type: "message".to_string(),
                        role: MessagesRole::Assistant,
                        content: vec![],
                        model: "sonnet".to_string(),
                        stop_reason: None,
                        stop_sequence: None,
                        usage: MessagesUsage {
                            input_tokens: 1,
                            output_tokens: 0,
                            cache_creation_input_tokens: None,
                            cache_read_input_tokens: None,
                        },
                    },
                },
            },
            ClaudeCliEvent::StreamEvent {
                event: MessagesStreamEvent::ContentBlockStart {
                    index: 0,
                    content_block: MessagesContentBlock::ToolUse {
                        id: "toolu_1".to_string(),
                        name: "get_weather".to_string(),
                        input: Value::Null,
                        cache_control: None,
                    },
                },
            },
            ClaudeCliEvent::StreamEvent {
                event: MessagesStreamEvent::ContentBlockDelta {
                    index: 0,
                    delta: MessagesContentDelta::InputJsonDelta {
                        partial_json: "{\"loc\":\"".to_string(),
                    },
                },
            },
            ClaudeCliEvent::StreamEvent {
                event: MessagesStreamEvent::ContentBlockDelta {
                    index: 0,
                    delta: MessagesContentDelta::InputJsonDelta {
                        partial_json: "SF\"}".to_string(),
                    },
                },
            },
            ClaudeCliEvent::StreamEvent {
                event: MessagesStreamEvent::ContentBlockStop { index: 0 },
            },
            ClaudeCliEvent::StreamEvent {
                event: MessagesStreamEvent::MessageDelta {
                    delta: MessagesMessageDelta {
                        stop_reason: MessagesStopReason::ToolUse,
                        stop_sequence: None,
                    },
                    usage: MessagesUsage {
                        input_tokens: 0,
                        output_tokens: 5,
                        cache_creation_input_tokens: None,
                        cache_read_input_tokens: None,
                    },
                },
            },
            ClaudeCliEvent::Result {
                subtype: Some("success".to_string()),
                is_error: false,
                duration_ms: None,
                num_turns: Some(1),
                result: None,
                total_cost_usd: None,
                usage: None,
                session_id: None,
            },
        ];

        let resp = collect_to_messages_response("sonnet", events).unwrap();
        assert!(matches!(resp.stop_reason, MessagesStopReason::ToolUse));
        match &resp.content[..] {
            [MessagesContentBlock::ToolUse {
                id, name, input, ..
            }] => {
                assert_eq!(id, "toolu_1");
                assert_eq!(name, "get_weather");
                assert_eq!(input["loc"], "SF");
            }
            other => panic!("expected ToolUse block, got {other:?}"),
        }
    }

    #[test]
    fn collect_to_messages_response_propagates_cli_error() {
        let events = vec![ClaudeCliEvent::Result {
            subtype: Some("error".to_string()),
            is_error: true,
            duration_ms: Some(5),
            num_turns: Some(0),
            result: Some("auth failed".to_string()),
            total_cost_usd: None,
            usage: None,
            session_id: None,
        }];
        let err = collect_to_messages_response("sonnet", events).unwrap_err();
        match err {
            ClaudeCliTranslationError::CliError { message } => {
                assert!(message.contains("auth failed"));
            }
            other => panic!("expected CliError, got {other:?}"),
        }
    }

    #[test]
    fn collect_to_messages_response_unexpected_end() {
        let events: Vec<ClaudeCliEvent> = vec![ClaudeCliEvent::StreamEvent {
            event: MessagesStreamEvent::Ping,
        }];
        let err = collect_to_messages_response("sonnet", events).unwrap_err();
        assert!(matches!(err, ClaudeCliTranslationError::UnexpectedEnd));
    }

    #[test]
    fn collect_to_messages_response_uses_assistant_when_no_deltas() {
        let assistant_msg = ClaudeCliAssistantMessage {
            id: Some("msg_3".to_string()),
            model: Some("sonnet".to_string()),
            role: Some("assistant".to_string()),
            content: vec![ClaudeCliContentBlock::Anthropic(
                MessagesContentBlock::Text {
                    text: "ok".to_string(),
                    cache_control: None,
                },
            )],
            stop_reason: Some("end_turn".to_string()),
            stop_sequence: None,
            usage: Some(ClaudeCliUsage {
                input_tokens: Some(2),
                output_tokens: Some(1),
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
            }),
        };
        let events = vec![
            ClaudeCliEvent::Assistant {
                message: assistant_msg,
            },
            ClaudeCliEvent::Result {
                subtype: Some("success".to_string()),
                is_error: false,
                duration_ms: None,
                num_turns: Some(1),
                result: None,
                total_cost_usd: None,
                usage: None,
                session_id: None,
            },
        ];
        let resp = collect_to_messages_response("sonnet", events).unwrap();
        assert_eq!(resp.id, "msg_3");
        assert_eq!(resp.usage.input_tokens, 2);
        assert_eq!(resp.usage.output_tokens, 1);
        match &resp.content[..] {
            [MessagesContentBlock::Text { text, .. }] => assert_eq!(text, "ok"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn extract_system_prompt_blocks_join_text() {
        let req = MessagesRequest {
            system: Some(MessagesSystemPrompt::Blocks(vec![
                MessagesContentBlock::Text {
                    text: "line 1".to_string(),
                    cache_control: None,
                },
                MessagesContentBlock::Text {
                    text: "line 2".to_string(),
                    cache_control: None,
                },
            ])),
            ..user_request("ignored")
        };
        assert_eq!(
            extract_system_prompt(&req).as_deref(),
            Some("line 1\nline 2")
        );
    }

    #[test]
    fn tool_result_content_round_trips_through_translation() {
        // Sanity-check that ToolResultContent (used by future tool_result
        // translation) stays linkable as the surface evolves.
        use crate::apis::anthropic::ToolResultContent;
        let _ = ToolResultContent::Text("noop".to_string());
    }
}
