//! PostHog Span Exporter
//!
//! A custom [`SpanExporter`] that translates Plano's LLM spans into PostHog
//! [`$ai_generation`](https://posthog.com/docs/ai-observability/generations)
//! events and POSTs them to PostHog's capture API (`{url}/batch/`).
//!
//! This makes PostHog a first-class, provider-agnostic export target: a user
//! only points `tracing.exporters` at their PostHog URL + project token and
//! every LLM call is captured — mirroring LiteLLM's `posthog` callback.
//!
//! # Behaviour
//!
//! - Receives every span in the provider (like all batch exporters do) and
//!   keeps only LLM generation spans, identified by the presence of the
//!   [`llm::MODEL_NAME`] (`llm.model`) attribute.
//! - Maps span attributes onto `$ai_*` PostHog properties (model, provider,
//!   latency, tokens, http status, ...).
//! - `distinct_id` is read from the [`plano::DISTINCT_ID`] span attribute (set
//!   by the LLM handler from the configured `distinct_id_header`). When absent
//!   the event is captured anonymously (`$process_person_profile = false`).
//! - Network failures are logged and dropped — telemetry export never blocks or
//!   fails request processing.

use std::time::Duration;

use opentelemetry::{Array, Value};
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SpanData, SpanExporter};
use opentelemetry_sdk::Resource;
use serde_json::{json, Map, Value as JsonValue};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use super::{http, llm, plano};

/// PostHog event name for an individual LLM call.
const AI_GENERATION_EVENT: &str = "$ai_generation";

/// PostHog capture path appended to the configured host.
const CAPTURE_PATH: &str = "batch/";

/// A [`SpanExporter`] that ships LLM spans to PostHog as `$ai_generation` events.
pub struct PostHogExporter {
    client: reqwest::Client,
    /// Fully-qualified capture endpoint, e.g. `https://us.i.posthog.com/batch/`.
    endpoint: String,
    /// PostHog project API key (token).
    api_key: String,
    /// Whether to attach the truncated user message preview as `$ai_input`.
    capture_messages: bool,
}

impl std::fmt::Debug for PostHogExporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostHogExporter")
            .field("endpoint", &self.endpoint)
            .field("capture_messages", &self.capture_messages)
            .finish()
    }
}

impl PostHogExporter {
    /// Create a new PostHog exporter.
    ///
    /// # Arguments
    /// * `url` – PostHog host (e.g. `https://us.i.posthog.com`). The `/batch/`
    ///   capture path is appended automatically.
    /// * `api_key` – PostHog project API key (token).
    /// * `capture_messages` – when true, send the user message preview as
    ///   `$ai_input`.
    pub fn new(url: &str, api_key: &str, capture_messages: bool) -> Self {
        let endpoint = format!("{}/{}", url.trim_end_matches('/'), CAPTURE_PATH);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        Self {
            client,
            endpoint,
            api_key: api_key.to_string(),
            capture_messages,
        }
    }

    /// Build the PostHog `batch` payload from a batch of spans, keeping only LLM
    /// generation spans. Returns `None` when no LLM spans are present.
    fn build_payload(&self, batch: &[SpanData]) -> Option<JsonValue> {
        let events: Vec<JsonValue> = batch
            .iter()
            .filter_map(|span| self.build_generation_event(span))
            .collect();

        if events.is_empty() {
            return None;
        }

        Some(json!({
            "api_key": self.api_key,
            "batch": events,
        }))
    }

