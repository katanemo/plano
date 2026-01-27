use std::collections::HashMap;

use common::configuration::{CustomTraceAttribute, CustomTraceAttributeType};
use common::traces::SpanBuilder;
use hyper::header::{HeaderMap, HeaderName};

pub fn extract_custom_trace_attributes(
    headers: &HeaderMap,
    custom_attributes: Option<&[CustomTraceAttribute]>,
) -> HashMap<String, String> {
    let mut attributes = HashMap::new();
    let Some(custom_attributes) = custom_attributes else {
        return attributes;
    };

    for attribute in custom_attributes {
        // Normalize/validate the configured header name; skip invalid names.
        let header_name = match HeaderName::from_bytes(attribute.header.as_bytes()) {
            Ok(name) => name,
            Err(_) => continue,
        };

        // Extract header value as UTF-8 text; skip missing or invalid values.
        let raw_value = match headers
            .get(header_name)
            .and_then(|value| value.to_str().ok())
        {
            Some(value) => value.trim(),
            None => continue,
        };

        // Parse the header value according to the configured type.
        let parsed_value = match attribute.value_type {
            CustomTraceAttributeType::Str => Some(raw_value.to_string()),
            CustomTraceAttributeType::Bool => raw_value.parse::<bool>().ok().map(|v| v.to_string()),
            CustomTraceAttributeType::Float => raw_value.parse::<f64>().ok().map(|v| v.to_string()),
            CustomTraceAttributeType::Int => raw_value.parse::<i64>().ok().map(|v| v.to_string()),
        };

        // Only include attributes that successfully parsed.
        if let Some(value) = parsed_value {
            attributes.insert(attribute.key.clone(), value);
        }
    }

    attributes
}

pub fn collect_custom_trace_attributes(
    headers: &HeaderMap,
    custom_attributes: Option<&[CustomTraceAttribute]>,
) -> HashMap<String, String> {
    extract_custom_trace_attributes(headers, custom_attributes)
}

pub fn append_span_attributes(
    mut span_builder: SpanBuilder,
    attributes: &HashMap<String, String>,
) -> SpanBuilder {
    for (key, value) in attributes {
        span_builder = span_builder.with_attribute(key, value);
    }
    span_builder
}

#[cfg(test)]
mod tests {
    use super::extract_custom_trace_attributes;
    use common::configuration::{CustomTraceAttribute, CustomTraceAttributeType};
    use hyper::header::{HeaderMap, HeaderValue};

    #[test]
    fn extracts_and_parses_custom_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-workspace-id", HeaderValue::from_static("ws_123"));
        headers.insert("x-tenant-id", HeaderValue::from_static("ten_456"));
        headers.insert("x-user-id", HeaderValue::from_static("usr_789"));
        headers.insert("x-admin-level", HeaderValue::from_static("3"));
        headers.insert("x-is-internal", HeaderValue::from_static("true"));
        headers.insert("x-budget", HeaderValue::from_static("42.5"));
        headers.insert("x-bad-int", HeaderValue::from_static("nope"));

        let custom_attributes = vec![
            CustomTraceAttribute {
                key: "workspace.id".to_string(),
                value_type: CustomTraceAttributeType::Str,
                header: "x-workspace-id".to_string(),
            },
            CustomTraceAttribute {
                key: "tenant.id".to_string(),
                value_type: CustomTraceAttributeType::Str,
                header: "x-tenant-id".to_string(),
            },
            CustomTraceAttribute {
                key: "user.id".to_string(),
                value_type: CustomTraceAttributeType::Str,
                header: "x-user-id".to_string(),
            },
            CustomTraceAttribute {
                key: "admin.level".to_string(),
                value_type: CustomTraceAttributeType::Int,
                header: "x-admin-level".to_string(),
            },
            CustomTraceAttribute {
                key: "is.internal".to_string(),
                value_type: CustomTraceAttributeType::Bool,
                header: "x-is-internal".to_string(),
            },
            CustomTraceAttribute {
                key: "budget.value".to_string(),
                value_type: CustomTraceAttributeType::Float,
                header: "x-budget".to_string(),
            },
            CustomTraceAttribute {
                key: "bad.int".to_string(),
                value_type: CustomTraceAttributeType::Int,
                header: "x-bad-int".to_string(),
            },
            CustomTraceAttribute {
                key: "missing.header".to_string(),
                value_type: CustomTraceAttributeType::Str,
                header: "x-missing".to_string(),
            },
        ];

        let attrs = extract_custom_trace_attributes(&headers, Some(&custom_attributes));

        assert_eq!(attrs.get("workspace.id"), Some(&"ws_123".to_string()));
        assert_eq!(attrs.get("tenant.id"), Some(&"ten_456".to_string()));
        assert_eq!(attrs.get("user.id"), Some(&"usr_789".to_string()));
        assert_eq!(attrs.get("admin.level"), Some(&"3".to_string()));
        assert_eq!(attrs.get("is.internal"), Some(&"true".to_string()));
        assert_eq!(attrs.get("budget.value"), Some(&"42.5".to_string()));
        assert!(!attrs.contains_key("bad.int"));
        assert!(!attrs.contains_key("missing.header"));
    }
}
