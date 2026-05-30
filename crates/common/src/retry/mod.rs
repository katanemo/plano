use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use hyper::HeaderMap;
use sha2::{Digest, Sha256};

use crate::configuration::{ApplyTo, LlmProvider, LlmProviderType};

// Sub-modules
pub mod backoff;
pub mod error_detector;
pub mod error_response;
pub mod latency_block_state;
pub mod latency_trigger;
pub mod orchestrator;
pub mod provider_selector;
pub mod retry_after_state;
pub mod validation;

// ── State Structs ──────────────────────────────────────────────────────────

/// In-memory Retry-After state entry.
#[derive(Debug, Clone)]
pub struct RetryAfterEntry {
    pub identifier: String,
    pub expires_at: Instant,
    pub apply_to: ApplyTo,
}

/// In-memory Latency Block state entry.
#[derive(Debug, Clone)]
pub struct LatencyBlockEntry {
    pub identifier: String,
    pub expires_at: Instant,
    pub measured_latency_ms: u64,
    pub apply_to: ApplyTo,
}

/// Error accumulated from a single attempt.
#[derive(Debug, Clone)]
pub struct AttemptError {
    pub model_id: String,
    pub error_type: AttemptErrorType,
    pub attempt_number: u32,
}

#[derive(Debug, Clone)]
pub enum AttemptErrorType {
    HttpError { status_code: u16, body: Vec<u8> },
    Timeout { duration_ms: u64 },
    HighLatency { measured_ms: u64, threshold_ms: u64 },
}

/// Lightweight request signature for retry tracking.
/// The actual request body bytes are passed by reference from the handler scope
/// (as `&Bytes`) rather than cloned into this struct.
#[derive(Debug, Clone)]
pub struct RequestSignature {
    /// SHA-256 hash of the original request body
    pub body_hash: [u8; 32],
    pub headers: HeaderMap,
    pub streaming: bool,
    pub original_model: String,
}

impl RequestSignature {
    pub fn new(body: &[u8], headers: &HeaderMap, streaming: bool, original_model: String) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(body);
        let hash: [u8; 32] = hasher.finalize().into();
        Self {
            body_hash: hash,
            headers: headers.clone(),
            streaming,
            original_model,
        }
    }
}

// ── Auth Header Constants ───────────────────────────────────────────────────

/// Headers that carry authentication credentials and must be sanitized
/// when forwarding requests to a different provider.
const AUTH_HEADERS: &[&str] = &["authorization", "x-api-key"];

/// Additional provider-specific headers that should be sanitized.
const PROVIDER_SPECIFIC_HEADERS: &[&str] = &["anthropic-version"];

/// Rebuild a request for a different target provider.
///
/// Updates the `model` field in the JSON body to match the target provider's
/// model name (without provider prefix), and applies the correct auth
/// credentials for the target provider. Sanitizes auth headers from the
/// original request to prevent credential leakage across providers.
///
/// Returns the updated body bytes and headers, or an error if the body
/// cannot be parsed as JSON.
pub fn rebuild_request_for_provider(
    body: &Bytes,
    target_provider: &LlmProvider,
    original_headers: &HeaderMap,
) -> Result<(Bytes, HeaderMap), RebuildError> {
    // Update the model field in the JSON body
    let mut json_body: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| RebuildError::InvalidJson(e.to_string()))?;

    // Extract model name without provider prefix (e.g., "openai/gpt-4o" -> "gpt-4o")
    let target_model = target_provider
        .model
        .as_deref()
        .or(Some(&target_provider.name))
        .unwrap_or(&target_provider.name);
    let model_name_only = if let Some((_, model)) = target_model.split_once('/') {
        model
    } else {
        target_model
    };

    if let Some(obj) = json_body.as_object_mut() {
        obj.insert(
            "model".to_string(),
            serde_json::Value::String(model_name_only.to_string()),
        );
    }

    let updated_body = Bytes::from(
        serde_json::to_vec(&json_body).map_err(|e| RebuildError::InvalidJson(e.to_string()))?,
    );

    // Sanitize and rebuild headers
    let mut headers = sanitize_headers(original_headers);
    apply_auth_headers(&mut headers, target_provider)?;

    Ok((updated_body, headers))
}

