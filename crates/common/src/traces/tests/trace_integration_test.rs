//! Integration tests for OpenTelemetry tracing in router.rs
//!
//! These tests validate that the spans created for LLM requests contain
//! all expected attributes and events by checking the raw JSON payloads
//! sent to the mock OTEL collector.
//!
//! ## Test Design
//! Each test creates its own MockOtelCollector and TraceCollector:
//! 1. Start MockOtelCollector on random port
//! 2. Create TraceCollector with 500ms flush interval
//! 3. Record spans using TraceCollector
//! 4. Flush and wait (500ms + 200ms buffer = 700ms total) for spans to arrive
//! 5. Get raw JSON payloads (GET /v1/traces) and validate structure
//! 6. Test cleanup happens automatically when collectors are dropped
//!
//! ## Serial Execution
//! Tests use the `#[serial]` attribute to run sequentially because they
//! use global environment variables (OTEL_COLLECTOR_URL, OTEL_TRACING_ENABLED,
//! TRACE_FLUSH_INTERVAL_MS). This ensures test isolation without requiring
//! the `--test-threads=1` command line flag.

const FLUSH_INTERVAL_MS: u64 = 50;
const FLUSH_BUFFER_MS: u64 = 50;
const TOTAL_WAIT_MS: u64 = FLUSH_INTERVAL_MS + FLUSH_BUFFER_MS;

use crate::traces::{SpanBuilder, SpanKind, TraceCollector};
use serde_json::Value;
use serial_test::serial;
use std::sync::Arc;

use super::MockOtelCollector;

/// Helper to extract all spans from OTLP JSON payloads
fn extract_spans(payloads: &[Value]) -> Vec<&Value> {
    let mut spans = Vec::new();
    for payload in payloads {
        if let Some(resource_spans) = payload.get("resourceSpans").and_then(|v| v.as_array()) {
            for resource_span in resource_spans {
                if let Some(scope_spans) =
                    resource_span.get("scopeSpans").and_then(|v| v.as_array())
                {
                    for scope_span in scope_spans {
                        if let Some(span_list) = scope_span.get("spans").and_then(|v| v.as_array())
                        {
                            spans.extend(span_list.iter());
                        }
                    }
                }
            }
        }
    }
    spans
}

/// Helper to get string attribute value from a span
fn get_string_attr<'a>(span: &'a Value, key: &str) -> Option<&'a str> {
    span.get("attributes")
        .and_then(|attrs| attrs.as_array())
        .and_then(|attrs| {
            attrs
                .iter()
                .find(|attr| attr.get("key").and_then(|k| k.as_str()) == Some(key))
        })
        .and_then(|attr| attr.get("value"))
        .and_then(|v| v.get("stringValue"))
        .and_then(|v| v.as_str())
}

#[tokio::test]
#[serial]
async fn test_llm_span_contains_basic_attributes() {
    // Start mock OTEL collector
    let mock_collector = MockOtelCollector::start().await;

    // Create TraceCollector pointing to mock with 500ms flush intervalc
    std::env::set_var(
        "OTEL_COLLECTOR_URL",
        format!("{}/v1/traces", mock_collector.address()),
    );
    std::env::set_var("OTEL_TRACING_ENABLED", "true");
    std::env::set_var("TRACE_FLUSH_INTERVAL_MS", "500");
    let trace_collector = Arc::new(TraceCollector::new(Some(true)));

    // Create a test span simulating router.rs behavior
    let span = SpanBuilder::new("POST /v1/chat/completions >> /v1/chat/completions")
        .with_kind(SpanKind::Client)
        .with_trace_id("test-trace-123")
        .with_attribute("http.method", "POST")
        .with_attribute("http.target", "/v1/chat/completions")
        .with_attribute("http.upstream_target", "/v1/chat/completions")
        .with_attribute("llm.model", "gpt-4o")
        .with_attribute("llm.provider", "openai")
        .with_attribute("llm.is_streaming", "true")
        .with_attribute("llm.temperature", "0.7")
        .build();

    trace_collector.record_span("archgw(llm)", span);

    // Flush and wait for spans to arrive (500ms flush interval + 200ms buffer)
    trace_collector.flush().await.expect("Failed to flush");
    tokio::time::sleep(tokio::time::Duration::from_millis(TOTAL_WAIT_MS)).await;

    let payloads = mock_collector.get_traces().await;
    let spans = extract_spans(&payloads);

    assert_eq!(spans.len(), 1, "Expected exactly one span");

    let span = spans[0];
    // Validate HTTP attributes
    assert_eq!(get_string_attr(span, "http.method"), Some("POST"));
    assert_eq!(
        get_string_attr(span, "http.target"),
        Some("/v1/chat/completions")
    );

    // Validate LLM attributes
    assert_eq!(get_string_attr(span, "llm.model"), Some("gpt-4o"));
    assert_eq!(get_string_attr(span, "llm.provider"), Some("openai"));
    assert_eq!(get_string_attr(span, "llm.is_streaming"), Some("true"));
    assert_eq!(get_string_attr(span, "llm.temperature"), Some("0.7"));
}

