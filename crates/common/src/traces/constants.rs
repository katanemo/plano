/// OpenTelemetry semantic convention constants for tracing
///
/// These constants ensure consistency across the codebase and prevent typos
/// Resource attribute keys following OTEL semantic conventions
pub mod resource {
    /// Logical name of the service
    pub const SERVICE_NAME: &str = "service.name";

    /// Version of the service
    pub const SERVICE_VERSION: &str = "service.version";

    /// Service namespace/environment
    pub const SERVICE_NAMESPACE: &str = "service.namespace";

    /// Service instance ID
    pub const SERVICE_INSTANCE_ID: &str = "service.instance.id";
}

/// Instrumentation scope defaults
pub mod scope {
    /// Default scope name for tracing instrumentation
    pub const DEFAULT_NAME: &str = "brightstaff.tracing";

    /// Default scope version
    pub const DEFAULT_VERSION: &str = "1.0.0";
}