/// Remove auth-related headers from the original request to prevent
/// credential leakage when forwarding to a different provider.
fn sanitize_headers(original: &HeaderMap) -> HeaderMap {
    let mut headers = original.clone();
    for header_name in AUTH_HEADERS.iter().chain(PROVIDER_SPECIFIC_HEADERS.iter()) {
        headers.remove(*header_name);
    }
    headers
}

/// Apply the correct auth headers for the target provider.
fn apply_auth_headers(headers: &mut HeaderMap, provider: &LlmProvider) -> Result<(), RebuildError> {
    // If passthrough_auth is enabled, don't set provider credentials
    if provider.passthrough_auth == Some(true) {
        return Ok(());
    }

    let access_key = provider
        .access_key
        .as_ref()
        .ok_or_else(|| RebuildError::MissingAccessKey(provider.name.clone()))?;

    match provider.provider_interface {
        LlmProviderType::Anthropic => {
            headers.insert(
                hyper::header::HeaderName::from_static("x-api-key"),
                hyper::header::HeaderValue::from_str(access_key)
                    .map_err(|_| RebuildError::InvalidHeaderValue("x-api-key".to_string()))?,
            );
            headers.insert(
                hyper::header::HeaderName::from_static("anthropic-version"),
                hyper::header::HeaderValue::from_static("2023-06-01"),
            );
        }
        _ => {
            // OpenAI-compatible providers use Authorization: Bearer <key>
            let bearer = format!("Bearer {}", access_key);
            headers.insert(
                hyper::header::AUTHORIZATION,
                hyper::header::HeaderValue::from_str(&bearer)
                    .map_err(|_| RebuildError::InvalidHeaderValue("authorization".to_string()))?,
            );
        }
    }

    Ok(())
}

/// Errors that can occur when rebuilding a request for a different provider.
#[derive(Debug, Clone, PartialEq)]
pub enum RebuildError {
    /// The request body is not valid JSON.
    InvalidJson(String),
    /// The target provider has no access_key configured.
    MissingAccessKey(String),
    /// A header value could not be constructed.
    InvalidHeaderValue(String),
}

impl std::fmt::Display for RebuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RebuildError::InvalidJson(e) => write!(f, "invalid JSON body: {}", e),
            RebuildError::MissingAccessKey(name) => {
                write!(f, "no access key configured for provider '{}'", name)
            }
            RebuildError::InvalidHeaderValue(header) => {
                write!(f, "invalid header value for '{}'", header)
            }
        }
    }
}

impl std::error::Error for RebuildError {}

/// Extended request context for retry tracking.
#[derive(Debug)]
pub struct RequestContext {
    pub request_id: String,
    pub attempted_providers: HashSet<String>,
    pub retry_start_time: Option<Instant>,
    pub attempt_number: u32,
    /// Request-scoped Retry_After_State (when apply_to: "request")
    pub request_retry_after_state: HashMap<String, Instant>,
    /// Request-scoped Latency_Block_State (when apply_to: "request")
    pub request_latency_block_state: HashMap<String, Instant>,
    /// Request signature for tracking
    pub request_signature: RequestSignature,
    /// Accumulated errors from all attempts
    pub errors: Vec<AttemptError>,
}

/// Bounded semaphore controlling the maximum number of concurrent in-flight
/// retry operations. Prevents OOM under high load by rejecting new retry
/// attempts when the limit is reached (fail-open: original request proceeds
/// without retry).
pub struct RetryGate {
    pub semaphore: Arc<tokio::sync::Semaphore>,
}

impl RetryGate {
    const DEFAULT_MAX_IN_FLIGHT: usize = 1000;

    pub fn new(max_in_flight_retries: usize) -> Self {
        Self {
            semaphore: Arc::new(tokio::sync::Semaphore::new(max_in_flight_retries)),
        }
    }