#[tokio::test]
#[serial]
async fn test_llm_span_contains_tool_information() {
    let mock_collector = MockOtelCollector::start().await;
    std::env::set_var(
        "OTEL_COLLECTOR_URL",
        format!("{}/v1/traces", mock_collector.address()),
    );
    std::env::set_var("OTEL_TRACING_ENABLED", "true");
    std::env::set_var("TRACE_FLUSH_INTERVAL_MS", "500");
    let trace_collector = Arc::new(TraceCollector::new(Some(true)));

    let tools_formatted = "get_weather(...)\nsearch_web(...)\ncalculate(...)";

    let span = SpanBuilder::new("POST /v1/chat/completions")
        .with_trace_id("test-trace-tools")
        .with_attribute("llm.request.tools", tools_formatted)
        .with_attribute("llm.model", "gpt-4o")
        .build();

    trace_collector.record_span("archgw(llm)", span);
    trace_collector.flush().await.expect("Failed to flush");
    tokio::time::sleep(tokio::time::Duration::from_millis(TOTAL_WAIT_MS)).await;

    let payloads = mock_collector.get_traces().await;
    let spans = extract_spans(&payloads);

    assert!(!spans.is_empty(), "No spans captured");

    let span = spans[0];
    let tools = get_string_attr(span, "llm.request.tools");

    assert!(tools.is_some(), "Tools attribute missing");
    assert!(tools.unwrap().contains("get_weather(...)"));
    assert!(tools.unwrap().contains("search_web(...)"));
    assert!(tools.unwrap().contains("calculate(...)"));
    assert!(
        tools.unwrap().contains('\n'),
        "Tools should be newline-separated"
    );
}

#[tokio::test]
#[serial]
async fn test_llm_span_contains_user_message_preview() {
    let mock_collector = MockOtelCollector::start().await;
    std::env::set_var(
        "OTEL_COLLECTOR_URL",
        format!("{}/v1/traces", mock_collector.address()),
    );
    std::env::set_var("OTEL_TRACING_ENABLED", "true");
    std::env::set_var("TRACE_FLUSH_INTERVAL_MS", "500");
    let trace_collector = Arc::new(TraceCollector::new(Some(true)));

    let long_message =
        "This is a very long user message that should be truncated to 50 characters in the span";
    let preview = if long_message.len() > 50 {
        format!("{}...", &long_message[..50])
    } else {
        long_message.to_string()
    };

    let span = SpanBuilder::new("POST /v1/messages")
        .with_trace_id("test-trace-preview")
        .with_attribute("llm.request.user_message_preview", &preview)
        .build();

    trace_collector.record_span("archgw(llm)", span);
    trace_collector.flush().await.expect("Failed to flush");
    tokio::time::sleep(tokio::time::Duration::from_millis(TOTAL_WAIT_MS)).await;

    let payloads = mock_collector.get_traces().await;
    let spans = extract_spans(&payloads);
    let span = spans[0];

    let message_preview = get_string_attr(span, "llm.request.user_message_preview");

    assert!(message_preview.is_some());
    assert!(message_preview.unwrap().len() <= 53); // 50 chars + "..."
    assert!(message_preview.unwrap().contains("..."));
}

#[tokio::test]
#[serial]
async fn test_llm_span_contains_time_to_first_token() {
    let mock_collector = MockOtelCollector::start().await;
    std::env::set_var(
        "OTEL_COLLECTOR_URL",
        format!("{}/v1/traces", mock_collector.address()),
    );
    std::env::set_var("OTEL_TRACING_ENABLED", "true");
    std::env::set_var("TRACE_FLUSH_INTERVAL_MS", "500");
    let trace_collector = Arc::new(TraceCollector::new(Some(true)));

    let ttft_ms = "245"; // milliseconds as string

    let span = SpanBuilder::new("POST /v1/chat/completions")
        .with_trace_id("test-trace-ttft")
        .with_attribute("llm.is_streaming", "true")
        .with_attribute("llm.time_to_first_token_ms", ttft_ms)
        .build();

    trace_collector.record_span("archgw(llm)", span);
    trace_collector.flush().await.expect("Failed to flush");
    tokio::time::sleep(tokio::time::Duration::from_millis(TOTAL_WAIT_MS)).await;

    let payloads = mock_collector.get_traces().await;
    let spans = extract_spans(&payloads);
    let span = spans[0];

    // Check TTFT attribute
    let ttft_attr = get_string_attr(span, "llm.time_to_first_token_ms");
    assert_eq!(ttft_attr, Some("245"));
}

