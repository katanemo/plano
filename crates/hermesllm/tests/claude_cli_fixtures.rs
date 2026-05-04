//! End-to-end fixture tests for `apis::claude_cli`. Each NDJSON file under
//! `tests/fixtures/claude_cli/` represents one canned subprocess output. We
//! parse it line-by-line and feed it through the same translation entry points
//! the brightstaff bridge uses at runtime.

use std::fs;
use std::path::PathBuf;

use hermesllm::apis::anthropic::{
    MessagesContentBlock, MessagesContentDelta, MessagesStopReason, MessagesStreamEvent,
};
use hermesllm::apis::claude_cli::{
    cli_event_to_messages_stream_event, collect_to_messages_response, parse_ndjson_line,
    ClaudeCliEvent, ClaudeCliTranslationError,
};

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("claude_cli")
        .join(name)
}

fn load_events(name: &str) -> Vec<ClaudeCliEvent> {
    let body = fs::read_to_string(fixture_path(name))
        .unwrap_or_else(|e| panic!("read fixture {name}: {e}"));
    body.lines()
        .filter_map(|line| parse_ndjson_line(line).map(|r| r.unwrap_or_else(|e| panic!("{e}"))))
        .collect()
}

#[test]
fn text_response_aggregates_into_messages_response() {
    let events = load_events("text_response.ndjson");
    let resp = collect_to_messages_response("claude-cli/sonnet", events.clone()).unwrap();
    assert_eq!(resp.id, "msg_01ABC");
    assert_eq!(resp.model, "claude-sonnet-4-6");
    assert_eq!(resp.usage.input_tokens, 12);
    assert_eq!(resp.usage.output_tokens, 4);
    assert!(matches!(resp.stop_reason, MessagesStopReason::EndTurn));
    match &resp.content[..] {
        [MessagesContentBlock::Text { text, .. }] => assert_eq!(text, "Hello, world!"),
        other => panic!("expected single Text, got {other:?}"),
    }

    // Verify the streaming projection emits exactly the events the Anthropic
    // SSE wire protocol expects, in order.
    let stream: Vec<MessagesStreamEvent> = events
        .iter()
        .filter_map(cli_event_to_messages_stream_event)
        .collect();
    assert!(matches!(
        stream[0],
        MessagesStreamEvent::MessageStart { .. }
    ));
    let final_event = stream.last().unwrap();
    assert!(matches!(final_event, MessagesStreamEvent::MessageStop));
    let text_deltas: String = stream
        .iter()
        .filter_map(|ev| match ev {
            MessagesStreamEvent::ContentBlockDelta {
                delta: MessagesContentDelta::TextDelta { text },
                ..
            } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text_deltas, "Hello, world!");
}

#[test]
fn tool_use_response_assembles_partial_json() {
    let events = load_events("tool_use_response.ndjson");
    let resp = collect_to_messages_response("sonnet", events).unwrap();
    assert!(matches!(resp.stop_reason, MessagesStopReason::ToolUse));
    match &resp.content[..] {
        [MessagesContentBlock::ToolUse {
            id, name, input, ..
        }] => {
            assert_eq!(id, "toolu_W");
            assert_eq!(name, "get_weather");
            assert_eq!(input["city"], "Seattle");
        }
        other => panic!("expected single ToolUse block, got {other:?}"),
    }
}

#[test]
fn error_response_returns_cli_error() {
    let events = load_events("error_response.ndjson");
    let err = collect_to_messages_response("sonnet", events).unwrap_err();
    match err {
        ClaudeCliTranslationError::CliError { message } => {
            assert!(
                message.contains("529"),
                "expected 529 in error message, got: {message}"
            );
        }
        other => panic!("expected CliError, got {other:?}"),
    }
}

#[test]
fn retry_then_success_is_treated_as_success() {
    let events = load_events("retry_then_success.ndjson");
    let resp = collect_to_messages_response("sonnet", events).unwrap();
    assert!(matches!(resp.stop_reason, MessagesStopReason::EndTurn));
    match &resp.content[..] {
        [MessagesContentBlock::Text { text, .. }] => assert_eq!(text, "ok"),
        other => panic!("expected Text block, got {other:?}"),
    }
}
