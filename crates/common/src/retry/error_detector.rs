use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use hyper::Response;

use crate::configuration::{
    LatencyMeasure, RetryPolicy, RetryStrategy, StatusCodeEntry,
};

// ── Types ──────────────────────────────────────────────────────────────────

/// Represents a request timeout (used in P1).
#[derive(Debug)]
pub struct TimeoutError {
    pub duration_ms: u64,
}

/// The HTTP response type used throughout the gateway.
pub type HttpResponse = Response<BoxBody<Bytes, hyper::Error>>;

/// Result of classifying an upstream response or error condition.
#[derive(Debug)]
pub enum ErrorClassification {
    /// 2xx success — pass through to client.
    Success(HttpResponse),
    /// Retriable HTTP error (matched on_status_codes or default 4xx/5xx).
    RetriableError {
        status_code: u16,
        retry_after_seconds: Option<u64>,
        response_body: Vec<u8>,
    },
    /// Request timed out (P1 — variant defined now for forward compatibility).
    TimeoutError { duration_ms: u64 },
    /// Response latency exceeded threshold (P2 — variant defined for forward compat).
    HighLatencyEvent {
        measured_ms: u64,
        threshold_ms: u64,
        measure: LatencyMeasure,
        response: Option<HttpResponse>,
    },
    /// Non-retriable error — return as-is to client.
    NonRetriableError(HttpResponse),
}

// ── ErrorDetector ──────────────────────────────────────────────────────────

pub struct ErrorDetector;

impl ErrorDetector {
    /// Classify an upstream response or error condition.
    ///
    /// In P0, only handles the `Ok(response)` path for HTTP status codes.
    /// The `Err(timeout)` path is added in P1.
    ///
    /// Dual-classification for timeout + high latency:
    /// When both `on_high_latency` and `on_timeout` are configured and a request
    /// times out after exceeding `threshold_ms`, this returns `TimeoutError` (for
    /// retry purposes) but the caller must ALSO record a `HighLatencyEvent` for
    /// blocking purposes.
    pub fn classify(
        &self,
        response: Result<HttpResponse, TimeoutError>,
        retry_policy: &RetryPolicy,
        elapsed_ttfb_ms: u64,
        elapsed_total_ms: u64,
    ) -> ErrorClassification {
        match response {
            Ok(resp) => self.classify_http_response(resp, retry_policy, elapsed_ttfb_ms, elapsed_total_ms),
            // Timeout takes priority for retry; caller handles dual-classification
            // for blocking (records HighLatencyEvent separately if applicable).
            Err(timeout) => ErrorClassification::TimeoutError {
                duration_ms: timeout.duration_ms,
            },
        }
    }

    /// Determine retry strategy and max_attempts for a given classification.
    ///
    /// - `RetriableError` with a matching `on_status_codes` entry → that entry's params
    /// - `RetriableError` without a match (default 4xx/5xx) → (default_strategy, default_max_attempts)
    /// - `TimeoutError` → `on_timeout` config or defaults
    /// - `HighLatencyEvent` → `on_high_latency` config (strategy, max_attempts)
    pub fn resolve_retry_params(
        &self,
        classification: &ErrorClassification,
        retry_policy: &RetryPolicy,
    ) -> (RetryStrategy, u32) {
        match classification {
            ErrorClassification::RetriableError { status_code, .. } => {
                // Try to find a matching on_status_codes entry
                for entry in &retry_policy.on_status_codes {
                    if status_code_matches(*status_code, &entry.codes) {
                        return (entry.strategy, entry.max_attempts);
                    }
                }
                // No specific match — use defaults
                (retry_policy.default_strategy, retry_policy.default_max_attempts)
            }
            ErrorClassification::TimeoutError { .. } => {
                match &retry_policy.on_timeout {
                    Some(timeout_config) => {
                        (timeout_config.strategy, timeout_config.max_attempts)
                    }
                    None => (retry_policy.default_strategy, retry_policy.default_max_attempts),
                }
            }
            ErrorClassification::HighLatencyEvent { .. } => {
                match &retry_policy.on_high_latency {
                    Some(hl_config) => (hl_config.strategy, hl_config.max_attempts),
                    // Shouldn't happen (HighLatencyEvent only created when config exists),
                    // but fall back to defaults for safety.
                    None => (retry_policy.default_strategy, retry_policy.default_max_attempts),
                }
            }
            // Success and NonRetriableError should not be passed here,
            // but return defaults as a safe fallback.
            _ => (retry_policy.default_strategy, retry_policy.default_max_attempts),
        }
    }

    // ── Private helpers ────────────────────────────────────────────────────

    fn classify_http_response(
        &self,
        response: HttpResponse,
        retry_policy: &RetryPolicy,
        elapsed_ttfb_ms: u64,
        elapsed_total_ms: u64,
    ) -> ErrorClassification {
        let status = response.status().as_u16();

        // 2xx → check for high latency, otherwise Success
        if (200..300).contains(&status) {
            // If on_high_latency is configured, check if the response was slow
            if let Some(hl_config) = &retry_policy.on_high_latency {
                let measured_ms = match hl_config.measure {
                    LatencyMeasure::Ttfb => elapsed_ttfb_ms,
                    LatencyMeasure::Total => elapsed_total_ms,
                };
                if measured_ms > hl_config.threshold_ms {
                    return ErrorClassification::HighLatencyEvent {
                        measured_ms,
                        threshold_ms: hl_config.threshold_ms,
                        measure: hl_config.measure,
                        response: Some(response), // completed-but-slow: include the response
                    };
                }
            }
            return ErrorClassification::Success(response);
        }

        // Check if this status code is retriable (4xx or 5xx)
        let is_4xx = (400..500).contains(&status);
        let is_5xx = (500..600).contains(&status);

        if is_4xx || is_5xx {
            // Check if it matches any on_status_codes entry, OR fall back to
            // default handling for all 4xx/5xx when retry_policy exists.
            let has_specific_match = retry_policy
                .on_status_codes
                .iter()
                .any(|entry| status_code_matches(status, &entry.codes));

            if has_specific_match || is_4xx || is_5xx {
                // Extract Retry-After header (P1 will use this; capture it now)
                let retry_after_seconds = extract_retry_after(&response);

                // We need the response body for the error record.
                // Since we can't easily consume the body from a BoxBody synchronously,
                // store an empty body for now — the orchestrator will handle body capture.
                return ErrorClassification::RetriableError {
                    status_code: status,
                    retry_after_seconds,
                    response_body: Vec::new(),
                };
            }
        }

        // Non-2xx, non-4xx, non-5xx (e.g. 3xx, 1xx) → NonRetriableError
        ErrorClassification::NonRetriableError(response)
    }
}

// ── Free functions ─────────────────────────────────────────────────────────

/// Check if a status code matches any entry in a codes list.
fn status_code_matches(status: u16, codes: &[StatusCodeEntry]) -> bool {
    for entry in codes {
        match entry.expand() {
            Ok(expanded) => {
                if expanded.contains(&status) {
                    return true;
                }
            }
            Err(_) => continue, // Skip malformed ranges
        }
    }
    false
}

/// Extract the Retry-After header value as seconds.
/// Parses integer seconds only; ignores malformed values.
fn extract_retry_after(response: &HttpResponse) -> Option<u64> {
    response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
}

