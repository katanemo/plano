use super::resource_span_builder::ResourceSpanBuilder;
use super::shapes::Span;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{interval, Duration};
use tracing::{debug, error, warn};

/// Parse W3C traceparent header into trace_id and parent_span_id
/// Format: "00-{trace_id}-{parent_span_id}-01"
///
/// Returns (trace_id, Option<parent_span_id>)
/// - parent_span_id is None if it's all zeros (0000000000000000), indicating a root span
pub fn parse_traceparent(traceparent: &str) -> (String, Option<String>) {
    let parts: Vec<&str> = traceparent.split('-').collect();
    if parts.len() == 4 {
        let trace_id = parts[1].to_string();
        let parent_span_id = parts[2].to_string();

        // If parent_span_id is all zeros, this is a root span with no parent
        let parent = if parent_span_id == "0000000000000000" {
            None
        } else {
            Some(parent_span_id)
        };

        (trace_id, parent)
    } else {
        warn!("Invalid traceparent format: {}", traceparent);
        // Return empty trace ID and None for parent if parsing fails
        (String::new(), None)
    }
}

/// Collects and batches spans, flushing them to an OTEL collector
///
/// Supports multiple services, with each service (e.g., "archgw(routing)", "archgw(llm)")
/// maintaining its own span queue. Flushes all services together periodically.
///
/// Tracing can be enabled/disabled in two ways:
/// 1. Via arch_config.yaml: presence of `tracing` configuration section
/// 2. Via environment variable: `OTEL_TRACING_ENABLED=true/false`
///
/// When disabled, span recording and flushing are no-ops.
pub struct TraceCollector {
    /// Spans grouped by service name
    /// Key: service name (e.g., "archgw(routing)", "archgw(llm)")
    /// Value: queue of spans for that service
    spans_by_service: Arc<Mutex<HashMap<String, VecDeque<Span>>>>,
    flush_interval: Duration,
    otel_url: String,
    /// Whether tracing is enabled
    enabled: bool,
}

