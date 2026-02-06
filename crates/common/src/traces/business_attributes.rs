//! Helpers for enriching OTEL spans with business identifiers from HTTP headers.
//!
//! When `tracing.span_attribute_headers` is configured in arch_config (e.g. `X-Tenant-Id` → `tenant.id`),
//! request headers are copied onto spans so traces can be filtered by tenant, workspace, user, etc.

use std::collections::HashMap;

/// Builds a map of span attribute key → value from request headers using the configured mapping.
///
/// Header names are matched case-insensitively. Only headers present in the request and in the
/// mapping are included. Intended for business identifiers like tenant.id, workspace.id, user.id.
///
/// # Arguments
/// * `header_iter` - Iterator of (header_name, header_value) (e.g. from `request.headers().iter()`)
/// * `mapping` - Optional map from HTTP header name to span attribute key (from config)
///
/// # Example
/// Config: `span_attribute_headers: {"X-Tenant-Id": "tenant.id", "X-User-Id": "user.id"}`
/// Request header `X-Tenant-Id: acme` → span attribute `tenant.id` = `acme`
pub fn span_attributes_from_headers<K, V, I>(
    header_iter: I,
    mapping: Option<&HashMap<String, String>>,
) -> HashMap<String, String>
where
    K: AsRef<str>,
    V: AsRef<str>,
    I: Iterator<Item = (K, V)>,
{
    let Some(mapping) = mapping else {
        return HashMap::new();
    };
    if mapping.is_empty() {
        return HashMap::new();
    }

    // Normalize mapping keys to lowercase for case-insensitive header lookup
    let mapping_lower: HashMap<String, String> = mapping
        .iter()
        .map(|(k, v)| (k.to_lowercase(), v.clone()))
        .collect();

    let mut attrs = HashMap::new();
    for (name, value) in header_iter {
        let name_str = name.as_ref();
        let value_str = value.as_ref().trim();
        if value_str.is_empty() {
            continue;
        }
        if let Some(attr_key) = mapping_lower.get(&name_str.to_lowercase()) {
            attrs.insert(attr_key.clone(), value_str.to_string());
        }
    }
    attrs
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::iter;

    #[test]
    fn test_empty_mapping_returns_empty() {
        let headers = iter::once(("x-tenant-id", "acme"));
        let mapping: Option<HashMap<String, String>> = None;
        assert!(span_attributes_from_headers(headers, mapping.as_ref()).is_empty());
    }

    #[test]
    fn test_extracts_mapped_headers() {
        let headers = vec![
            ("X-Tenant-Id".to_string(), "acme".to_string()),
            ("X-Workspace-Id".to_string(), "ws-1".to_string()),
            ("X-User-Id".to_string(), "user-42".to_string()),
        ];
        let mapping = HashMap::from([
            ("X-Tenant-Id".to_string(), "tenant.id".to_string()),
            ("X-Workspace-Id".to_string(), "workspace.id".to_string()),
            ("X-User-Id".to_string(), "user.id".to_string()),
        ]);
        let attrs = span_attributes_from_headers(headers.into_iter(), Some(&mapping));
        assert_eq!(attrs.get("tenant.id"), Some(&"acme".to_string()));
        assert_eq!(attrs.get("workspace.id"), Some(&"ws-1".to_string()));
        assert_eq!(attrs.get("user.id"), Some(&"user-42".to_string()));
    }

    #[test]
    fn test_case_insensitive_header_match() {
        let headers = iter::once(("X-TENANT-ID", "acme"));
        let mapping = HashMap::from([("X-Tenant-Id".to_string(), "tenant.id".to_string())]);
        let attrs = span_attributes_from_headers(headers, Some(&mapping));
        assert_eq!(attrs.get("tenant.id"), Some(&"acme".to_string()));
    }

    #[test]
    fn test_skips_headers_not_in_mapping() {
        let headers = vec![
            ("X-Tenant-Id".to_string(), "acme".to_string()),
            ("X-Other".to_string(), "ignored".to_string()),
        ];
        let mapping = HashMap::from([("x-tenant-id".to_string(), "tenant.id".to_string())]);
        let attrs = span_attributes_from_headers(headers.into_iter(), Some(&mapping));
        assert_eq!(attrs.len(), 1);
        assert_eq!(attrs.get("tenant.id"), Some(&"acme".to_string()));
    }
}
