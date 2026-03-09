use std::future::Future;
use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use log::{debug, info, warn};

use crate::configuration::{extract_provider, ApplyTo, BlockScope, HighLatencyConfig, LatencyMeasure, LlmProvider, RetryPolicy, RetryStrategy};

use super::backoff::BackoffCalculator;
use super::error_detector::{ErrorClassification, ErrorDetector, HttpResponse, TimeoutError};
use super::latency_block_state::LatencyBlockStateManager;
use super::provider_selector::{ProviderSelectionResult, ProviderSelector};
use super::retry_after_state::RetryAfterStateManager;
use super::latency_trigger::LatencyTriggerCounter;
use super::{
    AllProvidersExhaustedError, AttemptError, AttemptErrorType, RequestContext, RequestSignature,
    RetryExhaustedError, RetryGate,
};

// ── RetryOrchestrator ──────────────────────────────────────────────────────

/// Central coordinator for the retry loop.
///
/// Handles both the initial request attempt AND subsequent retries.
/// The primary model's `retry_policy` governs the entire retry sequence,
/// including when retrying to fallback models.
pub struct RetryOrchestrator {
    pub retry_after_state: Arc<RetryAfterStateManager>,
    pub latency_block_state: Arc<LatencyBlockStateManager>,
    pub latency_trigger_counter: Arc<LatencyTriggerCounter>,
    pub retry_gate: Arc<RetryGate>,
}

impl RetryOrchestrator {
    /// Create a new RetryOrchestrator with the given state managers and gate.
    pub fn new(
        retry_after_state: Arc<RetryAfterStateManager>,
        latency_block_state: Arc<LatencyBlockStateManager>,
        latency_trigger_counter: Arc<LatencyTriggerCounter>,
        retry_gate: Arc<RetryGate>,
    ) -> Self {
        Self {
            retry_after_state,
            latency_block_state,
            latency_trigger_counter,
            retry_gate,
        }
    }

    /// Create a RetryOrchestrator with default no-op/empty implementations (P0).
    pub fn new_default() -> Self {
        Self {
            retry_after_state: Arc::new(RetryAfterStateManager::new()),
            latency_block_state: Arc::new(LatencyBlockStateManager::new()),
            latency_trigger_counter: Arc::new(LatencyTriggerCounter::new()),
            retry_gate: Arc::new(RetryGate::default()),
        }
    }

