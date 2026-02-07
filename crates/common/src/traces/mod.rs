// Original tracing types (OTEL structures)
mod shapes;
// New tracing utilities
mod constants;
mod resource_span_builder;
mod span_builder;

// Re-export original types
pub use shapes::{
    Attribute, AttributeValue, Event, Resource, ResourceSpan, Scope, ScopeSpan, Span, Traceparent,
    TraceparentNewError,
};

// Re-export new utilities
pub use constants::*;
pub use resource_span_builder::ResourceSpanBuilder;
pub use span_builder::{generate_random_span_id, SpanBuilder, SpanKind};