    pub fn try_acquire(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        self.semaphore.clone().try_acquire_owned().ok()
    }
}

impl Default for RetryGate {
    fn default() -> Self {
        Self::new(Self::DEFAULT_MAX_IN_FLIGHT)
    }
}

// ── Error Types ────────────────────────────────────────────────────────────

/// All retry attempts exhausted for a single provider's retry sequence.
#[derive(Debug)]
pub struct RetryExhaustedError {
    /// All attempt errors accumulated during the retry sequence.
    pub attempts: Vec<AttemptError>,
    /// Maximum Retry-After value observed across all attempts (if any).
    pub max_retry_after_seconds: Option<u64>,
    /// Shortest remaining block duration among blocked candidates at exhaustion time.
    pub shortest_remaining_block_seconds: Option<u64>,
    /// Whether the retry budget (max_retry_duration_ms) was exceeded.
    pub retry_budget_exhausted: bool,
}

/// All providers (including fallbacks) exhausted.
#[derive(Debug)]
pub struct AllProvidersExhaustedError {
    /// Shortest remaining block duration among blocked candidates.
    pub shortest_remaining_block_seconds: Option<u64>,
}

// ── Validation Types ───────────────────────────────────────────────────────

/// Configuration validation errors that prevent gateway startup.
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationError {
    /// Backoff section present without required `apply_to` field.
    BackoffMissingApplyTo { model: String },
    /// `min_triggers > 1` without `trigger_window_seconds`.
    LatencyMissingTriggerWindow { model: String },
    /// Invalid strategy value.
    InvalidStrategy { model: String, value: String },
    /// Invalid `apply_to` value.
    InvalidApplyTo { model: String, value: String },
    /// Invalid `scope` value.
    InvalidScope { model: String, value: String },
    /// Status code outside 100–599.
    StatusCodeOutOfRange { model: String, code: u16 },
    /// Range with start > end.
    StatusCodeRangeInverted { model: String, range: String },
    /// Invalid status code range format.
    StatusCodeRangeInvalid { model: String, range: String },
    /// `threshold_ms`, `block_duration_seconds`, `max_retry_after_seconds`,
    /// `max_retry_duration_ms`, or `base_ms` not positive.
    NonPositiveValue { model: String, field: String },
    /// `trigger_window_seconds` not positive when specified.
    NonPositiveTriggerWindow { model: String },
    /// `max_ms` ≤ `base_ms` in backoff config.
    MaxMsNotGreaterThanBaseMs {
        model: String,
        base_ms: u64,
        max_ms: u64,
    },
    /// `max_attempts` is negative (represented as u32, so this catches zero if needed).
    InvalidMaxAttempts { model: String, value: String },
    /// Fallback model string is empty or doesn't contain a "/" separator.
    InvalidFallbackModel { model: String, fallback: String },
}

