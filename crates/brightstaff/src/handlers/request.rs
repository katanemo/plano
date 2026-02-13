use hyper::Request;

/// Extract request ID from incoming request headers, or generate a new UUID v4.
pub fn extract_request_id<T>(request: &Request<T>) -> String {
    request
        .headers()
        .get(common::consts::REQUEST_ID_HEADER)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
}
