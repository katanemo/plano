use std::collections::HashMap;

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
}