    /// Execute a request with retry logic.
    ///
    /// Called from the LLM handler after model alias resolution.
    /// Makes the initial request attempt and handles retries on failure.
    ///
    /// The `forward_request` callback sends a request to an upstream provider
    /// without coupling the orchestrator to the HTTP client.
    pub async fn execute<F, Fut>(
        &self,
        body: &Bytes,
        _request_signature: &RequestSignature,
        primary_provider: &LlmProvider,
        retry_policy: &RetryPolicy,
        all_providers: &[LlmProvider],
        request_context: &mut RequestContext,
        forward_request: F,
    ) -> Result<HttpResponse, RetryExhaustedError>
    where
        F: Fn(&Bytes, &LlmProvider) -> Fut + Send + Sync,
        Fut: Future<Output = Result<HttpResponse, TimeoutError>> + Send,
    {
        let error_detector = ErrorDetector;
        let backoff_calculator = BackoffCalculator;
        let provider_selector = ProviderSelector;

        // Acquire RetryGate permit; if unavailable, make a single attempt (fail-open).
        let _permit = match self.retry_gate.try_acquire() {
            Some(permit) => Some(permit),
            None => {
                warn!(
                    "RetryGate permit unavailable for request_id={}; proceeding without retry (fail-open)",
                    request_context.request_id
                );
                // Make a single attempt with the primary provider, no retries.
                let request_start = Instant::now();
                let result = forward_request(body, primary_provider).await;
                let elapsed_ttfb_ms = request_start.elapsed().as_millis() as u64;
                let elapsed_total_ms = elapsed_ttfb_ms; // Same for now; refined when streaming support is added
                let classification =
                    error_detector.classify(result, retry_policy, elapsed_ttfb_ms, elapsed_total_ms);
                return match classification {
                    ErrorClassification::Success(response) => Ok(response),
                    ErrorClassification::NonRetriableError(response) => Ok(response),
                    ErrorClassification::RetriableError {
                        status_code,
                        response_body,
                        ..
                    } => {
                        let model_id = primary_provider
                            .model
                            .as_deref()
                            .unwrap_or(&primary_provider.name)
                            .to_string();
                        Err(RetryExhaustedError {
                            attempts: vec![AttemptError {
                                model_id,
                                error_type: AttemptErrorType::HttpError {
                                    status_code,
                                    body: response_body,
                                },
                                attempt_number: 1,
                            }],
                            max_retry_after_seconds: None,
                            shortest_remaining_block_seconds: None,
                            retry_budget_exhausted: false,
                        })
                    }
                    ErrorClassification::TimeoutError { duration_ms } => {
                        let model_id = primary_provider
                            .model
                            .as_deref()
                            .unwrap_or(&primary_provider.name)
                            .to_string();
                        Err(RetryExhaustedError {
                            attempts: vec![AttemptError {
                                model_id,
                                error_type: AttemptErrorType::Timeout { duration_ms },
                                attempt_number: 1,
                            }],
                            max_retry_after_seconds: None,
                            shortest_remaining_block_seconds: None,
                            retry_budget_exhausted: false,
                        })
                    }
                    ErrorClassification::HighLatencyEvent {
                        measured_ms,
                        threshold_ms,
                        response,
                        ..
                    } => {
                        // If response completed, return it (fail-open).
                        if let Some(resp) = response {
                            return Ok(resp);
                        }
                        let model_id = primary_provider
                            .model
                            .as_deref()
                            .unwrap_or(&primary_provider.name)
                            .to_string();
                        Err(RetryExhaustedError {
                            attempts: vec![AttemptError {
                                model_id,
                                error_type: AttemptErrorType::HighLatency {
                                    measured_ms,
                                    threshold_ms,
                                },
                                attempt_number: 1,
                            }],
                            max_retry_after_seconds: None,
                            shortest_remaining_block_seconds: None,
                            retry_budget_exhausted: false,
                        })
                    }
                };
            }
        };

        // Track per-classification attempt counts: (strategy, max_attempts) -> count
        let mut attempt_counts: std::collections::HashMap<(u16, Option<u64>), u32> =
            std::collections::HashMap::new();

        let mut current_provider = primary_provider;
        let mut previous_provider_model = primary_provider
            .model
            .as_deref()
            .unwrap_or(&primary_provider.name)
            .to_string();

        // The overall attempt number (1-based).
        let mut overall_attempt: u32 = 0;

        loop {
            overall_attempt += 1;
            let current_model_id = current_provider
                .model
                .as_deref()
                .unwrap_or(&current_provider.name)
                .to_string();

            // Track attempted provider
            request_context
                .attempted_providers
                .insert(current_model_id.clone());
            request_context.attempt_number = overall_attempt;

            // Forward the request
            let request_start = Instant::now();
            let result = forward_request(body, current_provider).await;
            let elapsed_ttfb_ms = request_start.elapsed().as_millis() as u64;
            let elapsed_total_ms = elapsed_ttfb_ms; // Same for now; refined when streaming support is added

            // Emit latency metrics per model
            info!(
                "metric.latency: model={}, ttfb_ms={}, total_ms={}, request_id={}",
                current_model_id, elapsed_ttfb_ms, elapsed_total_ms, request_context.request_id
            );

            // Classify the response
            let classification = error_detector.classify(result, retry_policy, elapsed_ttfb_ms, elapsed_total_ms);

            match classification {
                ErrorClassification::Success(response) => {
                    if overall_attempt > 1 {
                        info!(
                            "Retry succeeded: model={}, total_attempts={}, request_id={}",
                            current_model_id, overall_attempt, request_context.request_id
                        );
                        // Emit metric event for retry success
                        info!(
                            "metric.retry_success: model={}, total_attempts={}, request_id={}",
                            current_model_id, overall_attempt, request_context.request_id
                        );
                    }
                    return Ok(response);
                }
                ErrorClassification::NonRetriableError(response) => {
                    // Non-retriable errors are returned as-is (not an exhaustion error).
                    return Ok(response);
                }
                ErrorClassification::HighLatencyEvent {
                    measured_ms,
                    threshold_ms: _,
                    response: Some(resp),
                    ..
                } => {
                    // Completed-but-slow response: deliver to client, but record
                    // the event for future blocking.
                    if let Some(hl_config) = &retry_policy.on_high_latency {
                        self.record_latency_event(
                            &current_model_id,
                            measured_ms,
                            hl_config,
                            request_context,
                        );
                    }
                    return Ok(resp);
                }
                _ => {
                    // Retriable error, timeout, or incomplete high-latency event.
                    // Proceed with retry logic.
                }
            }

            // Record Retry_After_State when a retriable error has a Retry-After header
            if let ErrorClassification::RetriableError {
                retry_after_seconds: Some(retry_after_value),
                ..
            } = &classification
            {
                // Log #1: Retry-After header value extracted
                info!(
                    "Retry-After header value: {}s for model {} (request_id={})",
                    retry_after_value, current_model_id, request_context.request_id
                );

                let ra_config = retry_policy.effective_retry_after_config();
                let identifier = match ra_config.scope {
                    BlockScope::Model => current_model_id.clone(),
                    BlockScope::Provider => extract_provider(&current_model_id).to_string(),
                };

                // Log #3: Retry-After value capped (if applicable)
                let capped = (*retry_after_value).min(ra_config.max_retry_after_seconds);
                if *retry_after_value > ra_config.max_retry_after_seconds {
                    warn!(
                        "Retry-After value capped: original={}s, capped={}s, max_retry_after_seconds={} (request_id={})",
                        retry_after_value, capped, ra_config.max_retry_after_seconds, request_context.request_id
                    );
                }

                match ra_config.apply_to {
                    ApplyTo::Global => {
                        self.retry_after_state.record(
                            &identifier,
                            *retry_after_value,
                            ra_config.max_retry_after_seconds,
                        );
                        // Log #2: Retry_After_State created
                        info!(
                            "Retry_After_State created: identifier={}, expires_in={}s, apply_to=global (request_id={})",
                            identifier, capped, request_context.request_id
                        );
                    }
                    ApplyTo::Request => {
                        let expires_at = Instant::now()
                            + std::time::Duration::from_secs(capped);
                        request_context
                            .request_retry_after_state
                            .insert(identifier.clone(), expires_at);
                        // Log #2: Retry_After_State created (request-scoped)
                        info!(
                            "Retry_After_State created: identifier={}, expires_in={}s, apply_to=request (request_id={})",
                            identifier, capped, request_context.request_id
                        );
                    }
                }
            }

            // Record latency event for HighLatencyEvent without completed response
            // (triggers retry, but also records for future blocking)
            if let ErrorClassification::HighLatencyEvent {
                measured_ms,
                ..
            } = &classification
            {
                if let Some(hl_config) = &retry_policy.on_high_latency {
                    self.record_latency_event(
                        &current_model_id,
                        *measured_ms,
                        hl_config,
                        request_context,
                    );
                }
            }

            // Dual-classification: TimeoutError + HighLatency
            // When on_high_latency is configured and the elapsed time exceeded threshold_ms,
            // record a HighLatencyEvent for blocking purposes even though we return TimeoutError
            // for retry purposes.
            if let ErrorClassification::TimeoutError { duration_ms } = &classification {
                if let Some(hl_config) = &retry_policy.on_high_latency {
                    let measured_ms = match hl_config.measure {
                        LatencyMeasure::Ttfb => elapsed_ttfb_ms,
                        LatencyMeasure::Total => elapsed_total_ms,
                    };
                    if measured_ms > hl_config.threshold_ms {
                        info!(
                            "Dual-classification: TimeoutError ({}ms) also exceeds high latency threshold ({}ms) for model={}, request_id={}",
                            duration_ms, hl_config.threshold_ms, current_model_id, request_context.request_id
                        );
                        self.record_latency_event(
                            &current_model_id,
                            measured_ms,
                            hl_config,
                            request_context,
                        );
                    }
                }
            }

            // Resolve retry params for this classification
            let (strategy, max_attempts) =
                error_detector.resolve_retry_params(&classification, retry_policy);

            // Build a key for per-classification attempt tracking.
            // Use (status_code_or_sentinel, timeout_duration) as key.
            let classification_key = match &classification {
                ErrorClassification::RetriableError { status_code, .. } => {
                    (*status_code, None)
                }
                ErrorClassification::TimeoutError { .. } => {
                    // All timeouts share a single counter regardless of duration,
                    // since on_timeout has a single max_attempts value.
                    (0u16, None)
                }
                ErrorClassification::HighLatencyEvent { .. } => (1u16, None),
                _ => (u16::MAX, None),
            };

            let count = attempt_counts.entry(classification_key).or_insert(0);
            *count += 1;

            // Record the attempt error
            let attempt_error = build_attempt_error(&classification, &current_model_id, overall_attempt);
            request_context.errors.push(attempt_error);

            // Log the retriable error
            log_retriable_error(&classification, &current_model_id, overall_attempt, &request_context.request_id);

            // Check max_attempts for this classification
            if *count >= max_attempts {
                let attempted_models: Vec<String> = request_context
                    .errors
                    .iter()
                    .map(|e| e.model_id.clone())
                    .collect();
                let error_types: Vec<String> = request_context
                    .errors
                    .iter()
                    .map(|e| format_attempt_error_type(&e.error_type))
                    .collect();
                warn!(
                    "All retries exhausted: attempted_models={:?}, error_types={:?}, total_attempts={}, request_id={}",
                    attempted_models, error_types, overall_attempt, request_context.request_id
                );
                // Emit metric event for retry failure (exhausted)
                info!(
                    "metric.retry_failure: reason=max_attempts_reached, model={}, total_attempts={}, request_id={}",
                    current_model_id, overall_attempt, request_context.request_id
                );
                return Err(build_exhausted_error(request_context));
            }

            // Check max_retry_duration_ms budget
            if let Some(max_duration_ms) = retry_policy.max_retry_duration_ms {
                // Start the timer on the first retry (not the original request)
                if request_context.retry_start_time.is_none() {
                    request_context.retry_start_time = Some(Instant::now());
                }

                if let Some(start) = request_context.retry_start_time {
                    let elapsed = start.elapsed();
                    if elapsed.as_millis() as u64 >= max_duration_ms {
                        warn!(
                            "Retry budget exhausted ({}ms >= {}ms), request_id={}",
                            elapsed.as_millis(),
                            max_duration_ms,
                            request_context.request_id
                        );
                        // Emit metric event for retry failure (budget exhausted)
                        info!(
                            "metric.retry_failure: reason=budget_exhausted, model={}, elapsed_ms={}, budget_ms={}, request_id={}",
                            current_model_id, elapsed.as_millis(), max_duration_ms, request_context.request_id
                        );
                        let mut err = build_exhausted_error(request_context);
                        err.retry_budget_exhausted = true;
                        return Err(err);
                    }
                }
            }

            // Select next provider
            // For same_model strategy, temporarily remove the current model from
            // attempted so the provider selector can re-select it.
            if strategy == RetryStrategy::SameModel {
                request_context
                    .attempted_providers
                    .remove(&current_model_id);
            }
            let selection = provider_selector.select(
                strategy,
                primary_provider
                    .model
                    .as_deref()
                    .unwrap_or(&primary_provider.name),
                &retry_policy.fallback_models,
                all_providers,
                &request_context.attempted_providers,
                &self.retry_after_state,
                &self.latency_block_state,
                request_context,
                retry_policy.retry_after_handling.is_some(),
                retry_policy.on_high_latency.is_some(),
            );

            let next_provider = match selection {
                Ok(ProviderSelectionResult::Selected(provider)) => provider,
                Ok(ProviderSelectionResult::WaitAndRetrySameModel { wait_duration }) => {
                    // Sleep for the wait duration, then retry the same provider.
                    // Pass the wait_duration as retry_after_seconds to backoff calculator.
                    let retry_after_secs = wait_duration.as_secs();
                    let delay = backoff_calculator.calculate_delay(
                        overall_attempt.saturating_sub(1),
                        retry_policy.backoff.as_ref(),
                        Some(retry_after_secs),
                        strategy,
                        &current_model_id,
                        &previous_provider_model,
                    );
                    tokio::time::sleep(delay).await;

                    // For same_model wait-and-retry, we need to allow re-attempting
                    // the same provider, so remove it from attempted set temporarily.
                    request_context
                        .attempted_providers
                        .remove(&current_model_id);
                    previous_provider_model = current_model_id;
                    continue;
                }
                Err(AllProvidersExhaustedError {
                    shortest_remaining_block_seconds,
                }) => {
                    let attempted_models: Vec<String> = request_context
                        .errors
                        .iter()
                        .map(|e| e.model_id.clone())
                        .collect();
                    let error_types: Vec<String> = request_context
                        .errors
                        .iter()
                        .map(|e| format_attempt_error_type(&e.error_type))
                        .collect();
                    warn!(
                        "All retries exhausted (providers exhausted): attempted_models={:?}, error_types={:?}, total_attempts={}, request_id={}",
                        attempted_models, error_types, overall_attempt, request_context.request_id
                    );
                    // Emit metric event for retry failure (providers exhausted)
                    info!(
                        "metric.retry_failure: reason=providers_exhausted, model={}, total_attempts={}, request_id={}",
                        current_model_id, overall_attempt, request_context.request_id
                    );
                    let mut err = build_exhausted_error(request_context);
                    err.shortest_remaining_block_seconds = shortest_remaining_block_seconds;
                    return Err(err);
                }
            };

            // Calculate backoff delay
            let next_model_id = next_provider
                .model
                .as_deref()
                .unwrap_or(&next_provider.name);

            let retry_after_secs = match &classification {
                ErrorClassification::RetriableError {
                    retry_after_seconds,
                    ..
                } => *retry_after_seconds,
                _ => None,
            };

            let delay = backoff_calculator.calculate_delay(
                overall_attempt.saturating_sub(1),
                retry_policy.backoff.as_ref(),
                retry_after_secs,
                strategy,
                next_model_id,
                &previous_provider_model,
            );

            // Check budget again after calculating delay
            if let Some(max_duration_ms) = retry_policy.max_retry_duration_ms {
                if let Some(start) = request_context.retry_start_time {
                    let elapsed_after_delay =
                        start.elapsed().as_millis() as u64 + delay.as_millis() as u64;
                    if elapsed_after_delay >= max_duration_ms {
                        warn!(
                            "Retry budget would be exhausted after backoff delay ({}ms >= {}ms), request_id={}",
                            elapsed_after_delay,
                            max_duration_ms,
                            request_context.request_id
                        );
                        // Emit metric event for retry failure (budget exhausted with backoff)
                        info!(
                            "metric.retry_failure: reason=budget_exhausted, model={}, elapsed_ms={}, budget_ms={}, request_id={}",
                            current_model_id, elapsed_after_delay, max_duration_ms, request_context.request_id
                        );
                        let mut err = build_exhausted_error(request_context);
                        err.retry_budget_exhausted = true;
                        return Err(err);
                    }
                }
            }

            if !delay.is_zero() {
                debug!(
                    "Backoff delay: {}ms before retry attempt {}, model={}, request_id={}",
                    delay.as_millis(),
                    overall_attempt + 1,
                    next_model_id,
                    request_context.request_id
                );
                tokio::time::sleep(delay).await;
            }

            info!(
                "Retry initiated: original_model={}, target_model={}, error_type={}, attempt={}, request_id={}",
                previous_provider_model,
                next_model_id,
                classify_error_type(&classification),
                overall_attempt + 1,
                request_context.request_id
            );

            // Emit metric event for retry attempt
            info!(
                "metric.retry_attempt: model={}, target_model={}, status_code={}, error_type={}, request_id={}",
                previous_provider_model,
                next_model_id,
                classify_status_code(&classification),
                classify_error_type(&classification),
                request_context.request_id
            );

            previous_provider_model = current_model_id;
            current_provider = next_provider;
        }
    }

