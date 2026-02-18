use std::collections::HashMap;

use common::configuration::SpanAttributes;
use common::traces::SpanBuilder;
use hyper::header::HeaderMap;

pub fn extract_custom_trace_attributes(
    headers: &HeaderMap,
    span_attribute_header_prefixes: Option<&[String]>,
) -> HashMap<String, String> {
    let mut attributes = HashMap::new();
    let Some(span_attribute_header_prefixes) = span_attribute_header_prefixes else {
        return attributes;
    };
    if span_attribute_header_prefixes.is_empty() {
        return attributes;
    }

    for (name, value) in headers.iter() {
        let header_name = name.as_str();
        let mut matched_prefix: Option<&str> = None;
        for prefix in span_attribute_header_prefixes {
            if header_name.starts_with(prefix) {
                matched_prefix = Some(prefix.as_str());
                break;
            }
        }
        let Some(prefix) = matched_prefix else {
            continue;
        };

        let raw_value = match value.to_str().ok() {
            Some(value) => value.trim(),
            None => continue,
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

    attributes.extend(extract_custom_trace_attributes(
        headers,
        span_attributes.header_prefixes.as_deref(),
    ));
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
    use super::extract_custom_trace_attributes;
    use hyper::header::{HeaderMap, HeaderValue};

    #[test]
    fn extracts_headers_by_prefix() {
        let mut headers = HeaderMap::new();
        headers.insert("x-katanemo-tenant-id", HeaderValue::from_static("ten_456"));
        headers.insert("x-katanemo-user-id", HeaderValue::from_static("usr_789"));
        headers.insert("x-katanemo-admin-level", HeaderValue::from_static("3"));
        headers.insert("x-other-id", HeaderValue::from_static("ignored"));

        let prefixes = vec!["x-katanemo-".to_string()];
        let attrs = extract_custom_trace_attributes(&headers, Some(&prefixes));

        assert_eq!(attrs.get("tenant.id"), Some(&"ten_456".to_string()));
        assert_eq!(attrs.get("user.id"), Some(&"usr_789".to_string()));
        assert_eq!(attrs.get("admin.level"), Some(&"3".to_string()));
        assert!(!attrs.contains_key("other.id"));
    }

    #[test]
    fn returns_empty_when_prefixes_missing_or_empty() {
        let mut headers = HeaderMap::new();
        headers.insert("x-katanemo-tenant-id", HeaderValue::from_static("ten_456"));

        let attrs_none = extract_custom_trace_attributes(&headers, None);
        assert!(attrs_none.is_empty());

        let empty_prefixes: Vec<String> = Vec::new();
        let attrs_empty = extract_custom_trace_attributes(&headers, Some(&empty_prefixes));
        assert!(attrs_empty.is_empty());
    }

    #[test]
    fn supports_multiple_prefixes() {
        let mut headers = HeaderMap::new();
        headers.insert("x-katanemo-tenant-id", HeaderValue::from_static("ten_456"));
        headers.insert("x-tenant-user-id", HeaderValue::from_static("usr_789"));

        let prefixes = vec!["x-katanemo-".to_string(), "x-tenant-".to_string()];
        let attrs = extract_custom_trace_attributes(&headers, Some(&prefixes));

        assert_eq!(attrs.get("tenant.id"), Some(&"ten_456".to_string()));
        assert_eq!(attrs.get("user.id"), Some(&"usr_789".to_string()));
    }
}
