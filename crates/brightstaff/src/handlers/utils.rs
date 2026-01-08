use bytes::Bytes;
use common::traces::{Attribute, AttributeValue, Event, Span, TraceCollector};
use http_body_util::combinators::BoxBody;
use http_body_util::StreamBody;
use hyper::body::Frame;
use std::sync::Arc;
use std::time::{Instant, SystemTime};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
use tracing::warn;

// Import tracing constants and signals
use crate::signals::{InteractionQuality, SignalAnalyzer, TextBasedSignalAnalyzer, FLAG_MARKER};
use crate::tracing::{error, llm, signals as signal_constants};
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

/// A processor that tracks streaming metrics and finalizes the span
pub struct ObservableStreamProcessor {
    collector: Arc<TraceCollector>,
    service_name: String,
    span: Span,
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
    /// * `collector` - The trace collector to record the span to
    /// * `service_name` - The service name for this span (e.g., "archgw(llm)")
    /// * `span` - The span to finalize after streaming completes
    /// * `start_time` - When the request started (for duration calculation)
    /// * `messages` - Optional conversation messages for signal analysis
    pub fn new(
        collector: Arc<TraceCollector>,
        service_name: impl Into<String>,
        span: Span,
        start_time: Instant,
        messages: Option<Vec<Message>>,
    ) -> Self {
        Self {
            collector,
            service_name: service_name.into(),
            span,
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
        // Update span with streaming metrics and end time
        let end_time_nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();

        self.span.end_time_unix_nano = format!("{}", end_time_nanos);

        // Add streaming metrics as attributes using constants
        self.span.attributes.push(Attribute {
            key: llm::RESPONSE_BYTES.to_string(),
            value: AttributeValue {
                string_value: Some(self.total_bytes.to_string()),
            },
        });

        self.span.attributes.push(Attribute {
            key: llm::DURATION_MS.to_string(),
            value: AttributeValue {
                string_value: Some(self.start_time.elapsed().as_millis().to_string()),
            },
        });

        // Add time to first token if available (streaming only)
        if let Some(ttft) = self.time_to_first_token {
            self.span.attributes.push(Attribute {
                key: llm::TIME_TO_FIRST_TOKEN_MS.to_string(),
                value: AttributeValue {
                    string_value: Some(ttft.to_string()),
                },
            });

            // Add time to first token as a span event
            // Calculate the timestamp by adding ttft duration to span start time
            if let Ok(start_time_nanos) = self.span.start_time_unix_nano.parse::<u128>() {
                // Convert ttft from milliseconds to nanoseconds and add to start time
                let event_timestamp = start_time_nanos + (ttft * 1_000_000);
                let mut event =
                    Event::new(llm::TIME_TO_FIRST_TOKEN_MS.to_string(), event_timestamp);
                event.add_attribute(llm::TIME_TO_FIRST_TOKEN_MS.to_string(), ttft.to_string());

                // Initialize events vector if needed
                if self.span.events.is_none() {
                    self.span.events = Some(Vec::new());
                }

                if let Some(ref mut events) = self.span.events {
                    events.push(event);
                }
            }
        }

        // Analyze signals if messages are available and add to span attributes
        if let Some(ref messages) = self.messages {
            let analyzer: Box<dyn SignalAnalyzer> = Box::new(TextBasedSignalAnalyzer::new());
            let report = analyzer.analyze(messages);

            // Add overall quality
            self.span.attributes.push(Attribute {
                key: signal_constants::QUALITY.to_string(),
                value: AttributeValue {
                    string_value: Some(format!("{:?}", report.overall_quality)),
                },
            });

            // Add repair/follow-up metrics if concerning
            if report.follow_up.is_concerning || report.follow_up.repair_count > 0 {
                self.span.attributes.push(Attribute {
                    key: signal_constants::REPAIR_COUNT.to_string(),
                    value: AttributeValue {
                        string_value: Some(report.follow_up.repair_count.to_string()),
                    },
                });

                self.span.attributes.push(Attribute {
                    key: signal_constants::REPAIR_RATIO.to_string(),
                    value: AttributeValue {
                        string_value: Some(format!("{:.3}", report.follow_up.repair_ratio)),
                    },
                });
            }

            // Add flag marker to operation name if any concerning signal is detected
            let should_flag = report.frustration.has_frustration
                || report.repetition.has_looping
                || report.escalation.escalation_requested
                || matches!(
                    report.overall_quality,
                    InteractionQuality::Poor | InteractionQuality::Severe
                );

            if should_flag {
                // Prepend flag marker to the operation name
                self.span.name = format!("{} {}", self.span.name, FLAG_MARKER);
            }

            // Add key signal metrics
            if report.frustration.has_frustration {
                self.span.attributes.push(Attribute {
                    key: signal_constants::FRUSTRATION_COUNT.to_string(),
                    value: AttributeValue {
                        string_value: Some(report.frustration.frustration_count.to_string()),
                    },
                });
                self.span.attributes.push(Attribute {
                    key: signal_constants::FRUSTRATION_SEVERITY.to_string(),
                    value: AttributeValue {
                        string_value: Some(report.frustration.severity.to_string()),
                    },
                });
            }

            if report.repetition.has_looping {
                self.span.attributes.push(Attribute {
                    key: signal_constants::REPETITION_COUNT.to_string(),
                    value: AttributeValue {
                        string_value: Some(report.repetition.repetition_count.to_string()),
                    },
                });
            }

            if report.escalation.escalation_requested {
                self.span.attributes.push(Attribute {
                    key: signal_constants::ESCALATION_REQUESTED.to_string(),
                    value: AttributeValue {
                        string_value: Some("true".to_string()),
                    },
                });
            }

            if report.positive_feedback.has_positive_feedback {
                self.span.attributes.push(Attribute {
                    key: signal_constants::POSITIVE_FEEDBACK_COUNT.to_string(),
                    value: AttributeValue {
                        string_value: Some(report.positive_feedback.positive_count.to_string()),
                    },
                });
            }
        }

        // Record the finalized span
        self.collector
            .record_span(&self.service_name, self.span.clone());
    }

    fn on_error(&mut self, error_msg: &str) {
        warn!("Stream error in PassthroughProcessor: {}", error_msg);

        // Update span with error info and end time
        let end_time_nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();

        self.span.end_time_unix_nano = format!("{}", end_time_nanos);

        self.span.attributes.push(Attribute {
            key: error::ERROR.to_string(),
            value: AttributeValue {
                string_value: Some("true".to_string()),
            },
        });

        self.span.attributes.push(Attribute {
            key: error::MESSAGE.to_string(),
            value: AttributeValue {
                string_value: Some(error_msg.to_string()),
            },
        });

        self.span.attributes.push(Attribute {
            key: llm::DURATION_MS.to_string(),
            value: AttributeValue {
                string_value: Some(self.start_time.elapsed().as_millis().to_string()),
            },
        });

        // Record the error span
        self.collector
            .record_span(&self.service_name, self.span.clone());
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

    // Spawn a task to process and forward chunks
    let processor_handle = tokio::spawn(async move {
        let mut is_first_chunk = true;

        while let Some(item) = byte_stream.next().await {
            let chunk = match item {
                Ok(chunk) => chunk,
                Err(err) => {
                    let err_msg = format!("Error receiving chunk: {:?}", err);
                    warn!("{}", err_msg);
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
                        warn!("Receiver dropped");
                        break;
                    }
                }
                Ok(None) => {
                    // Skip this chunk
                    continue;
                }
                Err(err) => {
                    warn!("Processor error: {}", err);
                    processor.on_error(&err);
                    break;
                }
            }
        }

        processor.on_complete();
    });

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