    /// Translate a single span into a PostHog `$ai_generation` event, or `None`
    /// if the span is not an LLM generation span.
    fn build_generation_event(&self, span: &SpanData) -> Option<JsonValue> {
        // Only LLM generation spans carry `llm.model`.
        let model = find_attr(span, llm::MODEL_NAME)?;

        let mut props = Map::new();
        props.insert("$ai_model".to_string(), otel_value_to_json(model));
        props.insert(
            "$ai_trace_id".to_string(),
            json!(span.span_context.trace_id().to_string()),
        );
        if span.parent_span_id != opentelemetry::trace::SpanId::INVALID {
            props.insert(
                "$ai_parent_id".to_string(),
                json!(span.parent_span_id.to_string()),
            );
        }

        if let Some(provider) = find_attr(span, llm::PROVIDER) {
            props.insert("$ai_provider".to_string(), otel_value_to_json(provider));
        }

        // Latency / TTFT are stored in milliseconds; PostHog wants seconds.
        if let Some(ms) = find_i64(span, llm::DURATION_MS) {
            props.insert("$ai_latency".to_string(), json!(ms as f64 / 1000.0));
        }
        if let Some(ms) = find_i64(span, llm::TIME_TO_FIRST_TOKEN_MS) {
            props.insert(
                "$ai_time_to_first_token".to_string(),
                json!(ms as f64 / 1000.0),
            );
            props.insert("$ai_stream".to_string(), json!(true));
        }

        if let Some(tokens) = find_i64(span, llm::PROMPT_TOKENS) {
            props.insert("$ai_input_tokens".to_string(), json!(tokens));
        }
        if let Some(tokens) = find_i64(span, llm::COMPLETION_TOKENS) {
            props.insert("$ai_output_tokens".to_string(), json!(tokens));
        }

        if let Some(status) = find_i64(span, http::STATUS_CODE) {
            props.insert("$ai_http_status".to_string(), json!(status));
            if status >= 400 {
                props.insert("$ai_is_error".to_string(), json!(true));
            }
        }

        if self.capture_messages {
            if let Some(preview) = find_attr(span, llm::USER_MESSAGE_PREVIEW) {
                props.insert(
                    "$ai_input".to_string(),
                    json!([{ "role": "user", "content": value_to_string(preview) }]),
                );
            }
        }

        // distinct_id: identified when the configured header was present,
        // otherwise anonymous (do not create/update a person profile).
        match find_attr(span, plano::DISTINCT_ID) {
            Some(id) => {
                props.insert("distinct_id".to_string(), otel_value_to_json(id));
            }
            None => {
                props.insert(
                    "distinct_id".to_string(),
                    json!(span.span_context.trace_id().to_string()),
                );
                props.insert("$process_person_profile".to_string(), json!(false));
            }
        }

        // Pass through any other non-reserved attributes (custom span attributes
        // such as static tags or header-derived tenant ids) as plain properties.
        for kv in span.attributes.iter() {
            let key = kv.key.as_str();
            if is_reserved_attr(key) {
                continue;
            }
            props
                .entry(key.to_string())
                .or_insert_with(|| otel_value_to_json(&kv.value));
        }

        let mut event = Map::new();
        event.insert("event".to_string(), json!(AI_GENERATION_EVENT));
        event.insert("properties".to_string(), JsonValue::Object(props));
        if let Ok(ts) = OffsetDateTime::from(span.end_time).format(&Rfc3339) {
            event.insert("timestamp".to_string(), json!(ts));
        }

        Some(JsonValue::Object(event))
    }
}

impl SpanExporter for PostHogExporter {
    fn export(
        &self,
        batch: Vec<SpanData>,
    ) -> impl std::future::Future<Output = OTelSdkResult> + Send {
        let payload = self.build_payload(&batch);
        let client = self.client.clone();
        let endpoint = self.endpoint.clone();
        async move {
            let Some(payload) = payload else {
                return Ok(());
            };
            match client.post(&endpoint).json(&payload).send().await {
                Ok(resp) if resp.status().is_success() => {}
                Ok(resp) => {
                    tracing::warn!(
                        status = %resp.status(),
                        endpoint = %endpoint,
                        "PostHog exporter: non-success response"
                    );
                }
                Err(e) => {
                    tracing::warn!(error = ?e, endpoint = %endpoint, "PostHog exporter: request failed");
                }
            }
            Ok(())
        }
    }

    fn shutdown_with_timeout(&mut self, _timeout: Duration) -> OTelSdkResult {
        Ok(())
    }

    fn set_resource(&mut self, _resource: &Resource) {}
}

/// Span attributes that are mapped to dedicated `$ai_*` properties (or are
/// internal plumbing) and should not be duplicated as raw properties.
fn is_reserved_attr(key: &str) -> bool {
    matches!(
        key,
        k if k == llm::MODEL_NAME
            || k == llm::PROVIDER
            || k == llm::DURATION_MS
            || k == llm::TIME_TO_FIRST_TOKEN_MS
            || k == llm::PROMPT_TOKENS
            || k == llm::COMPLETION_TOKENS
            || k == llm::USER_MESSAGE_PREVIEW
            || k == http::STATUS_CODE
            || k == plano::DISTINCT_ID
            || k == super::SERVICE_NAME_OVERRIDE_KEY
    )
}

fn find_attr<'a>(span: &'a SpanData, key: &str) -> Option<&'a Value> {
    span.attributes
        .iter()
        .find(|kv| kv.key.as_str() == key)
        .map(|kv| &kv.value)
}

fn find_i64(span: &SpanData, key: &str) -> Option<i64> {
    match find_attr(span, key)? {
        Value::I64(i) => Some(*i),
        _ => None,
    }
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.as_str().to_string(),
        other => other.to_string(),
    }
}