    /// Record a high latency event in the LatencyTriggerCounter and, if the
    /// min_triggers threshold is met, create a LatencyBlockState entry.
    ///
    /// The identifier is derived from the model ID based on the configured scope:
    /// - `BlockScope::Model` → use the full model ID
    /// - `BlockScope::Provider` → use the provider prefix (e.g., "openai" from "openai/gpt-4o")
    ///
    /// The block state is routed by `apply_to`:
    /// - `ApplyTo::Global` → recorded via `LatencyBlockStateManager`
    /// - `ApplyTo::Request` → recorded in `RequestContext.request_latency_block_state`
    fn record_latency_event(
        &self,
        model_id: &str,
        measured_ms: u64,
        hl_config: &HighLatencyConfig,
        request_context: &mut RequestContext,
    ) {
        let identifier = match hl_config.scope {
            BlockScope::Model => model_id.to_string(),
            BlockScope::Provider => extract_provider(model_id).to_string(),
        };

        let trigger_window = hl_config.trigger_window_seconds.unwrap_or(60);
        let threshold_met = self.latency_trigger_counter.record_event(
            &identifier,
            hl_config.min_triggers,
            trigger_window,
        );

        info!(
            "High latency event recorded: identifier={}, measured_ms={}, threshold_ms={}, measure={:?}, triggers_met={}, request_id={}",
            identifier, measured_ms, hl_config.threshold_ms, hl_config.measure, threshold_met, request_context.request_id
        );

        if threshold_met {
            // Reset the trigger counter to prevent re-triggering on the same events
            self.latency_trigger_counter.reset(&identifier);

            match hl_config.apply_to {
                ApplyTo::Global => {
                    self.latency_block_state.record_block(
                        &identifier,
                        hl_config.block_duration_seconds,
                        measured_ms,
                    );
                    info!(
                        "Latency_Block_State created: identifier={}, block_duration={}s, measured_ms={}, apply_to=global, request_id={}",
                        identifier, hl_config.block_duration_seconds, measured_ms, request_context.request_id
                    );
                    // Emit metric for LB creation
                    info!(
                        "metric.latency_block_created: model={}, block_duration_seconds={}, measured_ms={}, apply_to=global, request_id={}",
                        identifier, hl_config.block_duration_seconds, measured_ms, request_context.request_id
                    );
                }
                ApplyTo::Request => {
                    let expires_at = Instant::now()
                        + std::time::Duration::from_secs(hl_config.block_duration_seconds);
                    request_context
                        .request_latency_block_state
                        .insert(identifier.clone(), expires_at);
                    info!(
                        "Latency_Block_State created: identifier={}, block_duration={}s, measured_ms={}, apply_to=request, request_id={}",
                        identifier, hl_config.block_duration_seconds, measured_ms, request_context.request_id
                    );
                    // Emit metric for LB creation (request-scoped)
                    info!(
                        "metric.latency_block_created: model={}, block_duration_seconds={}, measured_ms={}, apply_to=request, request_id={}",
                        identifier, hl_config.block_duration_seconds, measured_ms, request_context.request_id
                    );
                }
            }
        }
    }
}

