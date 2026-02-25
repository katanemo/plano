use std::collections::HashMap;

use common::configuration::SpanAttributes;
use common::traces::SpanBuilder;
use hyper::header::HeaderMap;

pub fn collect_custom_trace_attributes(
    headers: &HeaderMap,
    span_attributes: Option<&SpanAttributes>,
) -> HashMap<String, String> {
    let mut attributes = HashMap::new();
    let Some(span_attributes) = span_attributes else {
        return attributes;
    };

    if let Some(static_attributes) = span_attributes.static_attributes.as_ref() {
        for (key, value) in static_attributes {
            attributes.insert(key.clone(), value.clone());
        }
    }

    let Some(header_prefixes) = span_attributes.header_prefixes.as_deref() else {
        return attributes;
    };
    if header_prefixes.is_empty() {
        return attributes;
    }

    for (name, value) in headers.iter() {
        let header_name = name.as_str();
        let matched_prefix = header_prefixes
            .iter()
            .find(|prefix| header_name.starts_with(prefix.as_str()))
            .map(String::as_str);
        let Some(prefix) = matched_prefix else {
            continue;
        };

        let Some(raw_value) = value.to_str().ok().map(str::trim) else {
            continue;
        };

        let suffix = header_name.strip_prefix(prefix).unwrap_or("");
        let suffix_key = suffix.trim_start_matches('-').replace('-', ".");
        if suffix_key.is_empty() {
            continue;
        }

        attributes.insert(suffix_key, raw_value.to_string());
    }

    attributes
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
    use super::collect_custom_trace_attributes;
    use common::configuration::SpanAttributes;
    use hyper::header::{HeaderMap, HeaderValue};
    use std::collections::HashMap;

    #[test]
    fn extracts_headers_by_prefix() {
        let mut headers = HeaderMap::new();
        headers.insert("x-katanemo-tenant-id", HeaderValue::from_static("ten_456"));
        headers.insert("x-katanemo-user-id", HeaderValue::from_static("usr_789"));
        headers.insert("x-katanemo-admin-level", HeaderValue::from_static("3"));
        headers.insert("x-other-id", HeaderValue::from_static("ignored"));

        let attrs = collect_custom_trace_attributes(
            &headers,
            Some(&SpanAttributes {
                header_prefixes: Some(vec!["x-katanemo-".to_string()]),
                static_attributes: None,
            }),
        );

        assert_eq!(attrs.get("tenant.id"), Some(&"ten_456".to_string()));
        assert_eq!(attrs.get("user.id"), Some(&"usr_789".to_string()));
        assert_eq!(attrs.get("admin.level"), Some(&"3".to_string()));
        assert!(!attrs.contains_key("other.id"));
    }

    #[test]
    fn returns_empty_when_prefixes_missing_or_empty() {
        let mut headers = HeaderMap::new();
        headers.insert("x-katanemo-tenant-id", HeaderValue::from_static("ten_456"));

        let attrs_none = collect_custom_trace_attributes(
            &headers,
            Some(&SpanAttributes {
                header_prefixes: None,
                static_attributes: None,
            }),
        );
        assert!(attrs_none.is_empty());

        let attrs_empty = collect_custom_trace_attributes(
            &headers,
            Some(&SpanAttributes {
                header_prefixes: Some(Vec::new()),
                static_attributes: None,
            }),
        );
        assert!(attrs_empty.is_empty());
    }

    #[test]
    fn supports_multiple_prefixes() {
        let mut headers = HeaderMap::new();
        headers.insert("x-katanemo-tenant-id", HeaderValue::from_static("ten_456"));
        headers.insert("x-tenant-user-id", HeaderValue::from_static("usr_789"));

        let attrs = collect_custom_trace_attributes(
            &headers,
            Some(&SpanAttributes {
                header_prefixes: Some(vec!["x-katanemo-".to_string(), "x-tenant-".to_string()]),
                static_attributes: None,
            }),
        );

        assert_eq!(attrs.get("tenant.id"), Some(&"ten_456".to_string()));
        assert_eq!(attrs.get("user.id"), Some(&"usr_789".to_string()));
    }

    #[test]
    fn header_attributes_override_static_attributes() {
        let mut headers = HeaderMap::new();
        headers.insert("x-katanemo-tenant-id", HeaderValue::from_static("ten_456"));

        let mut static_attributes = HashMap::new();
        static_attributes.insert("tenant.id".to_string(), "ten_static".to_string());
        static_attributes.insert("environment".to_string(), "prod".to_string());

        let attrs = collect_custom_trace_attributes(
            &headers,
            Some(&SpanAttributes {
                header_prefixes: Some(vec!["x-katanemo-".to_string()]),
                static_attributes: Some(static_attributes),
            }),
        );

        assert_eq!(attrs.get("tenant.id"), Some(&"ten_456".to_string()));
        assert_eq!(attrs.get("environment"), Some(&"prod".to_string()));
    }
}
