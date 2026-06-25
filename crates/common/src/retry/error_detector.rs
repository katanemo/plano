use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use hyper::Response;

use crate::configuration::{LatencyMeasure, RetryPolicy, RetryStrategy, StatusCodeEntry};

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
            Ok(resp) => {
                self.classify_http_response(resp, retry_policy, elapsed_ttfb_ms, elapsed_total_ms)
            }
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
                (
                    retry_policy.default_strategy,
                    retry_policy.default_max_attempts,
                )
            }
            ErrorClassification::TimeoutError { .. } => match &retry_policy.on_timeout {
                Some(timeout_config) => (timeout_config.strategy, timeout_config.max_attempts),
                None => (
                    retry_policy.default_strategy,
                    retry_policy.default_max_attempts,
                ),
            },
            ErrorClassification::HighLatencyEvent { .. } => {
                match &retry_policy.on_high_latency {
                    Some(hl_config) => (hl_config.strategy, hl_config.max_attempts),
                    // Shouldn't happen (HighLatencyEvent only created when config exists),
                    // but fall back to defaults for safety.
                    None => (
                        retry_policy.default_strategy,
                        retry_policy.default_max_attempts,
                    ),
                }
            }
            // Success and NonRetriableError should not be passed here,
            // but return defaults as a safe fallback.
            _ => (
                retry_policy.default_strategy,
                retry_policy.default_max_attempts,
            ),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::{StatusCodeConfig, TimeoutRetryConfig};
    use bytes::Bytes;
    use http_body_util::{BodyExt, Full};

    /// Helper to build an HttpResponse with a given status code.
    fn make_response(status: u16) -> HttpResponse {
        make_response_with_headers(status, vec![])
    }

    /// Helper to build an HttpResponse with a given status code and headers.
    fn make_response_with_headers(status: u16, headers: Vec<(&str, &str)>) -> HttpResponse {
        let body = Full::new(Bytes::from("test body"))
            .map_err(|_| unreachable!())
            .boxed();
        let mut builder = Response::builder().status(status);
        for (name, value) in headers {
            builder = builder.header(name, value);
        }
        builder.body(body).unwrap()
    }

    fn basic_retry_policy() -> RetryPolicy {
        RetryPolicy {
            fallback_models: vec![],
            default_strategy: RetryStrategy::DifferentProvider,
            default_max_attempts: 2,
            on_status_codes: vec![
                StatusCodeConfig {
                    codes: vec![StatusCodeEntry::Single(429)],
                    strategy: RetryStrategy::SameProvider,
                    max_attempts: 3,
                },
                StatusCodeConfig {
                    codes: vec![StatusCodeEntry::Single(503)],
                    strategy: RetryStrategy::DifferentProvider,
                    max_attempts: 4,
                },
            ],
            on_timeout: Some(TimeoutRetryConfig {
                strategy: RetryStrategy::DifferentProvider,
                max_attempts: 2,
            }),
            on_high_latency: None,
            backoff: None,
            retry_after_handling: None,
            max_retry_duration_ms: None,
        }
    }

    // ── classify tests ─────────────────────────────────────────────────

    #[test]
    fn classify_2xx_returns_success() {
        let detector = ErrorDetector;
        let policy = basic_retry_policy();
        let resp = make_response(200);
        let result = detector.classify(Ok(resp), &policy, 0, 0);
        assert!(matches!(result, ErrorClassification::Success(_)));
    }

    #[test]
    fn classify_201_returns_success() {
        let detector = ErrorDetector;
        let policy = basic_retry_policy();
        let resp = make_response(201);
        let result = detector.classify(Ok(resp), &policy, 0, 0);
        assert!(matches!(result, ErrorClassification::Success(_)));
    }

    #[test]
    fn classify_429_returns_retriable_error() {
        let detector = ErrorDetector;
        let policy = basic_retry_policy();
        let resp = make_response(429);
        let result = detector.classify(Ok(resp), &policy, 0, 0);
        match result {
            ErrorClassification::RetriableError { status_code, .. } => {
                assert_eq!(status_code, 429);
            }
            other => panic!("Expected RetriableError, got {:?}", other),
        }
    }

    #[test]
    fn classify_503_returns_retriable_error() {
        let detector = ErrorDetector;
        let policy = basic_retry_policy();
        let resp = make_response(503);
        let result = detector.classify(Ok(resp), &policy, 0, 0);
        match result {
            ErrorClassification::RetriableError { status_code, .. } => {
                assert_eq!(status_code, 503);
            }
            other => panic!("Expected RetriableError, got {:?}", other),
        }
    }

    #[test]
    fn classify_unconfigured_4xx_returns_retriable_with_defaults() {
        let detector = ErrorDetector;
        let policy = basic_retry_policy();
        let resp = make_response(400);
        let result = detector.classify(Ok(resp), &policy, 0, 0);
        match result {
            ErrorClassification::RetriableError { status_code, .. } => {
                assert_eq!(status_code, 400);
            }
            other => panic!(
                "Expected RetriableError for unconfigured 4xx, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn classify_unconfigured_5xx_returns_retriable_with_defaults() {
        let detector = ErrorDetector;
        let policy = basic_retry_policy();
        let resp = make_response(502);
        let result = detector.classify(Ok(resp), &policy, 0, 0);
        match result {
            ErrorClassification::RetriableError { status_code, .. } => {
                assert_eq!(status_code, 502);
            }
            other => panic!(
                "Expected RetriableError for unconfigured 5xx, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn classify_3xx_returns_non_retriable() {
        let detector = ErrorDetector;
        let policy = basic_retry_policy();
        let resp = make_response(301);
        let result = detector.classify(Ok(resp), &policy, 0, 0);
        assert!(matches!(result, ErrorClassification::NonRetriableError(_)));
    }

    #[test]
    fn classify_1xx_returns_non_retriable() {
        let detector = ErrorDetector;
        let policy = basic_retry_policy();
        let resp = make_response(100);
        let result = detector.classify(Ok(resp), &policy, 0, 0);
        assert!(matches!(result, ErrorClassification::NonRetriableError(_)));
    }

    #[test]
    fn classify_timeout_returns_timeout_error() {
        let detector = ErrorDetector;
        let policy = basic_retry_policy();
        let timeout = TimeoutError { duration_ms: 5000 };
        let result = detector.classify(Err(timeout), &policy, 0, 0);
        match result {
            ErrorClassification::TimeoutError { duration_ms } => {
                assert_eq!(duration_ms, 5000);
            }
            other => panic!("Expected TimeoutError, got {:?}", other),
        }
    }

    #[test]
    fn classify_extracts_retry_after_header() {
        let detector = ErrorDetector;
        let policy = basic_retry_policy();
        let resp = make_response_with_headers(429, vec![("retry-after", "120")]);
        let result = detector.classify(Ok(resp), &policy, 0, 0);
        match result {
            ErrorClassification::RetriableError {
                retry_after_seconds,
                ..
            } => {
                assert_eq!(retry_after_seconds, Some(120));
            }
            other => panic!("Expected RetriableError, got {:?}", other),
        }
    }

    #[test]
    fn classify_ignores_malformed_retry_after() {
        let detector = ErrorDetector;
        let policy = basic_retry_policy();
        let resp = make_response_with_headers(429, vec![("retry-after", "not-a-number")]);
        let result = detector.classify(Ok(resp), &policy, 0, 0);
        match result {
            ErrorClassification::RetriableError {
                retry_after_seconds,
                ..
            } => {
                assert_eq!(retry_after_seconds, None);
            }
            other => panic!("Expected RetriableError, got {:?}", other),
        }
    }

    #[test]
    fn classify_status_code_range() {
        let detector = ErrorDetector;
        let policy = RetryPolicy {
            on_status_codes: vec![StatusCodeConfig {
                codes: vec![StatusCodeEntry::Range("500-504".to_string())],
                strategy: RetryStrategy::DifferentProvider,
                max_attempts: 3,
            }],
            ..basic_retry_policy()
        };
        // 502 is within the range
        let resp = make_response(502);
        let result = detector.classify(Ok(resp), &policy, 0, 0);
        match result {
            ErrorClassification::RetriableError { status_code, .. } => {
                assert_eq!(status_code, 502);
            }
            other => panic!("Expected RetriableError, got {:?}", other),
        }
    }

    // ── resolve_retry_params tests ─────────────────────────────────────

    #[test]
    fn resolve_params_for_configured_status_code() {
        let detector = ErrorDetector;
        let policy = basic_retry_policy();
        let classification = ErrorClassification::RetriableError {
            status_code: 429,
            retry_after_seconds: None,
            response_body: vec![],
        };
        let (strategy, max_attempts) = detector.resolve_retry_params(&classification, &policy);
        assert_eq!(strategy, RetryStrategy::SameProvider);
        assert_eq!(max_attempts, 3);
    }

    #[test]
    fn resolve_params_for_unconfigured_status_code_uses_defaults() {
        let detector = ErrorDetector;
        let policy = basic_retry_policy();
        let classification = ErrorClassification::RetriableError {
            status_code: 400,
            retry_after_seconds: None,
            response_body: vec![],
        };
        let (strategy, max_attempts) = detector.resolve_retry_params(&classification, &policy);
        assert_eq!(strategy, RetryStrategy::DifferentProvider);
        assert_eq!(max_attempts, 2);
    }

    #[test]
    fn resolve_params_for_timeout_with_config() {
        let detector = ErrorDetector;
        let policy = basic_retry_policy();
        let classification = ErrorClassification::TimeoutError { duration_ms: 5000 };
        let (strategy, max_attempts) = detector.resolve_retry_params(&classification, &policy);
        assert_eq!(strategy, RetryStrategy::DifferentProvider);
        assert_eq!(max_attempts, 2);
    }

    #[test]
    fn resolve_params_for_timeout_without_config_uses_defaults() {
        let detector = ErrorDetector;
        let mut policy = basic_retry_policy();
        policy.on_timeout = None;
        let classification = ErrorClassification::TimeoutError { duration_ms: 5000 };
        let (strategy, max_attempts) = detector.resolve_retry_params(&classification, &policy);
        assert_eq!(strategy, RetryStrategy::DifferentProvider);
        assert_eq!(max_attempts, 2);
    }

    #[test]
    fn resolve_params_for_high_latency_with_config() {
        let detector = ErrorDetector;
        let mut policy = basic_retry_policy();
        policy.on_high_latency = Some(crate::configuration::HighLatencyConfig {
            threshold_ms: 5000,
            measure: LatencyMeasure::Ttfb,
            min_triggers: 1,
            trigger_window_seconds: None,
            strategy: RetryStrategy::SameProvider,
            max_attempts: 5,
            block_duration_seconds: 300,
            scope: crate::configuration::BlockScope::Model,
            apply_to: crate::configuration::ApplyTo::Global,
        });
        let classification = ErrorClassification::HighLatencyEvent {
            measured_ms: 6000,
            threshold_ms: 5000,
            measure: LatencyMeasure::Ttfb,
            response: None,
        };
        let (strategy, max_attempts) = detector.resolve_retry_params(&classification, &policy);
        assert_eq!(strategy, RetryStrategy::SameProvider);
        assert_eq!(max_attempts, 5);
    }

    #[test]
    fn resolve_params_for_success_returns_defaults() {
        let detector = ErrorDetector;
        let policy = basic_retry_policy();
        let resp = make_response(200);
        let classification = ErrorClassification::Success(resp);
        let (strategy, max_attempts) = detector.resolve_retry_params(&classification, &policy);
        // Shouldn't normally be called for Success, but returns defaults safely
        assert_eq!(strategy, RetryStrategy::DifferentProvider);
        assert_eq!(max_attempts, 2);
    }

    #[test]
    fn resolve_params_second_on_status_codes_entry() {
        let detector = ErrorDetector;
        let policy = basic_retry_policy();
        let classification = ErrorClassification::RetriableError {
            status_code: 503,
            retry_after_seconds: None,
            response_body: vec![],
        };
        let (strategy, max_attempts) = detector.resolve_retry_params(&classification, &policy);
        assert_eq!(strategy, RetryStrategy::DifferentProvider);
        assert_eq!(max_attempts, 4);
    }

    // ── High latency classification tests ─────────────────────────────

    fn high_latency_retry_policy(threshold_ms: u64, measure: LatencyMeasure) -> RetryPolicy {
        let mut policy = basic_retry_policy();
        policy.on_high_latency = Some(crate::configuration::HighLatencyConfig {
            threshold_ms,
            measure,
            min_triggers: 1,
            trigger_window_seconds: None,
            strategy: RetryStrategy::DifferentProvider,
            max_attempts: 2,
            block_duration_seconds: 300,
            scope: crate::configuration::BlockScope::Model,
            apply_to: crate::configuration::ApplyTo::Global,
        });
        policy
    }

    #[test]
    fn classify_2xx_high_latency_ttfb_returns_high_latency_event() {
        let detector = ErrorDetector;
        let policy = high_latency_retry_policy(5000, LatencyMeasure::Ttfb);
        let resp = make_response(200);
        // TTFB = 6000ms exceeds threshold of 5000ms
        let result = detector.classify(Ok(resp), &policy, 6000, 7000);
        match result {
            ErrorClassification::HighLatencyEvent {
                measured_ms,
                threshold_ms,
                measure,
                response,
            } => {
                assert_eq!(measured_ms, 6000);
                assert_eq!(threshold_ms, 5000);
                assert_eq!(measure, LatencyMeasure::Ttfb);
                assert!(response.is_some(), "Completed response should be present");
            }
            other => panic!("Expected HighLatencyEvent, got {:?}", other),
        }
    }

    #[test]
    fn classify_2xx_high_latency_total_returns_high_latency_event() {
        let detector = ErrorDetector;
        let policy = high_latency_retry_policy(5000, LatencyMeasure::Total);
        let resp = make_response(200);
        // Total = 8000ms exceeds threshold, TTFB = 3000ms does not
        let result = detector.classify(Ok(resp), &policy, 3000, 8000);
        match result {
            ErrorClassification::HighLatencyEvent {
                measured_ms,
                threshold_ms,
                measure,
                ..
            } => {
                assert_eq!(measured_ms, 8000);
                assert_eq!(threshold_ms, 5000);
                assert_eq!(measure, LatencyMeasure::Total);
            }
            other => panic!("Expected HighLatencyEvent, got {:?}", other),
        }
    }

    #[test]
    fn classify_2xx_below_threshold_returns_success() {
        let detector = ErrorDetector;
        let policy = high_latency_retry_policy(5000, LatencyMeasure::Ttfb);
        let resp = make_response(200);
        // TTFB = 3000ms is below threshold of 5000ms
        let result = detector.classify(Ok(resp), &policy, 3000, 4000);
        assert!(matches!(result, ErrorClassification::Success(_)));
    }

    #[test]
    fn classify_2xx_at_threshold_returns_success() {
        let detector = ErrorDetector;
        let policy = high_latency_retry_policy(5000, LatencyMeasure::Ttfb);
        let resp = make_response(200);
        // TTFB = 5000ms equals threshold — not exceeded
        let result = detector.classify(Ok(resp), &policy, 5000, 6000);
        assert!(matches!(result, ErrorClassification::Success(_)));
    }

    #[test]
    fn classify_2xx_no_high_latency_config_returns_success() {
        let detector = ErrorDetector;
        let policy = basic_retry_policy(); // no on_high_latency
        let resp = make_response(200);
        // High latency values but no config → Success
        let result = detector.classify(Ok(resp), &policy, 99999, 99999);
        assert!(matches!(result, ErrorClassification::Success(_)));
    }

    #[test]
    fn classify_timeout_takes_priority_over_high_latency() {
        let detector = ErrorDetector;
        let policy = high_latency_retry_policy(5000, LatencyMeasure::Ttfb);
        let timeout = TimeoutError { duration_ms: 10000 };
        // Even with high latency config, timeout returns TimeoutError
        let result = detector.classify(Err(timeout), &policy, 10000, 10000);
        match result {
            ErrorClassification::TimeoutError { duration_ms } => {
                assert_eq!(duration_ms, 10000);
            }
            other => panic!("Expected TimeoutError, got {:?}", other),
        }
    }

    #[test]
    fn classify_4xx_not_affected_by_high_latency() {
        let detector = ErrorDetector;
        let policy = high_latency_retry_policy(5000, LatencyMeasure::Ttfb);
        let resp = make_response(429);
        // Even with high latency, 4xx is still RetriableError
        let result = detector.classify(Ok(resp), &policy, 6000, 7000);
        assert!(matches!(
            result,
            ErrorClassification::RetriableError {
                status_code: 429,
                ..
            }
        ));
    }

    // ── P2 Edge Case: measure-specific classification tests ────────────

    #[test]
    fn classify_ttfb_measure_triggers_on_slow_ttfb_even_if_total_is_fast() {
        let detector = ErrorDetector;
        // measure: ttfb, threshold: 5000ms
        let policy = high_latency_retry_policy(5000, LatencyMeasure::Ttfb);
        let resp = make_response(200);
        // TTFB = 6000ms exceeds threshold, but total = 4000ms is below threshold
        let result = detector.classify(Ok(resp), &policy, 6000, 4000);
        match result {
            ErrorClassification::HighLatencyEvent {
                measured_ms,
                threshold_ms,
                measure,
                response,
            } => {
                assert_eq!(measured_ms, 6000, "Should measure TTFB, not total");
                assert_eq!(threshold_ms, 5000);
                assert_eq!(measure, LatencyMeasure::Ttfb);
                assert!(response.is_some(), "Completed response should be present");
            }
            other => panic!("Expected HighLatencyEvent for slow TTFB, got {:?}", other),
        }
    }

    #[test]
    fn classify_total_measure_does_not_trigger_when_only_ttfb_is_slow() {
        let detector = ErrorDetector;
        // measure: total, threshold: 5000ms
        let policy = high_latency_retry_policy(5000, LatencyMeasure::Total);
        let resp = make_response(200);
        // TTFB = 8000ms is slow, but total = 4000ms is below threshold
        // With measure: "total", only total time matters
        let result = detector.classify(Ok(resp), &policy, 8000, 4000);
        assert!(
            matches!(result, ErrorClassification::Success(_)),
            "measure: total should NOT trigger when only TTFB is slow but total is below threshold, got {:?}",
            result
        );
    }

    #[test]
    fn classify_total_measure_triggers_on_slow_total_even_if_ttfb_is_fast() {
        let detector = ErrorDetector;
        // measure: total, threshold: 5000ms
        let policy = high_latency_retry_policy(5000, LatencyMeasure::Total);
        let resp = make_response(200);
        // TTFB = 1000ms is fast, total = 7000ms exceeds threshold
        let result = detector.classify(Ok(resp), &policy, 1000, 7000);
        match result {
            ErrorClassification::HighLatencyEvent {
                measured_ms,
                threshold_ms,
                measure,
                response,
            } => {
                assert_eq!(measured_ms, 7000, "Should measure total, not TTFB");
                assert_eq!(threshold_ms, 5000);
                assert_eq!(measure, LatencyMeasure::Total);
                assert!(response.is_some(), "Completed response should be present");
            }
            other => panic!("Expected HighLatencyEvent for slow total, got {:?}", other),
        }
    }

    // ── Property-based tests ───────────────────────────────────────────

    use proptest::prelude::*;

    /// Generate an arbitrary RetryStrategy.
    fn arb_retry_strategy() -> impl Strategy<Value = RetryStrategy> {
        prop_oneof![
            Just(RetryStrategy::SameModel),
            Just(RetryStrategy::SameProvider),
            Just(RetryStrategy::DifferentProvider),
        ]
    }

    /// Generate an arbitrary StatusCodeEntry (single code in 100-599).
    fn arb_status_code_entry() -> impl Strategy<Value = StatusCodeEntry> {
        (100u16..=599u16).prop_map(StatusCodeEntry::Single)
    }

    /// Generate an arbitrary StatusCodeConfig with 1-5 single status code entries.
    fn arb_status_code_config() -> impl Strategy<Value = StatusCodeConfig> {
        (
            proptest::collection::vec(arb_status_code_entry(), 1..=5),
            arb_retry_strategy(),
            1u32..=10u32,
        )
            .prop_map(|(codes, strategy, max_attempts)| StatusCodeConfig {
                codes,
                strategy,
                max_attempts,
            })
    }

    /// Generate an arbitrary RetryPolicy with 0-3 on_status_codes entries.
    fn arb_retry_policy() -> impl Strategy<Value = RetryPolicy> {
        (
            arb_retry_strategy(),
            1u32..=10u32,
            proptest::collection::vec(arb_status_code_config(), 0..=3),
        )
            .prop_map(
                |(default_strategy, default_max_attempts, on_status_codes)| RetryPolicy {
                    fallback_models: vec![],
                    default_strategy,
                    default_max_attempts,
                    on_status_codes,
                    on_timeout: None,
                    on_high_latency: None,
                    backoff: None,
                    retry_after_handling: None,
                    max_retry_duration_ms: None,
                },
            )
    }

    // Feature: retry-on-ratelimit, Property 5: Error Classification Correctness
    // **Validates: Requirements 1.2**
    proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(100))]

        /// Property 5: For any status code in 100-599 and any RetryPolicy,
        /// classify() returns the correct variant:
        ///   2xx → Success
        ///   4xx/5xx → RetriableError with matching status_code
        ///   1xx/3xx → NonRetriableError
        #[test]
        fn prop_error_classification_correctness(
            status_code in 100u16..=599u16,
            policy in arb_retry_policy(),
        ) {
            let detector = ErrorDetector;
            let resp = make_response(status_code);
            let result = detector.classify(Ok(resp), &policy, 0, 0);

            match status_code {
                200..=299 => {
                    prop_assert!(
                        matches!(result, ErrorClassification::Success(_)),
                        "Expected Success for status {}, got {:?}", status_code, result
                    );
                }
                400..=499 | 500..=599 => {
                    match &result {
                        ErrorClassification::RetriableError { status_code: sc, .. } => {
                            prop_assert_eq!(
                                *sc, status_code,
                                "RetriableError status_code mismatch: expected {}, got {}", status_code, sc
                            );
                        }
                        other => {
                            prop_assert!(false, "Expected RetriableError for status {}, got {:?}", status_code, other);
                        }
                    }
                }
                100..=199 | 300..=399 => {
                    prop_assert!(
                        matches!(result, ErrorClassification::NonRetriableError(_)),
                        "Expected NonRetriableError for status {}, got {:?}", status_code, result
                    );
                }
                _ => {
                    // Should not happen given our range 100-599
                    prop_assert!(false, "Unexpected status code: {}", status_code);
                }
            }
        }
    }

    // Feature: retry-on-ratelimit, Property 17: Timeout vs High Latency Precedence
    // **Validates: Requirements 2.13, 2.14, 2.15, 2a.19, 2a.20**
    proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(100))]

        /// Property 17: When both on_high_latency and on_timeout are configured:
        /// - Timeout (Err) → always TimeoutError regardless of latency config
        /// - Completed 2xx exceeding threshold → HighLatencyEvent with response present
        /// - Completed 2xx below/at threshold → Success
        #[test]
        fn prop_timeout_vs_high_latency_precedence(
            threshold_ms in 1u64..=30_000u64,
            elapsed_ttfb_ms in 0u64..=60_000u64,
            elapsed_total_ms in 0u64..=60_000u64,
            timeout_duration_ms in 1u64..=60_000u64,
            measure_is_ttfb in proptest::bool::ANY,
            // 0 = timeout scenario, 1 = completed-above-threshold, 2 = completed-below-threshold
            scenario in 0u8..=2u8,
        ) {
            let measure = if measure_is_ttfb { LatencyMeasure::Ttfb } else { LatencyMeasure::Total };

            let mut policy = basic_retry_policy();
            policy.on_high_latency = Some(crate::configuration::HighLatencyConfig {
                threshold_ms,
                measure,
                min_triggers: 1,
                trigger_window_seconds: None,
                strategy: RetryStrategy::DifferentProvider,
                max_attempts: 2,
                block_duration_seconds: 300,
                scope: crate::configuration::BlockScope::Model,
                apply_to: crate::configuration::ApplyTo::Global,
            });
            // Ensure on_timeout is configured
            policy.on_timeout = Some(TimeoutRetryConfig {
                strategy: RetryStrategy::DifferentProvider,
                max_attempts: 2,
            });

            let detector = ErrorDetector;

            match scenario {
                0 => {
                    // Timeout scenario: Err(TimeoutError) → always TimeoutError
                    let timeout = TimeoutError { duration_ms: timeout_duration_ms };
                    let result = detector.classify(Err(timeout), &policy, elapsed_ttfb_ms, elapsed_total_ms);
                    match result {
                        ErrorClassification::TimeoutError { duration_ms } => {
                            prop_assert_eq!(duration_ms, timeout_duration_ms,
                                "TimeoutError duration should match input");
                        }
                        other => {
                            prop_assert!(false,
                                "Timeout should always produce TimeoutError, got {:?}", other);
                        }
                    }
                }
                1 => {
                    // Completed 2xx with latency ABOVE threshold → HighLatencyEvent
                    // Force the measured value to exceed threshold
                    let forced_ttfb = if measure_is_ttfb { threshold_ms + 1 + (elapsed_ttfb_ms % 30_000) } else { elapsed_ttfb_ms };
                    let forced_total = if !measure_is_ttfb { threshold_ms + 1 + (elapsed_total_ms % 30_000) } else { elapsed_total_ms };

                    let resp = make_response(200);
                    let result = detector.classify(Ok(resp), &policy, forced_ttfb, forced_total);
                    match result {
                        ErrorClassification::HighLatencyEvent {
                            measured_ms: actual_ms,
                            threshold_ms: actual_threshold,
                            measure: actual_measure,
                            response,
                        } => {
                            let expected_measured = if measure_is_ttfb { forced_ttfb } else { forced_total };
                            prop_assert_eq!(actual_ms, expected_measured,
                                "HighLatencyEvent measured_ms should match the selected measure");
                            prop_assert_eq!(actual_threshold, threshold_ms,
                                "HighLatencyEvent threshold_ms should match config");
                            prop_assert_eq!(actual_measure, measure,
                                "HighLatencyEvent measure should match config");
                            prop_assert!(response.is_some(),
                                "Completed response should be present in HighLatencyEvent");
                        }
                        other => {
                            prop_assert!(false,
                                "Completed 2xx above threshold should produce HighLatencyEvent, got {:?}", other);
                        }
                    }
                }
                2 => {
                    // Completed 2xx with latency AT or BELOW threshold → Success
                    // Force the measured value to be at or below threshold
                    let forced_ttfb = if measure_is_ttfb { threshold_ms.min(elapsed_ttfb_ms) } else { elapsed_ttfb_ms };
                    let forced_total = if !measure_is_ttfb { threshold_ms.min(elapsed_total_ms) } else { elapsed_total_ms };

                    let resp = make_response(200);
                    let result = detector.classify(Ok(resp), &policy, forced_ttfb, forced_total);
                    prop_assert!(
                        matches!(result, ErrorClassification::Success(_)),
                        "Completed 2xx at/below threshold should be Success, got {:?}", result
                    );
                }
                _ => {} // unreachable given range 0..=2
            }
        }
    }
}
