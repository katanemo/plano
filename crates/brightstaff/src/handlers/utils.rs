use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::StreamBody;
use hyper::body::Frame;
use opentelemetry::trace::TraceContextExt;
use opentelemetry::KeyValue;
use std::time::Instant;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
use tracing::{info, warn, Instrument};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::signals::{InteractionQuality, SignalAnalyzer, TextBasedSignalAnalyzer, FLAG_MARKER};
use crate::tracing::{llm, set_service_name, signals as signal_constants};
use hermesllm::apis::openai::Message;

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

/// A processor that tracks streaming metrics
pub struct ObservableStreamProcessor {
    service_name: String,
    operation_name: String,
    total_bytes: usize,
    chunk_count: usize,
    start_time: Instant,
    time_to_first_token: Option<u128>,
    messages: Option<Vec<Message>>,
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
        }
    }
}

impl StreamProcessor for ObservableStreamProcessor {
    fn process_chunk(&mut self, chunk: Bytes) -> Result<Option<Bytes>, String> {
        self.total_bytes += chunk.len();
        self.chunk_count += 1;
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

pub fn create_streaming_response<S, P>(
    mut byte_stream: S,
    mut processor: P,
    buffer_size: usize,
) -> StreamingResponse
where
    S: StreamExt<Item = Result<Bytes, reqwest::Error>> + Send + Unpin + 'static,
    P: StreamProcessor,
{
    let (tx, rx) = mpsc::channel::<Bytes>(buffer_size);

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

/// Truncates a message to the specified maximum length, adding "..." if truncated.
pub fn truncate_message(message: &str, max_length: usize) -> String {
    if message.chars().count() > max_length {
        let truncated: String = message.chars().take(max_length).collect();
        format!("{}...", truncated)
    } else {
        message.to_string()
    }
}
