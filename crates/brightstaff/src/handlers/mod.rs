pub mod agents;
pub mod debug;
pub mod function_calling;
pub mod llm;
pub mod models;
pub mod response;
pub mod routing_service;

#[cfg(test)]
mod integration_tests;

use bytes::Bytes;
use common::consts::TRACE_PARENT_HEADER;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty, Full};
use hyper::Request;
use tracing::warn;

/// Wrap a chunk into a `BoxBody` for hyper responses.
pub fn full<T: Into<Bytes>>(chunk: T) -> BoxBody<Bytes, hyper::Error> {
    Full::new(chunk.into())
        .map_err(|never| match never {})
        .boxed()
}

/// An empty HTTP body (used for 404 / OPTIONS responses).
pub fn empty() -> BoxBody<Bytes, hyper::Error> {
    Empty::<Bytes>::new()
        .map_err(|never| match never {})
        .boxed()
}

/// Extract request ID from incoming request headers, or generate a new UUID v4.
pub fn extract_request_id<T>(request: &Request<T>) -> String {
    request
        .headers()
        .get(common::consts::REQUEST_ID_HEADER)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
}

/// Extract or generate a W3C `traceparent` header value.
pub fn extract_or_generate_traceparent(headers: &hyper::HeaderMap) -> String {
    headers
        .get(TRACE_PARENT_HEADER)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            let trace_id = uuid::Uuid::new_v4().to_string().replace("-", "");
            let tp = format!("00-{}-0000000000000000-01", trace_id);
            warn!(
                generated_traceparent = %tp,
                "TRACE_PARENT header missing, generated new traceparent"
            );
            tp
        })
}
