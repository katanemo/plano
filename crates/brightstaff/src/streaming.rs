use bytes::Bytes;
use common::configuration::ResolvedFilterChain;
use http_body_util::combinators::BoxBody;
use http_body_util::StreamBody;
use hyper::body::Frame;
use hyper::header::HeaderMap;
use opentelemetry::trace::TraceContextExt;
use opentelemetry::KeyValue;
use std::time::Instant;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
use tracing::{debug, info, warn, Instrument};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::handlers::agents::pipeline::{PipelineError, PipelineProcessor};

const STREAM_BUFFER_SIZE: usize = 16;
/// Cap on accumulated response bytes kept for usage extraction.
/// Most chat responses are well under this; pathological ones are dropped without
/// affecting pass-through streaming to the client.
const USAGE_BUFFER_MAX: usize = 2 * 1024 * 1024;
use crate::signals::{InteractionQuality, SignalAnalyzer, TextBasedSignalAnalyzer, FLAG_MARKER};
use crate::tracing::{llm, set_service_name, signals as signal_constants};
use hermesllm::apis::openai::Message;

/// Parsed usage + resolved-model details from a provider response.
#[derive(Debug, Default, Clone)]
struct ExtractedUsage {
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
    total_tokens: Option<i64>,
    cached_input_tokens: Option<i64>,
    cache_creation_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
    /// The model the upstream actually used. For router aliases (e.g.
    /// `router:software-engineering`), this differs from the request model.
    resolved_model: Option<String>,
}

impl ExtractedUsage {
    fn is_empty(&self) -> bool {
        self.prompt_tokens.is_none()
            && self.completion_tokens.is_none()
            && self.total_tokens.is_none()
            && self.resolved_model.is_none()
    }

    fn from_json(value: &serde_json::Value) -> Self {
        let mut out = Self::default();
        if let Some(model) = value.get("model").and_then(|v| v.as_str()) {
            if !model.is_empty() {
                out.resolved_model = Some(model.to_string());
            }
        }
        if let Some(u) = value.get("usage") {
            // OpenAI-shape usage
            out.prompt_tokens = u.get("prompt_tokens").and_then(|v| v.as_i64());
            out.completion_tokens = u.get("completion_tokens").and_then(|v| v.as_i64());
            out.total_tokens = u.get("total_tokens").and_then(|v| v.as_i64());
            out.cached_input_tokens = u
                .get("prompt_tokens_details")
                .and_then(|d| d.get("cached_tokens"))
                .and_then(|v| v.as_i64());
            out.reasoning_tokens = u
                .get("completion_tokens_details")
                .and_then(|d| d.get("reasoning_tokens"))
                .and_then(|v| v.as_i64());

            // Anthropic-shape fallbacks
            if out.prompt_tokens.is_none() {
                out.prompt_tokens = u.get("input_tokens").and_then(|v| v.as_i64());
            }
            if out.completion_tokens.is_none() {
                out.completion_tokens = u.get("output_tokens").and_then(|v| v.as_i64());
            }
            if out.total_tokens.is_none() {
                if let (Some(p), Some(c)) = (out.prompt_tokens, out.completion_tokens) {
                    out.total_tokens = Some(p + c);
                }
            }
            if out.cached_input_tokens.is_none() {
                out.cached_input_tokens = u.get("cache_read_input_tokens").and_then(|v| v.as_i64());
            }
            if out.cached_input_tokens.is_none() {
                out.cached_input_tokens =
                    u.get("cached_content_token_count").and_then(|v| v.as_i64());
            }
            out.cache_creation_tokens = u
                .get("cache_creation_input_tokens")
                .and_then(|v| v.as_i64());
            if out.reasoning_tokens.is_none() {
                out.reasoning_tokens = u.get("thoughts_token_count").and_then(|v| v.as_i64());
            }
        }
        out
    }
}

