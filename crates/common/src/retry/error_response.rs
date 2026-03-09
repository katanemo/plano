use hyper::header::HeaderValue;
use hyper::Response;
use http_body_util::Full;
use bytes::Bytes;
use serde_json::json;

use super::{AttemptErrorType, RetryExhaustedError};

/// Build an HTTP response from a `RetryExhaustedError`.
///
/// The response body is a JSON object matching the design's error response format.
/// The HTTP status code is derived from the most recent attempt's error:
/// - For `HttpError`: the upstream status code
/// - For `Timeout` or `HighLatency`: 504 Gateway Timeout
///
/// The `request_id` is preserved in the `x-request-id` response header.
///
/// Optional fields `observed_max_retry_after_seconds` and
/// `shortest_remaining_block_seconds` are included only when their
/// corresponding values are `Some`.
pub fn build_error_response(
    error: &RetryExhaustedError,
    request_id: &str,
) -> Response<Full<Bytes>> {
    let status_code = determine_status_code(error);

    let attempts_json: Vec<serde_json::Value> = error
        .attempts
        .iter()
        .map(|a| {
            let error_type_str = match &a.error_type {
                AttemptErrorType::HttpError { status_code, .. } => {
                    format!("http_{}", status_code)
                }
                AttemptErrorType::Timeout { duration_ms } => {
                    format!("timeout_{}ms", duration_ms)
                }
                AttemptErrorType::HighLatency {
                    measured_ms,
                    threshold_ms,
                } => {
                    format!("high_latency_{}ms_threshold_{}ms", measured_ms, threshold_ms)
                }
            };
            json!({
                "model": a.model_id,
                "error_type": error_type_str,
                "attempt": a.attempt_number,
            })
        })
        .collect();

    let message = build_message(error);

    let mut error_obj = serde_json::Map::new();
    error_obj.insert("message".to_string(), json!(message));
    error_obj.insert("type".to_string(), json!("retry_exhausted"));
    error_obj.insert("attempts".to_string(), json!(attempts_json));
    error_obj.insert(
        "total_attempts".to_string(),
        json!(error.attempts.len()),
    );

    if let Some(max_ra) = error.max_retry_after_seconds {
        error_obj.insert(
            "observed_max_retry_after_seconds".to_string(),
            json!(max_ra),
        );
    }

    if let Some(shortest) = error.shortest_remaining_block_seconds {
        error_obj.insert(
            "shortest_remaining_block_seconds".to_string(),
            json!(shortest),
        );
    }

    error_obj.insert(
        "retry_budget_exhausted".to_string(),
        json!(error.retry_budget_exhausted),
    );

    let body_json = json!({ "error": error_obj });
    let body_bytes = serde_json::to_vec(&body_json).unwrap_or_default();

    let mut response = Response::builder()
        .status(status_code)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(body_bytes)))
        .unwrap();

    if let Ok(val) = HeaderValue::from_str(request_id) {
        response.headers_mut().insert("x-request-id", val);
    }

    response
}

/// Determine the HTTP status code from the most recent attempt error.
/// Returns 504 for timeouts and high latency exhaustion, otherwise the
/// upstream HTTP status code. Falls back to 502 if no attempts exist.
fn determine_status_code(error: &RetryExhaustedError) -> u16 {
    match error.attempts.last() {
        Some(last) => match &last.error_type {
            AttemptErrorType::HttpError { status_code, .. } => *status_code,
            AttemptErrorType::Timeout { .. } => 504,
            AttemptErrorType::HighLatency { .. } => 504,
        },
        None => 502,
    }
}

/// Build a human-readable message describing the exhaustion cause.
fn build_message(error: &RetryExhaustedError) -> String {
    if error.retry_budget_exhausted {
        return "All retry attempts exhausted: retry budget exceeded".to_string();
    }

    match error.attempts.last() {
        Some(last) => match &last.error_type {
            AttemptErrorType::Timeout { .. } => {
                "All retry attempts exhausted: upstream request timed out".to_string()
            }
            AttemptErrorType::HighLatency { .. } => {
                "All retry attempts exhausted: upstream high latency detected".to_string()
            }
            _ => "All retry attempts exhausted".to_string(),
        },
        None => "All retry attempts exhausted".to_string(),
    }
}

