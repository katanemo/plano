use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use hyper::HeaderMap;
use sha2::{Digest, Sha256};

use crate::configuration::{ApplyTo, LlmProvider, LlmProviderType};

// Sub-modules
pub mod validation;
pub mod error_detector;
pub mod backoff;
pub mod provider_selector;
pub mod orchestrator;
pub mod error_response;
pub mod retry_after_state;
pub mod latency_trigger;
pub mod latency_block_state;

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

    let updated_body =
        Bytes::from(serde_json::to_vec(&json_body).map_err(|e| RebuildError::InvalidJson(e.to_string()))?);

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
    MaxMsNotGreaterThanBaseMs { model: String, base_ms: u64, max_ms: u64 },
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
    BackoffApplyToMismatch { model: String, apply_to: String, strategy: String },
    /// Latency scope/strategy mismatch.
    LatencyScopeStrategyMismatch { model: String },
    /// Aggressive latency threshold (< 1000ms).
    AggressiveLatencyThreshold { model: String, threshold_ms: u64 },
    /// Fallback model not in Provider_List.
    FallbackModelNotInProviderList { model: String, fallback: String },
    /// Overlapping status codes across on_status_codes entries.
    OverlappingStatusCodes { model: String, code: u16 },
}