fn otel_value_to_json(value: &Value) -> JsonValue {
    match value {
        Value::Bool(b) => json!(b),
        Value::I64(i) => json!(i),
        Value::F64(f) => json!(f),
        Value::String(s) => json!(s.as_str()),
        Value::Array(arr) => match arr {
            Array::Bool(v) => json!(v),
            Array::I64(v) => json!(v),
            Array::F64(v) => json!(v),
            Array::String(v) => json!(v.iter().map(|s| s.as_str()).collect::<Vec<_>>()),
            _ => JsonValue::Null,
        },
        _ => json!(value.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::trace::{
        SpanContext, SpanId, SpanKind, Status, TraceFlags, TraceId, TraceState,
    };
    use opentelemetry::KeyValue;
    use opentelemetry_sdk::trace::{SpanData, SpanEvents, SpanLinks};
    use std::borrow::Cow;
    use std::time::SystemTime;

    fn span_with_attrs(attrs: Vec<KeyValue>) -> SpanData {
        SpanData {
            span_context: SpanContext::new(
                TraceId::from_bytes([
                    0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x12, 0x34, 0x56, 0x78, 0x9a,
                    0xbc, 0xde, 0xf0,
                ]),
                SpanId::from_bytes([0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]),
                TraceFlags::SAMPLED,
                false,
                TraceState::default(),
            ),
            parent_span_id: SpanId::INVALID,
            parent_span_is_remote: false,
            span_kind: SpanKind::Client,
            name: Cow::Borrowed("llm"),
            start_time: SystemTime::UNIX_EPOCH,
            end_time: SystemTime::UNIX_EPOCH,
            attributes: attrs,
            dropped_attributes_count: 0,
            events: SpanEvents::default(),
            links: SpanLinks::default(),
            status: Status::Unset,
            instrumentation_scope: Default::default(),
        }
    }

    fn props(event: &JsonValue) -> &Map<String, JsonValue> {
        event["properties"].as_object().unwrap()
    }

    #[test]
    fn non_llm_span_is_skipped() {
        let exporter = PostHogExporter::new("https://us.i.posthog.com", "phc_x", false);
        let span = span_with_attrs(vec![KeyValue::new("routing.strategy", "least-latency")]);
        assert!(exporter.build_generation_event(&span).is_none());
    }

    #[test]
    fn maps_llm_attributes_to_ai_properties() {
        let exporter = PostHogExporter::new("https://us.i.posthog.com/", "phc_x", false);
        let span = span_with_attrs(vec![
            KeyValue::new(llm::MODEL_NAME, "gpt-5-mini"),
            KeyValue::new(llm::PROVIDER, "openai"),
            KeyValue::new(llm::DURATION_MS, 1500_i64),
            KeyValue::new(llm::TIME_TO_FIRST_TOKEN_MS, 250_i64),
            KeyValue::new(llm::PROMPT_TOKENS, 10_i64),
            KeyValue::new(llm::COMPLETION_TOKENS, 20_i64),
            KeyValue::new(http::STATUS_CODE, 200_i64),
            KeyValue::new("tenant.id", "acme"),
        ]);

        let event = exporter.build_generation_event(&span).unwrap();
        assert_eq!(event["event"], json!("$ai_generation"));
        let p = props(&event);
        assert_eq!(p["$ai_model"], json!("gpt-5-mini"));
        assert_eq!(p["$ai_provider"], json!("openai"));
        assert_eq!(p["$ai_latency"], json!(1.5));
        assert_eq!(p["$ai_time_to_first_token"], json!(0.25));
        assert_eq!(p["$ai_stream"], json!(true));
        assert_eq!(p["$ai_input_tokens"], json!(10));
        assert_eq!(p["$ai_output_tokens"], json!(20));
        assert_eq!(p["$ai_http_status"], json!(200));
        // Anonymous (no distinct id header captured).
        assert_eq!(p["$process_person_profile"], json!(false));
        // Custom passthrough attribute preserved.
        assert_eq!(p["tenant.id"], json!("acme"));
        // No $ai_input unless capture_messages is enabled.
        assert!(!p.contains_key("$ai_input"));
    }

    #[test]
    fn uses_distinct_id_and_flags_errors() {
        let exporter = PostHogExporter::new("https://us.i.posthog.com", "phc_x", true);
        let span = span_with_attrs(vec![
            KeyValue::new(llm::MODEL_NAME, "gpt-5-mini"),
            KeyValue::new(plano::DISTINCT_ID, "user_123"),
            KeyValue::new(llm::USER_MESSAGE_PREVIEW, "hello"),
            KeyValue::new(http::STATUS_CODE, 500_i64),
        ]);

        let event = exporter.build_generation_event(&span).unwrap();
        let p = props(&event);
        assert_eq!(p["distinct_id"], json!("user_123"));
        assert!(!p.contains_key("$process_person_profile"));
        assert_eq!(p["$ai_is_error"], json!(true));
        assert_eq!(
            p["$ai_input"],
            json!([{ "role": "user", "content": "hello" }])
        );
    }

    #[test]
    fn payload_wraps_events_with_api_key() {
        let exporter = PostHogExporter::new("https://us.i.posthog.com", "phc_secret", false);
        let span = span_with_attrs(vec![KeyValue::new(llm::MODEL_NAME, "gpt-5-mini")]);
        let payload = exporter.build_payload(&[span]).unwrap();
        assert_eq!(payload["api_key"], json!("phc_secret"));
        assert_eq!(payload["batch"].as_array().unwrap().len(), 1);
    }
}