/// Try to pull usage out of an accumulated response body.
/// Handles both a single JSON object (non-streaming) and SSE streams where the
/// final `data: {...}` event carries the `usage` field.
///
/// Resolved model is captured independently from usage: many providers (DO
/// Inference Cloud, OpenAI in some modes) emit `model` on every SSE chunk but
/// `usage` only on the terminal one — and that terminal chunk may omit the
/// model field entirely. Scanning both fields separately means we still
/// surface the real upstream model even when usage extraction fails.
fn extract_usage_from_bytes(buf: &[u8]) -> ExtractedUsage {
    if buf.is_empty() {
        return ExtractedUsage::default();
    }

    // Fast path: full-body JSON (non-streaming).
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(buf) {
        let u = ExtractedUsage::from_json(&value);
        if !u.is_empty() {
            return u;
        }
    }

    let text = match std::str::from_utf8(buf) {
        Ok(t) => t,
        Err(_) => return ExtractedUsage::default(),
    };

    let mut out = ExtractedUsage::default();
    let mut found_usage = false;

    for line in text.lines().rev() {
        let trimmed = line.trim_start();
        let payload = match trimmed.strip_prefix("data:") {
            Some(p) => p.trim_start(),
            None => continue,
        };
        if payload == "[DONE]" || payload.is_empty() {
            continue;
        }

        let has_model_field = out.resolved_model.is_none() && payload.contains("\"model\"");
        let has_usage_field = !found_usage && payload.contains("\"usage\"");
        if !has_model_field && !has_usage_field {
            continue;
        }

        let value = match serde_json::from_str::<serde_json::Value>(payload) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if has_usage_field {
            let u = ExtractedUsage::from_json(&value);
            if u.prompt_tokens.is_some()
                || u.completion_tokens.is_some()
                || u.total_tokens.is_some()
            {
                let prior_model = out.resolved_model.take();
                out = u;
                if out.resolved_model.is_none() {
                    out.resolved_model = prior_model;
                }
                found_usage = true;
                if out.resolved_model.is_some() {
                    break;
                }
                continue;
            }
        }

        if out.resolved_model.is_none() {
            if let Some(model) = value.get("model").and_then(|v| v.as_str()) {
                if !model.is_empty() {
                    out.resolved_model = Some(model.to_string());
                    if found_usage {
                        break;
                    }
                }
            }
        }
    }

    out
}

/// Trait for processing streaming chunks
/// Implementors can inject custom logic during streaming (e.g., hallucination detection, logging)
pub trait StreamProcessor: Send + 'static {
    /// Process an incoming chunk of bytes
    fn process_chunk(&mut self, chunk: Bytes) -> Result<Option<Bytes>, String>;

    /// Called when the first bytes are received (for time-to-first-token tracking)
    fn on_first_bytes(&mut self) {}

    /// Called when streaming completes successfully
    fn on_complete(&mut self) {}

    /// Called when streaming encounters an error
    fn on_error(&mut self, _error: &str) {}
}

impl StreamProcessor for Box<dyn StreamProcessor> {
    fn process_chunk(&mut self, chunk: Bytes) -> Result<Option<Bytes>, String> {
        (**self).process_chunk(chunk)
    }
    fn on_first_bytes(&mut self) {
        (**self).on_first_bytes()
    }
    fn on_complete(&mut self) {
        (**self).on_complete()
    }
    fn on_error(&mut self, error: &str) {
        (**self).on_error(error)
    }
}

/// A processor that tracks streaming metrics
pub struct ObservableStreamProcessor {
    service_name: String,
    operation_name: String,
    total_bytes: usize,
    chunk_count: usize,
    start_time: Instant,
    time_to_first_token: Option<u128>,
    messages: Option<Vec<Message>>,
    /// Accumulated response bytes used only for best-effort usage extraction
    /// on `on_complete`. Capped at `USAGE_BUFFER_MAX`; excess chunks are dropped
    /// from the buffer (they still pass through to the client).
    response_buffer: Vec<u8>,
}

