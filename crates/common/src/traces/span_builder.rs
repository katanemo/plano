use super::shapes::{Attribute, AttributeValue, Span};
use std::collections::HashMap;
use std::time::SystemTime;

/// OpenTelemetry span kinds
/// https://opentelemetry.io/docs/specs/otel/trace/api/#spankind
#[derive(Debug, Clone, Copy)]
pub enum SpanKind {
    /// Default value. Indicates that the span represents an internal operation within an application
    Internal = 0,
    /// Indicates that the span describes a request to some remote service
    Client = 3,
}

/// Builder for creating OTEL-compliant spans with a fluent API
///
/// This is the recommended way to create spans with proper trace context.
///
/// # Example
/// ```no_run
/// use common::traces::{SpanBuilder, SpanKind};
/// use std::time::SystemTime;
///
/// let span = SpanBuilder::new("router_chat")
///     .with_trace_id("abc123")
///     .with_parent_span_id("parent456")
///     .with_kind(SpanKind::Internal)
///     .with_attribute("http.method", "POST")
///     .with_attribute("http.path", "/v1/chat/completions")
///     .build();
/// ```
pub struct SpanBuilder {
    name: String,
    trace_id: Option<String>,
    parent_span_id: Option<String>,
    start_time: SystemTime,
    end_time: Option<SystemTime>,
    kind: SpanKind,
    attributes: HashMap<String, String>,
    span_id: Option<String>,
}

impl SpanBuilder {
    /// Create a new span builder
    ///
    /// # Arguments
    /// * `name` - The operation name for this span (e.g., "router_chat", "determine_route")
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            trace_id: None,
            parent_span_id: None,
            start_time: SystemTime::now(),
            end_time: None,
            kind: SpanKind::Internal,
            attributes: HashMap::new(),
            span_id: None,
        }
    }

    /// Set the trace ID (extracted from traceparent or OpenTelemetry context)
    pub fn with_trace_id(mut self, trace_id: impl Into<String>) -> Self {
        self.trace_id = Some(trace_id.into());
        self
    }

    pub fn with_span_id(mut self, span_id: impl Into<String>) -> Self {
        self.span_id = Some(span_id.into());
        self
    }

    /// Set the parent span ID to link this span to its parent
    pub fn with_parent_span_id(mut self, parent_span_id: impl Into<String>) -> Self {
        self.parent_span_id = Some(parent_span_id.into());
        self
    }

    /// Set the span kind (defaults to Internal)
    pub fn with_kind(mut self, kind: SpanKind) -> Self {
        self.kind = kind;
        self
    }

    /// Set explicit start time (defaults to now)
    pub fn with_start_time(mut self, start_time: SystemTime) -> Self {
        self.start_time = start_time;
        self
    }

    /// Set explicit end time (defaults to build time)
    pub fn with_end_time(mut self, end_time: SystemTime) -> Self {
        self.end_time = Some(end_time);
        self
    }

    /// Add a single attribute to the span
    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }

    /// Add multiple attributes at once
    pub fn with_attributes(mut self, attrs: HashMap<String, String>) -> Self {
        self.attributes.extend(attrs);
        self
    }

    /// Build the span, consuming the builder
    ///
    /// Creates a complete OTEL-compliant span with all provided attributes,
    /// generating span_id and using provided or random trace_id.
    pub fn build(self) -> Span {
        let end_time = self.end_time.unwrap_or_else(SystemTime::now);

        let start_nanos = system_time_to_nanos(self.start_time);
        let end_nanos = system_time_to_nanos(end_time);

        // Generate trace_id if not provided
        let trace_id = self.trace_id.unwrap_or_else(generate_random_trace_id);

        // Create attributes in OTEL format
        let attributes: Vec<Attribute> = self
            .attributes
            .into_iter()
            .map(|(key, value)| Attribute {
                key,
                value: AttributeValue {
                    string_value: Some(value),
                },
            })
            .collect();

        // Build span directly without going through Span::new()
        Span {
            trace_id,
            span_id: self.span_id.unwrap_or_else(generate_random_span_id),
            parent_span_id: self.parent_span_id,
            name: self.name,
            start_time_unix_nano: format!("{}", start_nanos),
            end_time_unix_nano: format!("{}", end_nanos),
            kind: self.kind as u32,
            attributes,
            events: None,
        }
    }
}

/// Convert SystemTime to nanoseconds since UNIX epoch for OTEL
fn system_time_to_nanos(time: SystemTime) -> u128 {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

/// Generate a random span ID (16 hex characters = 8 bytes)
pub fn generate_random_span_id() -> String {
    use rand::RngCore;
    let mut rng = rand::thread_rng();
    let mut random_bytes = [0u8; 8];
    rng.fill_bytes(&mut random_bytes);
    hex::encode(random_bytes)
}

/// Generate a random trace ID (32 hex characters = 16 bytes)
fn generate_random_trace_id() -> String {
    use rand::RngCore;
    let mut rng = rand::thread_rng();
    let mut random_bytes = [0u8; 16];
    rng.fill_bytes(&mut random_bytes);
    hex::encode(random_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_span_builder_basic() {
        let span = SpanBuilder::new("test_operation")
            .with_trace_id("abc123")
            .with_parent_span_id("parent123")
            .with_attribute("key", "value")
            .build();

        assert_eq!(span.name, "test_operation");
        assert_eq!(span.trace_id, "abc123");
        assert_eq!(span.parent_span_id, Some("parent123".to_string()));
        assert_eq!(span.attributes.len(), 1);
    }

    #[test]
    fn test_span_builder_no_parent() {
        let span = SpanBuilder::new("root_span")
            .with_trace_id("xyz789")
            .build();

        assert_eq!(span.name, "root_span");
        assert_eq!(span.trace_id, "xyz789");
        assert_eq!(span.parent_span_id, None);
    }
}
