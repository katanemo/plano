//! Service Name Override Exporter
//!
//! This module provides a custom SpanExporter that allows per-span service.name overrides.
//! In OpenTelemetry, `service.name` is part of the Resource, which is tied to the TracerProvider.
//! However, if you need different service names for different spans (e.g., `plano(orchestrator)`,
//! `plano(filter)`, `plano(llm)`) within the same provider, this exporter handles that by:
//!
//! 1. Looking for a special span attribute `service.name.override`
//! 2. Grouping spans by their effective service name
//! 3. Exporting each group via a dedicated OTLP exporter whose Resource has the correct
//!    `service.name`
//!
//! All per-service exporters are created eagerly at construction time so that no tonic
//! channel creation happens later inside `futures_executor::block_on` (which the
//! `BatchSpanProcessor` uses and which lacks a tokio runtime).
//!
//! # Usage
//!
//! ```rust
//! use brightstaff::tracing::{set_service_name, operation_component};
//!
//! // In your instrumented code, set the service name override:
//! set_service_name(operation_component::LLM);
//! ```

use opentelemetry::Key;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SpanData, SpanExporter};
use opentelemetry_sdk::Resource;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::Mutex;

use super::operation_component;

/// The attribute key used to override the service name for a specific span.
/// Set this as a span attribute to route the span to a different service.
pub const SERVICE_NAME_OVERRIDE_KEY: &str = "service.name.override";

/// Default service name used when no override is set on a span.
const DEFAULT_SERVICE_NAME: &str = "plano";

/// All known service names that will have dedicated exporters.
const ALL_SERVICE_NAMES: &[&str] = &[
    DEFAULT_SERVICE_NAME,
    operation_component::INBOUND,
    operation_component::ROUTING,
    operation_component::ORCHESTRATOR,
    operation_component::AGENT_FILTER,
    operation_component::AGENT,
    operation_component::LLM,
];

/// Span attribute keys to remove before export.
const FILTERED_ATTR_KEYS: &[&str] = &[
    "busy_ns",
    "idle_ns",
    "thread.id",
    "thread.name",
    "code.file.path",
    "code.line.number",
    "code.module.name",
    "target",
];

/// A SpanExporter that supports per-span `service.name` overrides.
///
/// Internally it holds one OTLP exporter per known service name.  Each exporter
/// has its own `Resource` with the correct `service.name`, so backends like
/// Jaeger see the spans under the right service.
pub struct ServiceNameOverrideExporter {
    /// Map from service name → pre-created OTLP exporter (behind tokio Mutex
    /// because `SpanExporter::export` takes `&self` and the future must be Send).
    exporters: HashMap<String, Mutex<opentelemetry_otlp::SpanExporter>>,
}

// Manual Debug because `opentelemetry_otlp::SpanExporter` doesn't implement Debug
impl std::fmt::Debug for ServiceNameOverrideExporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceNameOverrideExporter")
            .field("services", &self.exporters.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl ServiceNameOverrideExporter {
    /// Create a new `ServiceNameOverrideExporter`.
    ///
    /// This eagerly creates one OTLP gRPC exporter per known service name so
    /// that the tonic channel is established while a tokio runtime is available.
    ///
    /// # Arguments
    /// * `endpoint` – The OTLP collector endpoint URL (e.g. `http://localhost:4317`)
    pub fn new(endpoint: &str) -> Self {
        let mut exporters = HashMap::new();

        for &service_name in ALL_SERVICE_NAMES {
            let resource = Resource::builder_empty()
                .with_service_name(service_name)
                .build();

            let mut exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint)
                .build()
                .expect("Failed to create OTLP span exporter");

            exporter.set_resource(&resource);
            exporters.insert(service_name.to_string(), Mutex::new(exporter));
        }

        Self { exporters }
    }
}

impl SpanExporter for ServiceNameOverrideExporter {
    fn export(
        &self,
        batch: Vec<SpanData>,
    ) -> impl std::future::Future<Output = OTelSdkResult> + Send {
        let override_key = Key::new(SERVICE_NAME_OVERRIDE_KEY);

        // Group spans by their effective service name
        let mut spans_by_service: HashMap<String, Vec<SpanData>> = HashMap::new();

        let should_filter = !tracing::enabled!(tracing::Level::DEBUG);

        for span in batch {
            let mut span = span;

            if should_filter {
                span.attributes
                    .retain(|kv| !FILTERED_ATTR_KEYS.contains(&kv.key.as_str()));
            }

            let service_name = span
                .attributes
                .iter()
                .find(|kv| kv.key == override_key)
                .map(|kv| kv.value.to_string())
                .unwrap_or_else(|| DEFAULT_SERVICE_NAME.to_string());

            spans_by_service.entry(service_name).or_default().push(span);
        }

        // Collect grouped spans into a Vec so the async block owns the data.
        let results: Vec<(String, Vec<SpanData>)> = spans_by_service.into_iter().collect();
        async move {
            for (service_name, spans) in results {
                // Look up the pre-created exporter; fall back to default if
                // the service name isn't one of the known ones.
                let key = if self.exporters.contains_key(&service_name) {
                    service_name.clone()
                } else {
                    DEFAULT_SERVICE_NAME.to_string()
                };

                if let Some(exporter_mutex) = self.exporters.get(&key) {
                    let exporter = exporter_mutex.lock().await;
                    if let Err(e) = exporter.export(spans).await {
                        tracing::warn!(
                            service = %service_name,
                            error = ?e,
                            "Failed to export spans"
                        );
                    }
                }
            }
            Ok(())
        }
    }

    fn shutdown_with_timeout(&mut self, timeout: Duration) -> OTelSdkResult {
        for (_, exporter_mutex) in self.exporters.iter() {
            if let Ok(mut exporter) = exporter_mutex.try_lock() {
                let _ = exporter.shutdown_with_timeout(timeout);
            }
        }
        Ok(())
    }

    fn set_resource(&mut self, _resource: &Resource) {
        // Each inner exporter already has its own resource set at creation time.
        // Nothing to propagate.
    }
}
