mod constants;
mod service_name_exporter;

pub use constants::{
    error, http, llm, operation_component, routing, signals, OperationNameBuilder,
};
pub use service_name_exporter::{ServiceNameOverrideExporter, SERVICE_NAME_OVERRIDE_KEY};

use opentelemetry::trace::get_active_span;
use opentelemetry::KeyValue;

/// Sets the service name override on the current active OpenTelemetry span.
///
/// This function adds the `service.name.override` attribute to the active
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
    get_active_span(|span| {
        span.set_attribute(KeyValue::new(
            SERVICE_NAME_OVERRIDE_KEY,
            service_name.to_string(),
        ));
    });
}