impl ObservableStreamProcessor {
    /// Create a new passthrough processor
    ///
    /// # Arguments
    /// * `service_name` - The service name for this span (e.g., "plano(llm)")
    ///   This will be set as the `service.name.override` attribute on the current span,
    ///   allowing the ServiceNameOverrideExporter to route spans to different services.
    /// * `operation_name` - The current span operation name (e.g., "POST /v1/chat/completions gpt-4")
    ///   Used to append the flag marker when concerning signals are detected.
    /// * `start_time` - When the request started (for duration calculation)
    /// * `messages` - Optional conversation messages for signal analysis
    pub fn new(
        service_name: impl Into<String>,
        operation_name: impl Into<String>,
        start_time: Instant,
        messages: Option<Vec<Message>>,
    ) -> Self {
        let service_name = service_name.into();

        // Set the service name override on the current span for OpenTelemetry export
        // This allows the ServiceNameOverrideExporter to route this span to the correct service
        set_service_name(&service_name);

        Self {
            service_name,
            operation_name: operation_name.into(),
            total_bytes: 0,
            chunk_count: 0,
            start_time,
            time_to_first_token: None,
            messages,
            response_buffer: Vec::new(),
        }
    }
}

impl StreamProcessor for ObservableStreamProcessor {
    fn process_chunk(&mut self, chunk: Bytes) -> Result<Option<Bytes>, String> {
        self.total_bytes += chunk.len();
        self.chunk_count += 1;
        // Accumulate for best-effort usage extraction; drop further chunks once
        // the cap is reached so we don't retain huge response bodies in memory.
        if self.response_buffer.len() < USAGE_BUFFER_MAX {
            let remaining = USAGE_BUFFER_MAX - self.response_buffer.len();
            let take = chunk.len().min(remaining);
            self.response_buffer.extend_from_slice(&chunk[..take]);
        }
        Ok(Some(chunk))
    }

    fn on_first_bytes(&mut self) {
        // Record time to first token (only for streaming)
        if self.time_to_first_token.is_none() {
            self.time_to_first_token = Some(self.start_time.elapsed().as_millis());
        }
    }

