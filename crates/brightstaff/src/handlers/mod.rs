pub mod agents;
pub mod debug;
pub mod embeddings;
pub mod function_calling;
pub mod llm;
pub mod models;
pub mod response;
pub mod routing_budget_request;
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

/// Caller attribution for metering, taken from request headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribution {
    pub org: String,
    pub service: String,
    pub user: String,
    pub execution: String,
}

pub const SERVICE_HEADER: &str = "x-fikar-service";
pub const USER_HEADER: &str = "x-fikar-user";
pub const EXECUTION_HEADER: &str = "x-fikar-execution";

/// Resolve the caller's attribution for metering. The org comes from the configured
/// attribution header ([`normalize_org`]) and is stamped on the active span
/// (`plano.org`); service, user and execution come from `x-fikar-service`,
/// `x-fikar-user` and `x-fikar-execution`. Every header whose name starts with
/// `x-fikar-` (and the configured attribution header) is removed so the provider
/// never sees any of them, even when no attribution header is configured.
///
/// When no attribution header is configured the org is the "unattributed" label
/// and no span attribute is set.
///
/// [`normalize_org`]: crate::metrics::labels::normalize_org
pub fn take_attribution_from_headers(
    headers: &mut hyper::HeaderMap,
    attribution_header: Option<&str>,
) -> Attribution {
    use crate::metrics::labels as l;
    let get = |headers: &hyper::HeaderMap, name: &str| -> Option<String> {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let org = l::normalize_org(
        attribution_header
            .and_then(|h| headers.get(h))
            .and_then(|v| v.to_str().ok()),
    );
    let attribution = Attribution {
        org,
        service: l::normalize_service(get(headers, SERVICE_HEADER).as_deref()),
        user: l::normalize_user(get(headers, USER_HEADER).as_deref()),
        execution: l::normalize_execution(get(headers, EXECUTION_HEADER).as_deref()),
    };
    // HeaderName is always lowercase, so a prefix check is case-insensitive.
    let doomed: Vec<hyper::header::HeaderName> = headers
        .keys()
        .filter(|k| k.as_str().starts_with("x-fikar-"))
        .cloned()
        .collect();
    for k in doomed {
        headers.remove(k);
    }
    if let Some(header_name) = attribution_header {
        headers.remove(header_name);
        opentelemetry::trace::get_active_span(|span| {
            span.set_attribute(opentelemetry::KeyValue::new(
                crate::tracing::plano::ORG,
                attribution.org.clone(),
            ));
        });
    }
    attribution
}

/// Org-only form of [`take_attribution_from_headers`].
pub fn take_org_from_headers(
    headers: &mut hyper::HeaderMap,
    attribution_header: Option<&str>,
) -> String {
    take_attribution_from_headers(headers, attribution_header).org
}