// ── Helper functions ───────────────────────────────────────────────────────

fn build_attempt_error(
    classification: &ErrorClassification,
    model_id: &str,
    attempt_number: u32,
) -> AttemptError {
    let error_type = match classification {
        ErrorClassification::RetriableError {
            status_code,
            response_body,
            ..
        } => AttemptErrorType::HttpError {
            status_code: *status_code,
            body: response_body.clone(),
        },
        ErrorClassification::TimeoutError { duration_ms } => {
            AttemptErrorType::Timeout {
                duration_ms: *duration_ms,
            }
        }
        ErrorClassification::HighLatencyEvent {
            measured_ms,
            threshold_ms,
            ..
        } => AttemptErrorType::HighLatency {
            measured_ms: *measured_ms,
            threshold_ms: *threshold_ms,
        },
        // Should not be called for Success/NonRetriableError, but handle gracefully.
        _ => AttemptErrorType::HttpError {
            status_code: 0,
            body: Vec::new(),
        },
    };

    AttemptError {
        model_id: model_id.to_string(),
        error_type,
        attempt_number,
    }
}

fn build_exhausted_error(request_context: &RequestContext) -> RetryExhaustedError {
    RetryExhaustedError {
        attempts: request_context.errors.clone(),
        max_retry_after_seconds: None,
        shortest_remaining_block_seconds: None,
        retry_budget_exhausted: false,
    }
}