    fn on_complete(&mut self) {
        // Record time-to-first-token as an OTel span attribute + event (streaming only)
        if let Some(ttft) = self.time_to_first_token {
            let span = tracing::Span::current();
            let otel_context = span.context();
            let otel_span = otel_context.span();
            otel_span.set_attribute(KeyValue::new(llm::TIME_TO_FIRST_TOKEN_MS, ttft as i64));
            otel_span.add_event(
                llm::TIME_TO_FIRST_TOKEN_MS,
                vec![KeyValue::new(llm::TIME_TO_FIRST_TOKEN_MS, ttft as i64)],
            );
        }

        // Record total duration on the span for the observability console.
        let duration_ms = self.start_time.elapsed().as_millis() as i64;
        {
            let span = tracing::Span::current();
            let otel_context = span.context();
            let otel_span = otel_context.span();
            otel_span.set_attribute(KeyValue::new(llm::DURATION_MS, duration_ms));
            otel_span.set_attribute(KeyValue::new(llm::RESPONSE_BYTES, self.total_bytes as i64));
        }

        // Best-effort usage extraction + emission (works for both streaming
        // SSE and non-streaming JSON responses that include a `usage` object).
        let usage = extract_usage_from_bytes(&self.response_buffer);
        if !usage.is_empty() {
            let span = tracing::Span::current();
            let otel_context = span.context();
            let otel_span = otel_context.span();
            if let Some(v) = usage.prompt_tokens {
                otel_span.set_attribute(KeyValue::new(llm::PROMPT_TOKENS, v));
            }
            if let Some(v) = usage.completion_tokens {
                otel_span.set_attribute(KeyValue::new(llm::COMPLETION_TOKENS, v));
            }
            if let Some(v) = usage.total_tokens {
                otel_span.set_attribute(KeyValue::new(llm::TOTAL_TOKENS, v));
            }
            if let Some(v) = usage.cached_input_tokens {
                otel_span.set_attribute(KeyValue::new(llm::CACHED_INPUT_TOKENS, v));
            }
            if let Some(v) = usage.cache_creation_tokens {
                otel_span.set_attribute(KeyValue::new(llm::CACHE_CREATION_TOKENS, v));
            }
            if let Some(v) = usage.reasoning_tokens {
                otel_span.set_attribute(KeyValue::new(llm::REASONING_TOKENS, v));
            }
            // Override `llm.model` with the model the upstream actually ran
            // (e.g. `openai-gpt-5.4` resolved from `router:software-engineering`).
            // Cost lookup keys off the real model, not the alias.
            if let Some(resolved) = usage.resolved_model.clone() {
                otel_span.set_attribute(KeyValue::new(llm::MODEL_NAME, resolved));
            }
        }
        // Release the buffered bytes early; nothing downstream needs them.
        self.response_buffer.clear();
        self.response_buffer.shrink_to_fit();

        // Analyze signals if messages are available and record as span attributes
        if let Some(ref messages) = self.messages {
            let analyzer: Box<dyn SignalAnalyzer> = Box::new(TextBasedSignalAnalyzer::new());
            let report = analyzer.analyze(messages);

            // Get the current OTel span to set signal attributes
            let span = tracing::Span::current();
            let otel_context = span.context();
            let otel_span = otel_context.span();

            // Add overall quality
            otel_span.set_attribute(KeyValue::new(
                signal_constants::QUALITY,
                format!("{:?}", report.overall_quality),
            ));

            // Add repair/follow-up metrics if concerning
            if report.follow_up.is_concerning || report.follow_up.repair_count > 0 {
                otel_span.set_attribute(KeyValue::new(
                    signal_constants::REPAIR_COUNT,
                    report.follow_up.repair_count as i64,
                ));
                otel_span.set_attribute(KeyValue::new(
                    signal_constants::REPAIR_RATIO,
                    format!("{:.3}", report.follow_up.repair_ratio),
                ));
            }

            // Add frustration metrics
            if report.frustration.has_frustration {
                otel_span.set_attribute(KeyValue::new(
                    signal_constants::FRUSTRATION_COUNT,
                    report.frustration.frustration_count as i64,
                ));
                otel_span.set_attribute(KeyValue::new(
                    signal_constants::FRUSTRATION_SEVERITY,
                    report.frustration.severity as i64,
                ));
            }

            // Add repetition metrics
            if report.repetition.has_looping {
                otel_span.set_attribute(KeyValue::new(
                    signal_constants::REPETITION_COUNT,
                    report.repetition.repetition_count as i64,
                ));
            }

            // Add escalation metrics
            if report.escalation.escalation_requested {
                otel_span
                    .set_attribute(KeyValue::new(signal_constants::ESCALATION_REQUESTED, true));
            }

            // Add positive feedback metrics
            if report.positive_feedback.has_positive_feedback {
                otel_span.set_attribute(KeyValue::new(
                    signal_constants::POSITIVE_FEEDBACK_COUNT,
                    report.positive_feedback.positive_count as i64,
                ));
            }

            // Flag the span name if any concerning signal is detected
            let should_flag = report.frustration.has_frustration
                || report.repetition.has_looping
                || report.escalation.escalation_requested
                || matches!(
                    report.overall_quality,
                    InteractionQuality::Poor | InteractionQuality::Severe
                );

            if should_flag {
                otel_span.update_name(format!("{} {}", self.operation_name, FLAG_MARKER));
            }
        }

        info!(
            service = %self.service_name,
            total_bytes = self.total_bytes,
            chunk_count = self.chunk_count,
            duration_ms = self.start_time.elapsed().as_millis(),
            time_to_first_token_ms = ?self.time_to_first_token,
            "streaming completed"
        );
    }

    fn on_error(&mut self, error_msg: &str) {
        warn!(
            service = %self.service_name,
            error = error_msg,
            duration_ms = self.start_time.elapsed().as_millis(),
            "stream error"
        );
    }
}

/// Result of creating a streaming response
pub struct StreamingResponse {
    pub body: BoxBody<Bytes, hyper::Error>,
    pub processor_handle: tokio::task::JoinHandle<()>,
}