/// Configuration validation warnings (gateway starts, warning logged).
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationWarning {
    /// Single provider with failover strategy.
    SingleProviderWithFailover { model: String, strategy: String },
    /// Provider-scope Retry-After with same_model strategy.
    ProviderScopeWithSameModel { model: String },
    /// Backoff apply_to mismatch with default strategy.
    BackoffApplyToMismatch {
        model: String,
        apply_to: String,
        strategy: String,
    },
    /// Latency scope/strategy mismatch.
    LatencyScopeStrategyMismatch { model: String },
    /// Aggressive latency threshold (< 1000ms).
    AggressiveLatencyThreshold { model: String, threshold_ms: u64 },
    /// Fallback model not in Provider_List.
    FallbackModelNotInProviderList { model: String, fallback: String },
    /// Overlapping status codes across on_status_codes entries.
    OverlappingStatusCodes { model: String, code: u16 },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::{LlmProvider, LlmProviderType};
    use bytes::Bytes;
    use hyper::header::{HeaderMap, HeaderValue, AUTHORIZATION};
    use proptest::prelude::*;

    fn make_provider(name: &str, interface: LlmProviderType, key: Option<&str>) -> LlmProvider {
        LlmProvider {
            name: name.to_string(),
            provider_interface: interface,
            access_key: key.map(|k| k.to_string()),
            model: Some(name.to_string()),
            default: None,
            stream: None,
            endpoint: None,
            port: None,
            rate_limits: None,
            usage: None,
            cluster_name: None,
            base_url_path_prefix: None,
            internal: None,
            passthrough_auth: None,
            retry_policy: None,
            headers: None,
        }
    }

    // ── RequestSignature tests ─────────────────────────────────────────

    #[test]
    fn test_request_signature_computes_hash() {
        let body = b"hello world";
        let headers = HeaderMap::new();
        let sig = RequestSignature::new(body, &headers, false, "openai/gpt-4o".to_string());

        // SHA-256 of "hello world" is deterministic
        let mut hasher = Sha256::new();
        hasher.update(b"hello world");
        let expected: [u8; 32] = hasher.finalize().into();
        assert_eq!(sig.body_hash, expected);
        assert!(!sig.streaming);
        assert_eq!(sig.original_model, "openai/gpt-4o");
    }

    #[test]
    fn test_request_signature_preserves_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-custom", HeaderValue::from_static("value"));
        let sig = RequestSignature::new(b"body", &headers, true, "model".to_string());
        assert_eq!(sig.headers.get("x-custom").unwrap(), "value");
        assert!(sig.streaming);
    }

    #[test]
    fn test_request_signature_different_bodies_different_hashes() {
        let headers = HeaderMap::new();
        let sig1 = RequestSignature::new(b"body1", &headers, false, "m".to_string());
        let sig2 = RequestSignature::new(b"body2", &headers, false, "m".to_string());
        assert_ne!(sig1.body_hash, sig2.body_hash);
    }

    // ── RetryGate tests ────────────────────────────────────────────────

    #[test]
    fn test_retry_gate_default_permits() {
        let gate = RetryGate::default();
        // Should be able to acquire at least one permit
        assert!(gate.try_acquire().is_some());
    }

    #[test]
    fn test_retry_gate_exhaustion() {
        let gate = RetryGate::new(1);
        let permit = gate.try_acquire();
        assert!(permit.is_some());
        // Second acquire should fail (only 1 permit)
        assert!(gate.try_acquire().is_none());
        // Drop permit, should be able to acquire again
        drop(permit);
        assert!(gate.try_acquire().is_some());
    }

    #[test]
    fn test_retry_gate_custom_capacity() {
        let gate = RetryGate::new(3);
        let _p1 = gate.try_acquire().unwrap();
        let _p2 = gate.try_acquire().unwrap();
        let _p3 = gate.try_acquire().unwrap();
        assert!(gate.try_acquire().is_none());
    }

    // ── rebuild_request_for_provider tests ─────────────────────────────

    #[test]
    fn test_rebuild_updates_model_field() {
        let body = Bytes::from(r#"{"model":"gpt-4o","messages":[]}"#);
        let headers = HeaderMap::new();
        let provider = make_provider(
            "openai/gpt-4o-mini",
            LlmProviderType::OpenAI,
            Some("sk-test"),
        );

        let (new_body, _) = rebuild_request_for_provider(&body, &provider, &headers).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&new_body).unwrap();
        assert_eq!(json["model"], "gpt-4o-mini");
    }

    #[test]
    fn test_rebuild_preserves_other_fields() {
        let body = Bytes::from(
            r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"temperature":0.7}"#,
        );
        let headers = HeaderMap::new();
        let provider = make_provider(
            "openai/gpt-4o-mini",
            LlmProviderType::OpenAI,
            Some("sk-test"),
        );

        let (new_body, _) = rebuild_request_for_provider(&body, &provider, &headers).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&new_body).unwrap();
        assert_eq!(json["messages"][0]["role"], "user");
        assert_eq!(json["messages"][0]["content"], "hi");
        assert_eq!(json["temperature"], 0.7);
    }

    #[test]
    fn test_rebuild_sets_openai_auth() {
        let body = Bytes::from(r#"{"model":"old"}"#);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer old-key"));
        let provider = make_provider("openai/gpt-4o", LlmProviderType::OpenAI, Some("sk-new"));

        let (_, new_headers) = rebuild_request_for_provider(&body, &provider, &headers).unwrap();
        assert_eq!(
            new_headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
            "Bearer sk-new"
        );
        assert!(new_headers.get("x-api-key").is_none());
    }

    #[test]
    fn test_rebuild_sets_anthropic_auth() {
        let body = Bytes::from(r#"{"model":"old"}"#);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer old-key"));
        let provider = make_provider(
            "anthropic/claude-3-5-sonnet",
            LlmProviderType::Anthropic,
            Some("ant-key"),
        );

        let (_, new_headers) = rebuild_request_for_provider(&body, &provider, &headers).unwrap();
        // Anthropic uses x-api-key, not Authorization
        assert!(new_headers.get(AUTHORIZATION).is_none());
        assert_eq!(
            new_headers.get("x-api-key").unwrap().to_str().unwrap(),
            "ant-key"
        );
        assert_eq!(
            new_headers
                .get("anthropic-version")
                .unwrap()
                .to_str()
                .unwrap(),
            "2023-06-01"
        );
    }

    #[test]
    fn test_rebuild_sanitizes_old_auth_headers() {
        let body = Bytes::from(r#"{"model":"old"}"#);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer old-key"));
        headers.insert("x-api-key", HeaderValue::from_static("old-api-key"));
        headers.insert("anthropic-version", HeaderValue::from_static("old-version"));
        headers.insert("x-custom", HeaderValue::from_static("keep-me"));

        let provider = make_provider("openai/gpt-4o", LlmProviderType::OpenAI, Some("sk-new"));
        let (_, new_headers) = rebuild_request_for_provider(&body, &provider, &headers).unwrap();

        // Old x-api-key and anthropic-version should be removed
        assert!(new_headers.get("anthropic-version").is_none());
        // New auth should be set
        assert_eq!(
            new_headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
            "Bearer sk-new"
        );
        // Custom headers preserved
        assert_eq!(
            new_headers.get("x-custom").unwrap().to_str().unwrap(),
            "keep-me"
        );
    }

    #[test]
    fn test_rebuild_passthrough_auth_skips_credentials() {
        let body = Bytes::from(r#"{"model":"old"}"#);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer client-key"));

        let mut provider = make_provider("openai/gpt-4o", LlmProviderType::OpenAI, Some("sk-new"));
        provider.passthrough_auth = Some(true);

        let (_, new_headers) = rebuild_request_for_provider(&body, &provider, &headers).unwrap();
        // Auth headers are sanitized, and passthrough_auth means no new ones are set
        assert!(new_headers.get(AUTHORIZATION).is_none());
    }

    #[test]
    fn test_rebuild_missing_access_key_errors() {
        let body = Bytes::from(r#"{"model":"old"}"#);
        let headers = HeaderMap::new();
        let provider = make_provider("openai/gpt-4o", LlmProviderType::OpenAI, None);

        let result = rebuild_request_for_provider(&body, &provider, &headers);
        assert!(matches!(result, Err(RebuildError::MissingAccessKey(_))));
    }

    #[test]
    fn test_rebuild_invalid_json_errors() {
        let body = Bytes::from("not json");
        let headers = HeaderMap::new();
        let provider = make_provider("openai/gpt-4o", LlmProviderType::OpenAI, Some("key"));

        let result = rebuild_request_for_provider(&body, &provider, &headers);
        assert!(matches!(result, Err(RebuildError::InvalidJson(_))));
    }

    #[test]
    fn test_rebuild_model_without_provider_prefix() {
        let body = Bytes::from(r#"{"model":"old"}"#);
        let headers = HeaderMap::new();
        let mut provider = make_provider("gpt-4o", LlmProviderType::OpenAI, Some("key"));
        provider.model = Some("gpt-4o".to_string());

        let (new_body, _) = rebuild_request_for_provider(&body, &provider, &headers).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&new_body).unwrap();
        // No prefix to strip, model name used as-is
        assert_eq!(json["model"], "gpt-4o");
    }

    // --- Proptest strategies ---

    fn arb_provider_type() -> impl Strategy<Value = LlmProviderType> {
        prop_oneof![
            Just(LlmProviderType::OpenAI),
            Just(LlmProviderType::Anthropic),
            Just(LlmProviderType::Gemini),
            Just(LlmProviderType::Deepseek),
        ]
    }

    fn arb_model_name() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("openai/gpt-4o".to_string()),
            Just("openai/gpt-4o-mini".to_string()),
            Just("anthropic/claude-3-5-sonnet".to_string()),
            Just("gemini/gemini-pro".to_string()),
            Just("deepseek/deepseek-chat".to_string()),
        ]
    }

    fn arb_target_provider() -> impl Strategy<Value = LlmProvider> {
        (arb_model_name(), arb_provider_type())
            .prop_map(|(model, iface)| make_provider(&model, iface, Some("test-key-123")))
    }

    fn arb_message_content() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9 ]{1,50}"
    }

    fn arb_messages() -> impl Strategy<Value = Vec<serde_json::Value>> {
        prop::collection::vec(
            (
                prop_oneof![Just("user"), Just("assistant"), Just("system")],
                arb_message_content(),
            )
                .prop_map(|(role, content)| serde_json::json!({"role": role, "content": content})),
            1..5,
        )
    }

    fn arb_json_body() -> impl Strategy<Value = serde_json::Value> {
        (
            arb_model_name(),
            arb_messages(),
            prop::option::of(0.0f64..2.0),
            prop::option::of(1u32..4096),
            proptest::bool::ANY,
        )
            .prop_map(|(model, messages, temperature, max_tokens, stream)| {
                let model_only = model.split('/').nth(1).unwrap_or(&model);
                let mut obj = serde_json::json!({
                    "model": model_only,
                    "messages": messages,
                });
                if let Some(t) = temperature {
                    obj["temperature"] = serde_json::json!(t);
                }
                if let Some(mt) = max_tokens {
                    obj["max_tokens"] = serde_json::json!(mt);
                }
                if stream {
                    obj["stream"] = serde_json::json!(true);
                }
                obj
            })
    }

    fn arb_custom_headers() -> impl Strategy<Value = Vec<(String, String)>> {
        prop::collection::vec(
            (
                prop_oneof![
                    Just("x-request-id".to_string()),
                    Just("x-custom-header".to_string()),
                    Just("x-trace-id".to_string()),
                    Just("content-type".to_string()),
                ],
                "[a-zA-Z0-9-]{1,30}",
            ),
            0..4,
        )
    }

    // Feature: retry-on-ratelimit, Property 14: Request Preservation Across Retries
    // **Validates: Requirements 5.1, 5.2, 5.3, 5.4, 5.5, 3.15**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// Property 14 – The original body bytes are unchanged after rebuild (body is passed by reference).
        /// The rebuilt body has the model field updated to the target provider's model.
        /// All other JSON fields are preserved. The RequestSignature hash matches the original body hash.
        /// Custom headers are preserved while auth headers are sanitized.
        #[test]
        fn prop_request_preservation_across_retries(
            json_body in arb_json_body(),
            custom_headers in arb_custom_headers(),
            streaming in proptest::bool::ANY,
            target_provider in arb_target_provider(),
        ) {
            let body_bytes = serde_json::to_vec(&json_body).unwrap();
            let body = Bytes::from(body_bytes.clone());

            // Build original headers with custom + auth headers
            let mut original_headers = HeaderMap::new();
            for (name, value) in &custom_headers {
                if let (Ok(hn), Ok(hv)) = (
                    hyper::header::HeaderName::from_bytes(name.as_bytes()),
                    HeaderValue::from_str(value),
                ) {
                    original_headers.insert(hn, hv);
                }
            }
            // Add auth headers that should be sanitized
            original_headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer old-secret"));
            original_headers.insert("x-api-key", HeaderValue::from_static("old-api-key"));

            let original_model = json_body["model"].as_str().unwrap_or("unknown").to_string();

            // Create RequestSignature from original body
            let sig = RequestSignature::new(&body, &original_headers, streaming, original_model.clone());

            // Assert: body bytes are unchanged (passed by reference, not modified)
            prop_assert_eq!(&body[..], &body_bytes[..], "Original body bytes must be unchanged");

            // Assert: RequestSignature hash matches a fresh hash of the same body
            let mut hasher = Sha256::new();
            hasher.update(&body);
            let expected_hash: [u8; 32] = hasher.finalize().into();
            prop_assert_eq!(sig.body_hash, expected_hash, "RequestSignature hash must match original body hash");

            // Assert: streaming flag preserved
            prop_assert_eq!(sig.streaming, streaming, "Streaming flag must be preserved in signature");

            // Rebuild for target provider
            let result = rebuild_request_for_provider(&body, &target_provider, &original_headers);
            prop_assert!(result.is_ok(), "rebuild_request_for_provider should succeed for valid JSON body");
            let (rebuilt_body, rebuilt_headers) = result.unwrap();

            // Parse rebuilt body
            let rebuilt_json: serde_json::Value = serde_json::from_slice(&rebuilt_body).unwrap();

            // Assert: model field updated to target provider's model (without prefix)
            let target_model = target_provider.model.as_deref().unwrap_or(&target_provider.name);
            let expected_model = target_model.split_once('/').map(|(_, m)| m).unwrap_or(target_model);
            prop_assert_eq!(
                rebuilt_json["model"].as_str().unwrap(),
                expected_model,
                "Model field must be updated to target provider's model"
            );

            // Assert: messages array preserved
            prop_assert_eq!(
                &rebuilt_json["messages"],
                &json_body["messages"],
                "Messages array must be preserved across rebuild"
            );

            // Assert: other JSON fields preserved (temperature, max_tokens, stream)
            // The rebuild function does a JSON round-trip (deserialize → modify model → serialize),
            // so we compare against a round-tripped version of the original to account for
            // any f64 precision changes inherent to JSON serialization.
            let original_round_tripped: serde_json::Value = serde_json::from_slice(
                &serde_json::to_vec(&json_body).unwrap()
            ).unwrap();
            for key in ["temperature", "max_tokens", "stream"] {
                if let Some(original_val) = original_round_tripped.get(key) {
                    prop_assert_eq!(
                        &rebuilt_json[key],
                        original_val,
                        "Field '{}' must be preserved across rebuild",
                        key
                    );
                }
            }

            // Assert: custom headers preserved (non-auth headers)
            // Note: HeaderMap::insert overwrites, so only the last value for each name survives
            let mut last_custom: std::collections::HashMap<String, String> = std::collections::HashMap::new();
            for (name, value) in &custom_headers {
                let lower = name.to_lowercase();
                if lower == "authorization" || lower == "x-api-key" || lower == "anthropic-version" {
                    continue;
                }
                last_custom.insert(lower, value.clone());
            }
            for (name, value) in &last_custom {
                if let Some(hv) = rebuilt_headers.get(name.as_str()) {
                    prop_assert_eq!(
                        hv.to_str().unwrap(),
                        value.as_str(),
                        "Custom header '{}' must be preserved",
                        name
                    );
                }
            }

            // Assert: old auth headers are sanitized (not leaked to target provider)
            // The old "Bearer old-secret" and "old-api-key" should NOT appear
            if let Some(auth) = rebuilt_headers.get(AUTHORIZATION) {
                prop_assert_ne!(
                    auth.to_str().unwrap(),
                    "Bearer old-secret",
                    "Old authorization header must be sanitized"
                );
            }
            if let Some(api_key) = rebuilt_headers.get("x-api-key") {
                prop_assert_ne!(
                    api_key.to_str().unwrap(),
                    "old-api-key",
                    "Old x-api-key header must be sanitized"
                );
            }

            // Assert: original body is still unchanged after rebuild
            prop_assert_eq!(&body[..], &body_bytes[..], "Original body bytes must remain unchanged after rebuild");
        }
    }
}