/// Return a human-readable error type string for a classification.
fn classify_error_type(classification: &ErrorClassification) -> &'static str {
    match classification {
        ErrorClassification::RetriableError { .. } => "retriable_http_error",
        ErrorClassification::TimeoutError { .. } => "timeout",
        ErrorClassification::HighLatencyEvent { .. } => "high_latency",
        ErrorClassification::Success(_) => "success",
        ErrorClassification::NonRetriableError(_) => "non_retriable",
    }
}

/// Return the HTTP status code from a classification, or 0 for non-HTTP errors.
fn classify_status_code(classification: &ErrorClassification) -> u16 {
    match classification {
        ErrorClassification::RetriableError { status_code, .. } => *status_code,
        _ => 0,
    }
}

/// Format an AttemptErrorType for logging.
fn format_attempt_error_type(error_type: &AttemptErrorType) -> String {
    match error_type {
        AttemptErrorType::HttpError { status_code, .. } => format!("http_{}", status_code),
        AttemptErrorType::Timeout { duration_ms } => format!("timeout_{}ms", duration_ms),
        AttemptErrorType::HighLatency {
            measured_ms,
            threshold_ms,
        } => format!("high_latency_{}ms_threshold_{}ms", measured_ms, threshold_ms),
    }
}

fn log_retriable_error(
    classification: &ErrorClassification,
    model_id: &str,
    attempt_number: u32,
    request_id: &str,
) {
    match classification {
        ErrorClassification::RetriableError {
            status_code,
            retry_after_seconds,
            ..
        } => {
            warn!(
                "Retriable error detected: provider={}, status_code={}, retry_after={:?}, attempt={}, request_id={}",
                model_id, status_code, retry_after_seconds, attempt_number, request_id
            );
            // Emit metric event for retriable error per model and status code
            info!(
                "metric.retriable_error: model={}, status_code={}, retry_after={:?}, request_id={}",
                model_id, status_code, retry_after_seconds, request_id
            );
        }
        ErrorClassification::TimeoutError { duration_ms } => {
            warn!(
                "Timeout error detected: provider={}, duration_ms={}, attempt={}, request_id={}",
                model_id, duration_ms, attempt_number, request_id
            );
            // Emit metric event for timeout per model
            info!(
                "metric.timeout_error: model={}, duration_ms={}, request_id={}",
                model_id, duration_ms, request_id
            );
        }
        ErrorClassification::HighLatencyEvent {
            measured_ms,
            threshold_ms,
            measure,
            ..
        } => {
            warn!(
                "High latency event detected: provider={}, measured_ms={}, threshold_ms={}, measure={:?}, attempt={}, request_id={}",
                model_id, measured_ms, threshold_ms, measure, attempt_number, request_id
            );
            // Emit metric event for high latency per model
            info!(
                "metric.high_latency_event: model={}, measured_ms={}, threshold_ms={}, measure={:?}, request_id={}",
                model_id, measured_ms, threshold_ms, measure, request_id
            );
        }
        _ => {}
    }
}

