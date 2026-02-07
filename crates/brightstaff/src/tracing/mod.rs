mod constants;
mod service_name_exporter;

pub use constants::{
    error, http, llm, operation_component, routing, signals, OperationNameBuilder,
};
pub use service_name_exporter::{ServiceNameOverrideExporter, SERVICE_NAME_OVERRIDE_KEY};

use opentelemetry::trace::TraceContextExt;
use opentelemetry::KeyValue;
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Sets the service name override on the current tracing span.
///
/// This function adds the `service.name.override` attribute to the underlying
/// OpenTelemetry span, which allows observability backends to filter and group
/// spans by their logical service (e.g., `plano(llm)`, `plano(filter)`).
///
/// # Arguments
/// * `service_name` - The service name to use (e.g., `operation_component::LLM`)
///
/// # Example
/// ```rust,ignore
/// use brightstaff::tracing::{set_service_name, operation_component};
///
/// // Inside a traced function:
/// set_service_name(operation_component::LLM);
/// ```
pub fn set_service_name(service_name: &str) {
    let span = tracing::Span::current();
    let otel_context = span.context();
    let otel_span = otel_context.span();
    otel_span.set_attribute(KeyValue::new(
        SERVICE_NAME_OVERRIDE_KEY,
        service_name.to_string(),
    ));
}

/// Sets the service name override on the given tracing span.
///
/// Use this when you have a specific span reference and want to set
/// the service name override attribute on it.
///
/// # Arguments
/// * `span` - The tracing span to set the service name on
/// * `service_name` - The service name to use (e.g., `operation_component::LLM`)
pub fn set_service_name_on_span(span: &tracing::Span, service_name: &str) {
    let otel_context = span.context();
    let otel_span = otel_context.span();
    otel_span.set_attribute(KeyValue::new(
        SERVICE_NAME_OVERRIDE_KEY,
        service_name.to_string(),
    ));
}
