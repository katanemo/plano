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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retry::{AttemptError, AttemptErrorType, RetryExhaustedError};
    use http_body_util::BodyExt;
    use proptest::prelude::*;

    /// Helper to extract the JSON body from a response.
    async fn response_json(resp: Response<Full<Bytes>>) -> serde_json::Value {
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn test_basic_http_error_response() {
        let error = RetryExhaustedError {
            attempts: vec![
                AttemptError {
                    model_id: "openai/gpt-4o".to_string(),
                    error_type: AttemptErrorType::HttpError {
                        status_code: 429,
                        body: b"rate limited".to_vec(),
                    },
                    attempt_number: 1,
                },
                AttemptError {
                    model_id: "anthropic/claude-3-5-sonnet".to_string(),
                    error_type: AttemptErrorType::HttpError {
                        status_code: 503,
                        body: b"unavailable".to_vec(),
                    },
                    attempt_number: 2,
                },
            ],
            max_retry_after_seconds: Some(30),
            shortest_remaining_block_seconds: Some(12),
            retry_budget_exhausted: false,
        };

        let resp = build_error_response(&error, "req-123");
        assert_eq!(resp.status().as_u16(), 503); // most recent error
        assert_eq!(
            resp.headers().get("x-request-id").unwrap().to_str().unwrap(),
            "req-123"
        );
        assert_eq!(
            resp.headers().get("content-type").unwrap().to_str().unwrap(),
            "application/json"
        );

        let json = response_json(resp).await;
        let err = &json["error"];
        assert_eq!(err["type"], "retry_exhausted");
        assert_eq!(err["total_attempts"], 2);
        assert_eq!(err["observed_max_retry_after_seconds"], 30);
        assert_eq!(err["shortest_remaining_block_seconds"], 12);
        assert_eq!(err["retry_budget_exhausted"], false);

        let attempts = err["attempts"].as_array().unwrap();
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0]["model"], "openai/gpt-4o");
        assert_eq!(attempts[0]["error_type"], "http_429");
        assert_eq!(attempts[0]["attempt"], 1);
        assert_eq!(attempts[1]["model"], "anthropic/claude-3-5-sonnet");
        assert_eq!(attempts[1]["error_type"], "http_503");
        assert_eq!(attempts[1]["attempt"], 2);
    }

    #[tokio::test]
    async fn test_timeout_returns_504() {
        let error = RetryExhaustedError {
            attempts: vec![AttemptError {
                model_id: "openai/gpt-4o".to_string(),
                error_type: AttemptErrorType::Timeout { duration_ms: 30000 },
                attempt_number: 1,
            }],
            max_retry_after_seconds: None,
            shortest_remaining_block_seconds: None,
            retry_budget_exhausted: false,
        };

        let resp = build_error_response(&error, "req-timeout");
        assert_eq!(resp.status().as_u16(), 504);

        let json = response_json(resp).await;
        let err = &json["error"];
        assert_eq!(err["attempts"][0]["error_type"], "timeout_30000ms");
        assert!(err["message"]
            .as_str()
            .unwrap()
            .contains("timed out"));
    }

    #[tokio::test]
    async fn test_high_latency_returns_504() {
        let error = RetryExhaustedError {
            attempts: vec![AttemptError {
                model_id: "openai/gpt-4o".to_string(),
                error_type: AttemptErrorType::HighLatency {
                    measured_ms: 8000,
                    threshold_ms: 5000,
                },
                attempt_number: 1,
            }],
            max_retry_after_seconds: None,
            shortest_remaining_block_seconds: None,
            retry_budget_exhausted: false,
        };

        let resp = build_error_response(&error, "req-latency");
        assert_eq!(resp.status().as_u16(), 504);

        let json = response_json(resp).await;
        let err = &json["error"];
        assert_eq!(
            err["attempts"][0]["error_type"],
            "high_latency_8000ms_threshold_5000ms"
        );
        assert!(err["message"]
            .as_str()
            .unwrap()
            .contains("high latency"));
    }

    #[tokio::test]
    async fn test_optional_fields_omitted_when_none() {
        let error = RetryExhaustedError {
            attempts: vec![AttemptError {
                model_id: "openai/gpt-4o".to_string(),
                error_type: AttemptErrorType::HttpError {
                    status_code: 429,
                    body: vec![],
                },
                attempt_number: 1,
            }],
            max_retry_after_seconds: None,
            shortest_remaining_block_seconds: None,
            retry_budget_exhausted: false,
        };

        let resp = build_error_response(&error, "req-456");
        let json = response_json(resp).await;
        let err = &json["error"];

        // These fields should not be present
        assert!(err.get("observed_max_retry_after_seconds").is_none());
        assert!(err.get("shortest_remaining_block_seconds").is_none());

        // These should always be present
        assert!(err.get("retry_budget_exhausted").is_some());
        assert!(err.get("total_attempts").is_some());
        assert!(err.get("type").is_some());
        assert!(err.get("message").is_some());
        assert!(err.get("attempts").is_some());
    }

    #[tokio::test]
    async fn test_retry_budget_exhausted_message() {
        let error = RetryExhaustedError {
            attempts: vec![AttemptError {
                model_id: "openai/gpt-4o".to_string(),
                error_type: AttemptErrorType::HttpError {
                    status_code: 429,
                    body: vec![],
                },
                attempt_number: 1,
            }],
            max_retry_after_seconds: None,
            shortest_remaining_block_seconds: None,
            retry_budget_exhausted: true,
        };

        let resp = build_error_response(&error, "req-budget");
        let json = response_json(resp).await;
        let err = &json["error"];
        assert_eq!(err["retry_budget_exhausted"], true);
        assert!(err["message"]
            .as_str()
            .unwrap()
            .contains("budget exceeded"));
    }

    #[tokio::test]
    async fn test_empty_attempts_returns_502() {
        let error = RetryExhaustedError {
            attempts: vec![],
            max_retry_after_seconds: None,
            shortest_remaining_block_seconds: None,
            retry_budget_exhausted: false,
        };

        let resp = build_error_response(&error, "req-empty");
        assert_eq!(resp.status().as_u16(), 502);

        let json = response_json(resp).await;
        assert_eq!(json["error"]["total_attempts"], 0);
        assert_eq!(json["error"]["attempts"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_request_id_preserved_in_header() {
        let error = RetryExhaustedError {
            attempts: vec![AttemptError {
                model_id: "m".to_string(),
                error_type: AttemptErrorType::HttpError {
                    status_code: 500,
                    body: vec![],
                },
                attempt_number: 1,
            }],
            max_retry_after_seconds: None,
            shortest_remaining_block_seconds: None,
            retry_budget_exhausted: false,
        };

        let resp = build_error_response(&error, "unique-request-id-abc-123");
        assert_eq!(
            resp.headers()
                .get("x-request-id")
                .unwrap()
                .to_str()
                .unwrap(),
            "unique-request-id-abc-123"
        );
    }

    #[tokio::test]
    async fn test_mixed_error_types_in_attempts() {
        let error = RetryExhaustedError {
            attempts: vec![
                AttemptError {
                    model_id: "openai/gpt-4o".to_string(),
                    error_type: AttemptErrorType::HttpError {
                        status_code: 429,
                        body: vec![],
                    },
                    attempt_number: 1,
                },
                AttemptError {
                    model_id: "anthropic/claude".to_string(),
                    error_type: AttemptErrorType::Timeout { duration_ms: 5000 },
                    attempt_number: 2,
                },
                AttemptError {
                    model_id: "gemini/pro".to_string(),
                    error_type: AttemptErrorType::HighLatency {
                        measured_ms: 10000,
                        threshold_ms: 3000,
                    },
                    attempt_number: 3,
                },
            ],
            max_retry_after_seconds: Some(60),
            shortest_remaining_block_seconds: Some(5),
            retry_budget_exhausted: false,
        };

        // Last attempt is HighLatency → 504
        let resp = build_error_response(&error, "req-mixed");
        assert_eq!(resp.status().as_u16(), 504);

        let json = response_json(resp).await;
        let err = &json["error"];
        assert_eq!(err["total_attempts"], 3);
        assert_eq!(err["observed_max_retry_after_seconds"], 60);
        assert_eq!(err["shortest_remaining_block_seconds"], 5);

        let attempts = err["attempts"].as_array().unwrap();
        assert_eq!(attempts[0]["error_type"], "http_429");
        assert_eq!(attempts[1]["error_type"], "timeout_5000ms");
        assert_eq!(attempts[2]["error_type"], "high_latency_10000ms_threshold_3000ms");
    }

    // ── Proptest strategies ────────────────────────────────────────────────

    /// Generate an arbitrary AttemptErrorType.
    fn arb_attempt_error_type() -> impl Strategy<Value = AttemptErrorType> {
        prop_oneof![
            (100u16..=599u16, proptest::collection::vec(any::<u8>(), 0..32))
                .prop_map(|(status_code, body)| AttemptErrorType::HttpError { status_code, body }),
            (1u64..=120_000u64)
                .prop_map(|duration_ms| AttemptErrorType::Timeout { duration_ms }),
            (1u64..=120_000u64, 1u64..=120_000u64)
                .prop_map(|(measured_ms, threshold_ms)| AttemptErrorType::HighLatency {
                    measured_ms,
                    threshold_ms,
                }),
        ]
    }

    /// Generate an arbitrary AttemptError with a model_id from a small set of
    /// realistic provider/model identifiers.
    fn arb_attempt_error() -> impl Strategy<Value = AttemptError> {
        let model_ids = prop_oneof![
            Just("openai/gpt-4o".to_string()),
            Just("openai/gpt-4o-mini".to_string()),
            Just("anthropic/claude-3-5-sonnet".to_string()),
            Just("gemini/pro".to_string()),
            Just("azure/gpt-4o".to_string()),
        ];
        (model_ids, arb_attempt_error_type(), 1u32..=10u32).prop_map(
            |(model_id, error_type, attempt_number)| AttemptError {
                model_id,
                error_type,
                attempt_number,
            },
        )
    }

    /// Generate an arbitrary RetryExhaustedError with 1..=8 attempts.
    fn arb_retry_exhausted_error() -> impl Strategy<Value = RetryExhaustedError> {
        (
            proptest::collection::vec(arb_attempt_error(), 1..=8),
            proptest::option::of(1u64..=600u64),
            proptest::option::of(1u64..=600u64),
            any::<bool>(),
        )
            .prop_map(
                |(attempts, max_retry_after_seconds, shortest_remaining_block_seconds, retry_budget_exhausted)| {
                    RetryExhaustedError {
                        attempts,
                        max_retry_after_seconds,
                        shortest_remaining_block_seconds,
                        retry_budget_exhausted,
                    }
                },
            )
    }

    /// Generate an arbitrary request_id (non-empty ASCII string valid for HTTP headers).
    fn arb_request_id() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9_-]{1,64}"
    }

    // Feature: retry-on-ratelimit, Property 21: Error Response Contains Attempt Details
    // **Validates: Requirements 10.4, 10.5, 10.7**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// Property 21: For any exhausted retry sequence, the error response
        /// must include all attempted model identifiers and their error types,
        /// and must preserve the original request_id.
        #[test]
        fn prop_error_response_contains_attempt_details(
            error in arb_retry_exhausted_error(),
            request_id in arb_request_id(),
        ) {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async {
                let resp = build_error_response(&error, &request_id);

                // request_id preserved in x-request-id header
                let header_val = resp.headers().get("x-request-id")
                    .expect("x-request-id header must be present");
                prop_assert_eq!(header_val.to_str().unwrap(), request_id.as_str());

                // Content-Type is application/json
                let ct = resp.headers().get("content-type")
                    .expect("content-type header must be present");
                prop_assert_eq!(ct.to_str().unwrap(), "application/json");

                // Parse JSON body
                let body = resp.into_body().collect().await.unwrap().to_bytes();
                let json: serde_json::Value = serde_json::from_slice(&body)
                    .expect("response body must be valid JSON");

                let err_obj = &json["error"];

                // type is always "retry_exhausted"
                prop_assert_eq!(err_obj["type"].as_str().unwrap(), "retry_exhausted");

                // total_attempts matches input
                prop_assert_eq!(
                    err_obj["total_attempts"].as_u64().unwrap(),
                    error.attempts.len() as u64
                );

                // retry_budget_exhausted matches input
                prop_assert_eq!(
                    err_obj["retry_budget_exhausted"].as_bool().unwrap(),
                    error.retry_budget_exhausted
                );

                // attempts array has correct length
                let attempts_arr = err_obj["attempts"].as_array()
                    .expect("attempts must be an array");
                prop_assert_eq!(attempts_arr.len(), error.attempts.len());

                // Every attempt's model_id and error_type are present and correct
                for (i, attempt) in error.attempts.iter().enumerate() {
                    let json_attempt = &attempts_arr[i];

                    // model_id preserved
                    prop_assert_eq!(
                        json_attempt["model"].as_str().unwrap(),
                        attempt.model_id.as_str()
                    );

                    // attempt_number preserved
                    prop_assert_eq!(
                        json_attempt["attempt"].as_u64().unwrap(),
                        attempt.attempt_number as u64
                    );

                    // error_type string matches the variant
                    let error_type_str = json_attempt["error_type"].as_str().unwrap();
                    match &attempt.error_type {
                        AttemptErrorType::HttpError { status_code, .. } => {
                            prop_assert_eq!(
                                error_type_str,
                                &format!("http_{}", status_code)
                            );
                        }
                        AttemptErrorType::Timeout { duration_ms } => {
                            prop_assert_eq!(
                                error_type_str,
                                &format!("timeout_{}ms", duration_ms)
                            );
                        }
                        AttemptErrorType::HighLatency { measured_ms, threshold_ms } => {
                            prop_assert_eq!(
                                error_type_str,
                                &format!("high_latency_{}ms_threshold_{}ms", measured_ms, threshold_ms)
                            );
                        }
                    }
                }

                // Optional fields: observed_max_retry_after_seconds
                match error.max_retry_after_seconds {
                    Some(v) => {
                        prop_assert_eq!(
                            err_obj["observed_max_retry_after_seconds"].as_u64().unwrap(),
                            v
                        );
                    }
                    None => {
                        prop_assert!(err_obj.get("observed_max_retry_after_seconds").is_none()
                            || err_obj["observed_max_retry_after_seconds"].is_null());
                    }
                }

                // Optional fields: shortest_remaining_block_seconds
                match error.shortest_remaining_block_seconds {
                    Some(v) => {
                        prop_assert_eq!(
                            err_obj["shortest_remaining_block_seconds"].as_u64().unwrap(),
                            v
                        );
                    }
                    None => {
                        prop_assert!(err_obj.get("shortest_remaining_block_seconds").is_none()
                            || err_obj["shortest_remaining_block_seconds"].is_null());
                    }
                }

                // message is a non-empty string
                let message = err_obj["message"].as_str()
                    .expect("message must be a string");
                prop_assert!(!message.is_empty());

                Ok(())
            })?;
        }
    }
}