#[tokio::test]
#[serial]
async fn test_llm_span_contains_upstream_path() {
    let mock_collector = MockOtelCollector::start().await;
    std::env::set_var(
        "OTEL_COLLECTOR_URL",
        format!("{}/v1/traces", mock_collector.address()),
    );
    std::env::set_var("OTEL_TRACING_ENABLED", "true");
    std::env::set_var("TRACE_FLUSH_INTERVAL_MS", "500");
    let trace_collector = Arc::new(TraceCollector::new(Some(true)));

    // Test Zhipu provider with path transformation
    let span = SpanBuilder::new("POST /v1/chat/completions >> /api/paas/v4/chat/completions")
        .with_trace_id("test-trace-upstream")
        .with_attribute("http.upstream_target", "/api/paas/v4/chat/completions")
        .with_attribute("llm.provider", "zhipu")
        .with_attribute("llm.model", "glm-4")
        .build();

    trace_collector.record_span("archgw(llm)", span);
    trace_collector.flush().await.expect("Failed to flush");
    tokio::time::sleep(tokio::time::Duration::from_millis(TOTAL_WAIT_MS)).await;

    let payloads = mock_collector.get_traces().await;
    let spans = extract_spans(&payloads);
    let span = spans[0];

    // Operation name should show the transformation
    let name = span.get("name").and_then(|v| v.as_str());
    assert!(name.is_some());
    assert!(
        name.unwrap().contains(">>"),
        "Operation name should show path transformation"
    );

    // Check upstream target attribute
    let upstream = get_string_attr(span, "http.upstream_target");
    assert_eq!(upstream, Some("/api/paas/v4/chat/completions"));
}

#[tokio::test]
#[serial]
async fn test_llm_span_multiple_services() {
    let mock_collector = MockOtelCollector::start().await;
    std::env::set_var(
        "OTEL_COLLECTOR_URL",
        format!("{}/v1/traces", mock_collector.address()),
    );
    std::env::set_var("OTEL_TRACING_ENABLED", "true");
    std::env::set_var("TRACE_FLUSH_INTERVAL_MS", "500");
    let trace_collector = Arc::new(TraceCollector::new(Some(true)));

    // Create spans for different services
    let llm_span = SpanBuilder::new("LLM Request")
        .with_trace_id("test-multi")
        .with_attribute("service", "llm")
        .build();

    let routing_span = SpanBuilder::new("Routing Decision")
        .with_trace_id("test-multi")
        .with_attribute("service", "routing")
        .build();

    trace_collector.record_span("archgw(llm)", llm_span);
    trace_collector.record_span("archgw(routing)", routing_span);
    trace_collector.flush().await.expect("Failed to flush");
    tokio::time::sleep(tokio::time::Duration::from_millis(TOTAL_WAIT_MS)).await;

    let payloads = mock_collector.get_traces().await;
    let all_spans = extract_spans(&payloads);

    assert_eq!(all_spans.len(), 2, "Should have captured both spans");
}

#[tokio::test]
#[serial]
async fn test_tracing_disabled_produces_no_spans() {
    let mock_collector = MockOtelCollector::start().await;

    // Create TraceCollector with tracing DISABLED
    std::env::set_var(
        "OTEL_COLLECTOR_URL",
        format!("{}/v1/traces", mock_collector.address()),
    );
    std::env::set_var("OTEL_TRACING_ENABLED", "false");
    std::env::set_var("TRACE_FLUSH_INTERVAL_MS", "500");
    let trace_collector = Arc::new(TraceCollector::new(Some(false)));

    let span = SpanBuilder::new("Test Span")
        .with_trace_id("test-disabled")
        .build();

    trace_collector.record_span("archgw(llm)", span);
    trace_collector.flush().await.ok(); // Should be no-op when disabled
    tokio::time::sleep(tokio::time::Duration::from_millis(TOTAL_WAIT_MS)).await;

    let payloads = mock_collector.get_traces().await;
    let all_spans = extract_spans(&payloads);
    assert_eq!(
        all_spans.len(),
        0,
        "No spans should be captured when tracing is disabled"
    );
}