impl TraceCollector {
    /// Create a new trace collector
    ///
    /// # Arguments
    /// * `enabled` - Whether tracing is enabled
    ///   - `Some(true)` - Force enable tracing
    ///   - `Some(false)` - Force disable tracing
    ///   - `None` - Check `OTEL_TRACING_ENABLED` env var (defaults to true if not set)
    ///
    /// Other parameters are read from environment variables:
    /// - `TRACE_FLUSH_INTERVAL_MS` - Flush interval in milliseconds (default: 1000)
    /// - `OTEL_COLLECTOR_URL` - OTEL collector endpoint (default: http://localhost:9903/v1/traces)
    pub fn new(enabled: Option<bool>) -> Self {
        let flush_interval_ms = std::env::var("TRACE_FLUSH_INTERVAL_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1000);

        let otel_url = std::env::var("OTEL_COLLECTOR_URL")
            .unwrap_or_else(|_| "http://localhost:9903/v1/traces".to_string());

        // Determine if tracing is enabled:
        // 1. Use explicit parameter if provided
        // 2. Otherwise check OTEL_TRACING_ENABLED env var
        // 3. Default to false if neither is set (tracing opt-in, not opt-out)
        let enabled = enabled.unwrap_or_else(|| {
            std::env::var("OTEL_TRACING_ENABLED")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(false)
        });

        debug!(
            "TraceCollector initialized: flush_interval={}ms, url={}, enabled={}",
            flush_interval_ms, otel_url, enabled
        );

        Self {
            spans_by_service: Arc::new(Mutex::new(HashMap::new())),
            flush_interval: Duration::from_millis(flush_interval_ms),
            otel_url,
            enabled,
        }
    }

    /// Record a span for a specific service
    ///
    /// # Arguments
    /// * `service_name` - Name of the service (e.g., "archgw(routing)", "archgw(llm)")
    /// * `span` - The span to record
    pub fn record_span(&self, service_name: impl Into<String>, span: Span) {
        // Skip recording if tracing is disabled
        if !self.enabled {
            return;
        }

        let service_name = service_name.into();

        // Use try_lock to avoid blocking in async contexts
        // If the lock is held, we skip recording (telemetry shouldn't block the app)
        if let Ok(mut spans_by_service) = self.spans_by_service.try_lock() {
            // Get or create the queue for this service
            let spans = spans_by_service
                .entry(service_name)
                .or_insert_with(VecDeque::new);

            spans.push_back(span);
        } else {
            // Lock contention - skip recording this span
            debug!("Skipped span recording due to lock contention");
        }
        // Flushing is handled by the periodic background flusher (see `start_background_flusher`).
    }

    /// Flush all buffered spans to the OTEL collector
    /// Builds ResourceSpans for each service with spans
    pub async fn flush(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Skip flushing if tracing is disabled
        if !self.enabled {
            return Ok(());
        }

        let mut spans_by_service = self.spans_by_service.lock().await;

        if spans_by_service.is_empty() {
            return Ok(());
        }

        // Snapshot and drain all services' spans
        let service_batches: Vec<(String, Vec<Span>)> = spans_by_service
            .iter_mut()
            .filter_map(|(service_name, spans)| {
                if spans.is_empty() {
                    None
                } else {
                    Some((service_name.clone(), spans.drain(..).collect()))
                }
            })
            .collect();

        drop(spans_by_service); // Release lock before HTTP call

        if service_batches.is_empty() {
            return Ok(());
        }

        let total_spans: usize = service_batches.iter().map(|(_, spans)| spans.len()).sum();
        debug!(
            "Flushing {} spans across {} services to OTEL collector",
            total_spans,
            service_batches.len()
        );

        // Build canonical OTEL payload structure - one ResourceSpan per service
        let resource_spans = self.build_resource_spans(service_batches);

        match self.send_to_otel(resource_spans).await {
            Ok(_) => {
                debug!("Successfully flushed {} spans", total_spans);
                Ok(())
            }
            Err(e) => {
                warn!("Failed to send spans to OTEL collector: {:?}", e);
                Err(e)
            }
        }
    }

    /// Build OTEL-compliant resource spans from collected spans, one ResourceSpan per service
    fn build_resource_spans(
        &self,
        service_batches: Vec<(String, Vec<Span>)>,
    ) -> Vec<super::shapes::ResourceSpan> {
        service_batches
            .into_iter()
            .map(|(service_name, spans)| {
                ResourceSpanBuilder::new(&service_name)
                    .add_spans(spans)
                    .build()
            })
            .collect()
    }

    /// Send resource spans to OTEL collector
    /// Serializes as {"resourceSpans": [...]} per OTEL spec
    async fn send_to_otel(
        &self,
        resource_spans: Vec<super::shapes::ResourceSpan>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let client = reqwest::Client::new();

        // Create OTEL payload with proper structure
        let payload = serde_json::json!({
            "resourceSpans": resource_spans
        });

        let response = client
            .post(&self.otel_url)
            .header("Content-Type", "application/json")
            .json(&payload)
            .timeout(Duration::from_secs(5))
            .send()
            .await?;

        if !response.status().is_success() {
            warn!(
                "OTEL collector returned non-success status: {}",
                response.status()
            );
            return Err(format!("OTEL collector error: {}", response.status()).into());
        }

        Ok(())
    }

    /// Start a background task that periodically flushes traces
    /// Returns a join handle that can be used to stop the flusher
    pub fn start_background_flusher(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        let flush_interval = self.flush_interval;

        tokio::spawn(async move {
            let mut ticker = interval(flush_interval);

            loop {
                ticker.tick().await;

                if let Err(e) = self.flush().await {
                    error!("Background trace flush failed: {:?}", e);
                }
            }
        })
    }

    /// Get current number of buffered spans across all services (for testing/monitoring)
    pub async fn buffered_count(&self) -> usize {
        self.spans_by_service
            .lock()
            .await
            .values()
            .map(|spans| spans.len())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traces::SpanBuilder;

    #[tokio::test]
    async fn test_collector_basic() {
        let collector = TraceCollector::new(Some(true));

        let span = SpanBuilder::new("test_operation")
            .with_trace_id("abc123")
            .build();

        collector.record_span("test-service", span);

        assert_eq!(collector.buffered_count().await, 1);
    }

    #[tokio::test]
    async fn test_collector_auto_flush() {
        // Since batch-triggered flush behavior was removed, record two spans and verify both are buffered
        let collector = Arc::new(TraceCollector::new(Some(true)));

        let span1 = SpanBuilder::new("test1").build();
        let span2 = SpanBuilder::new("test2").build();

        collector.record_span("test-service", span1);
        collector.record_span("test-service", span2);

        // With no batch-triggered flush, both spans should remain buffered
        assert_eq!(collector.buffered_count().await, 2);
    }
}
