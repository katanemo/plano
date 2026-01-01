use super::constants::{resource, scope};
use super::shapes::{Attribute, AttributeValue, Resource, ResourceSpan, Scope, ScopeSpan, Span};
use std::collections::HashMap;

/// Builder for creating OTEL ResourceSpan structures
///
/// Provides a fluent API for building the resource/scope/span hierarchy
pub struct ResourceSpanBuilder {
    service_name: String,
    resource_attributes: HashMap<String, String>,
    scope_name: String,
    scope_version: String,
    spans: Vec<Span>,
}

impl ResourceSpanBuilder {
    /// Create a new ResourceSpan builder with service name
    pub fn new(service_name: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
            resource_attributes: HashMap::new(),
            scope_name: scope::DEFAULT_NAME.to_string(),
            scope_version: scope::DEFAULT_VERSION.to_string(),
            spans: Vec::new(),
        }
    }

    /// Add a resource attribute (e.g., deployment.environment, host.name)
    pub fn with_resource_attribute(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        self.resource_attributes.insert(key.into(), value.into());
        self
    }

    /// Set the instrumentation scope name
    pub fn with_scope_name(mut self, name: impl Into<String>) -> Self {
        self.scope_name = name.into();
        self
    }

    /// Set the instrumentation scope version
    pub fn with_scope_version(mut self, version: impl Into<String>) -> Self {
        self.scope_version = version.into();
        self
    }

    /// Add a single span
    pub fn add_span(mut self, span: Span) -> Self {
        self.spans.push(span);
        self
    }

    /// Add multiple spans
    pub fn add_spans(mut self, spans: Vec<Span>) -> Self {
        self.spans.extend(spans);
        self
    }

    /// Build the ResourceSpan
    pub fn build(self) -> ResourceSpan {
        // Build resource attributes
        let mut attributes = vec![Attribute {
            key: resource::SERVICE_NAME.to_string(),
            value: AttributeValue {
                string_value: Some(self.service_name),
            },
        }];

        // Add custom resource attributes
        for (key, value) in self.resource_attributes {
            attributes.push(Attribute {
                key,
                value: AttributeValue {
                    string_value: Some(value),
                },
            });
        }

        let resource = Resource { attributes };

        let scope = Scope {
            name: self.scope_name,
            version: self.scope_version,
            attributes: Vec::new(),
        };

        let scope_span = ScopeSpan {
            scope,
            spans: self.spans,
        };

        ResourceSpan {
            resource,
            scope_spans: vec![scope_span],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traces::SpanBuilder;

    #[test]
    fn test_resource_span_builder() {
        let span1 = SpanBuilder::new("operation1").build();
        let span2 = SpanBuilder::new("operation2").build();

        let resource_span = ResourceSpanBuilder::new("test-service")
            .with_resource_attribute("deployment.environment", "production")
            .with_scope_name("test-scope")
            .add_span(span1)
            .add_span(span2)
            .build();

        assert_eq!(resource_span.resource.attributes.len(), 2); // service.name + custom
        assert_eq!(resource_span.scope_spans.len(), 1);
        assert_eq!(resource_span.scope_spans[0].spans.len(), 2);
    }
}