pub fn create_streaming_response<S, P>(mut byte_stream: S, mut processor: P) -> StreamingResponse
where
    S: StreamExt<Item = Result<Bytes, reqwest::Error>> + Send + Unpin + 'static,
    P: StreamProcessor,
{
    let (tx, rx) = mpsc::channel::<Bytes>(STREAM_BUFFER_SIZE);

    // Capture the current span so the spawned task inherits the request context
    let current_span = tracing::Span::current();

    // Spawn a task to process and forward chunks
    let processor_handle = tokio::spawn(
        async move {
            let mut is_first_chunk = true;

            while let Some(item) = byte_stream.next().await {
                let chunk = match item {
                    Ok(chunk) => chunk,
                    Err(err) => {
                        let err_msg = format!("Error receiving chunk: {:?}", err);
                        warn!(error = %err_msg, "stream error");
                        processor.on_error(&err_msg);
                        break;
                    }
                };

                // Call on_first_bytes for the first chunk
                if is_first_chunk {
                    processor.on_first_bytes();
                    is_first_chunk = false;
                }

                // Process the chunk
                match processor.process_chunk(chunk) {
                    Ok(Some(processed_chunk)) => {
                        if tx.send(processed_chunk).await.is_err() {
                            warn!("receiver dropped");
                            break;
                        }
                    }
                    Ok(None) => {
                        // Skip this chunk
                        continue;
                    }
                    Err(err) => {
                        warn!("processor error: {}", err);
                        processor.on_error(&err);
                        break;
                    }
                }
            }

            processor.on_complete();
        }
        .instrument(current_span),
    );

    // Convert channel receiver to HTTP stream
    let stream = ReceiverStream::new(rx).map(|chunk| Ok::<_, hyper::Error>(Frame::data(chunk)));
    let stream_body = BoxBody::new(StreamBody::new(stream));

    StreamingResponse {
        body: stream_body,
        processor_handle,
    }
}

/// Creates a streaming response that processes each raw chunk through output filters.
/// Filters receive the raw LLM response bytes and request path (any API shape; not limited to
/// chat completions). On filter error mid-stream the original chunk is passed through (headers already sent).
pub fn create_streaming_response_with_output_filter<S, P>(
    mut byte_stream: S,
    mut inner_processor: P,
    output_chain: ResolvedFilterChain,
    request_headers: HeaderMap,
    request_path: String,
) -> StreamingResponse
where
    S: StreamExt<Item = Result<Bytes, reqwest::Error>> + Send + Unpin + 'static,
    P: StreamProcessor,
{
    let (tx, rx) = mpsc::channel::<Bytes>(STREAM_BUFFER_SIZE);
    let current_span = tracing::Span::current();

    let processor_handle = tokio::spawn(
        async move {
            let mut is_first_chunk = true;
            let mut pipeline_processor = PipelineProcessor::default();
            let chain = output_chain.to_agent_filter_chain("output_filter");

            while let Some(item) = byte_stream.next().await {
                let chunk = match item {
                    Ok(chunk) => chunk,
                    Err(err) => {
                        let err_msg = format!("Error receiving chunk: {:?}", err);
                        warn!(error = %err_msg, "stream error");
                        inner_processor.on_error(&err_msg);
                        break;
                    }
                };

                if is_first_chunk {
                    inner_processor.on_first_bytes();
                    is_first_chunk = false;
                }

                // Pass raw chunk bytes through the output filter chain
                let processed_chunk = match pipeline_processor
                    .process_raw_filter_chain(
                        &chunk,
                        &chain,
                        &output_chain.agents,
                        &request_headers,
                        &request_path,
                    )
                    .await
                {
                    Ok(filtered) => filtered,
                    Err(PipelineError::ClientError {
                        agent,
                        status,
                        body,
                    }) => {
                        warn!(
                            agent = %agent,
                            status = %status,
                            body = %body,
                            "output filter client error, passing through original chunk"
                        );
                        chunk
                    }
                    Err(e) => {
                        warn!(error = %e, "output filter error, passing through original chunk");
                        chunk
                    }
                };

                // Pass through inner processor for metrics/observability
                match inner_processor.process_chunk(processed_chunk) {
                    Ok(Some(final_chunk)) => {
                        if tx.send(final_chunk).await.is_err() {
                            warn!("receiver dropped");
                            break;
                        }
                    }
                    Ok(None) => continue,
                    Err(err) => {
                        warn!("processor error: {}", err);
                        inner_processor.on_error(&err);
                        break;
                    }
                }
            }

            inner_processor.on_complete();
            debug!("output filter streaming completed");
        }
        .instrument(current_span),
    );

    let stream = ReceiverStream::new(rx).map(|chunk| Ok::<_, hyper::Error>(Frame::data(chunk)));
    let stream_body = BoxBody::new(StreamBody::new(stream));

    StreamingResponse {
        body: stream_body,
        processor_handle,
    }
}

/// Truncates a message to the specified maximum length, adding "..." if truncated.
pub fn truncate_message(message: &str, max_length: usize) -> String {
    if message.chars().count() > max_length {
        let truncated: String = message.chars().take(max_length).collect();
        format!("{}...", truncated)
    } else {
        message.to_string()
    }
}

#[cfg(test)]
mod usage_extraction_tests {
    use super::*;

    #[test]
    fn non_streaming_openai_with_cached() {
        let body = br#"{"id":"x","model":"gpt-4o","choices":[],"usage":{"prompt_tokens":12,"completion_tokens":34,"total_tokens":46,"prompt_tokens_details":{"cached_tokens":5}}}"#;
        let u = extract_usage_from_bytes(body);
        assert_eq!(u.prompt_tokens, Some(12));
        assert_eq!(u.completion_tokens, Some(34));
        assert_eq!(u.total_tokens, Some(46));
        assert_eq!(u.cached_input_tokens, Some(5));
        assert_eq!(u.reasoning_tokens, None);
    }

    #[test]
    fn non_streaming_anthropic_with_cache_creation() {
        let body = br#"{"id":"x","model":"claude","usage":{"input_tokens":100,"output_tokens":50,"cache_creation_input_tokens":20,"cache_read_input_tokens":30}}"#;
        let u = extract_usage_from_bytes(body);
        assert_eq!(u.prompt_tokens, Some(100));
        assert_eq!(u.completion_tokens, Some(50));
        assert_eq!(u.total_tokens, Some(150));
        assert_eq!(u.cached_input_tokens, Some(30));
        assert_eq!(u.cache_creation_tokens, Some(20));
    }

    #[test]
    fn streaming_openai_final_chunk_has_usage() {
        let sse = b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}

data: {\"choices\":[{\"delta\":{}, \"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3,\"total_tokens\":10}}

data: [DONE]

";
        let u = extract_usage_from_bytes(sse);
        assert_eq!(u.prompt_tokens, Some(7));
        assert_eq!(u.completion_tokens, Some(3));
        assert_eq!(u.total_tokens, Some(10));
    }

    #[test]
    fn empty_returns_default() {
        assert!(extract_usage_from_bytes(b"").is_empty());
    }

    #[test]
    fn no_usage_in_body_returns_default() {
        assert!(extract_usage_from_bytes(br#"{"ok":true}"#).is_empty());
    }

    #[test]
    fn streaming_resolved_model_from_chunk_without_usage() {
        // DO Inference Cloud often emits model on every chunk and usage only on
        // the last — and the usage chunk itself omits the model field.
        let sse = b"data: {\"id\":\"x\",\"model\":\"openai-gpt-5.4\",\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}

data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3,\"total_tokens\":10}}

data: [DONE]

";
        let u = extract_usage_from_bytes(sse);
        assert_eq!(u.prompt_tokens, Some(7));
        assert_eq!(u.completion_tokens, Some(3));
        assert_eq!(u.resolved_model.as_deref(), Some("openai-gpt-5.4"));
    }

    #[test]
    fn streaming_resolved_model_when_usage_missing() {
        // Even without any usage chunk, we should still surface the upstream
        // model so the obs view doesn't fall back to the router alias.
        let sse = b"data: {\"id\":\"x\",\"model\":\"openai-gpt-5.4\",\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}

data: [DONE]

";
        let u = extract_usage_from_bytes(sse);
        assert_eq!(u.prompt_tokens, None);
        assert_eq!(u.resolved_model.as_deref(), Some("openai-gpt-5.4"));
        assert!(!u.is_empty());
    }
}
