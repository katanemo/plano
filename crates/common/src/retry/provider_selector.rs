use std::collections::HashSet;
use std::time::{Duration, Instant};

use log::{info, warn};

use crate::configuration::{
    extract_provider, ApplyTo, BlockScope, HighLatencyConfig, LlmProvider,
    RetryAfterHandlingConfig, RetryStrategy,
};

use super::latency_block_state::LatencyBlockStateManager;
use super::retry_after_state::RetryAfterStateManager;
use super::{AllProvidersExhaustedError, RequestContext};

// ── Provider Selection ─────────────────────────────────────────────────

/// Result of a provider selection attempt.
#[derive(Debug)]
pub enum ProviderSelectionResult<'a> {
    /// A provider was selected for the next attempt.
    Selected(&'a LlmProvider),
    /// The same model should be retried after waiting the specified duration.
    /// Used when strategy is "same_model" and the model is blocked by global Retry_After_State.
    WaitAndRetrySameModel { wait_duration: Duration },
}

pub struct ProviderSelector;

impl ProviderSelector {
    /// Select the next provider for an attempt (initial or retry).
    ///
    /// When `has_retry_policy` is true, checks Retry_After_State before selecting:
    /// - Global state is checked via `retry_after_state` (RetryAfterStateManager)
    /// - Request-scoped state is checked via `request_context.request_retry_after_state`
    /// - The `retry_after_config` determines scope (model vs provider) and apply_to (global vs request)
    ///
    /// For `SameModel` strategy with a global RA block, returns `WaitAndRetrySameModel`
    /// with the remaining block duration. For other strategies, blocked candidates are skipped.
    ///
    /// When `fallback_models` is non-empty and strategy is `SameProvider` or
    /// `DifferentProvider`, candidates from `fallback_models` are tried first
    /// (in defined order), applying the same strategy filter. Fallback models
    /// not present in `all_providers` are skipped with a warning. Once all
    /// fallback candidates are exhausted, remaining providers from
    /// `all_providers` (Provider_List) are tried.
    #[allow(unused_variables)]
    pub fn select<'a>(
        &self,
        strategy: RetryStrategy,
        primary_model: &str,
        fallback_models: &[String],
        all_providers: &'a [LlmProvider],
        attempted: &HashSet<String>,
        retry_after_state: &RetryAfterStateManager,
        latency_block_state: &LatencyBlockStateManager,
        request_context: &RequestContext,
        has_retry_policy: bool,
        has_high_latency_config: bool,
    ) -> Result<ProviderSelectionResult<'a>, AllProvidersExhaustedError> {
        let primary_provider_prefix = extract_provider(primary_model);

        // Resolve the effective RA config — only used when has_retry_policy is true.
        // We need scope and apply_to to determine how to check blocking state.
        // The caller should ensure has_retry_policy aligns with the presence of a retry policy.

        match strategy {
            RetryStrategy::SameModel => {
                // Return the provider whose model matches primary_model exactly,
                // provided it hasn't already been attempted.
                let candidate = all_providers.iter().find(|p| {
                    p.model.as_deref() == Some(primary_model)
                        && !attempted.contains(primary_model)
                });

                match candidate {
                    Some(provider) => {
                        // Check RA state for same_model: if blocked, return WaitAndRetrySameModel
                        if has_retry_policy {
                            if let Some(ra_config) = provider
                                .retry_policy
                                .as_ref()
                                .map(|rp| rp.effective_retry_after_config())
                            {
                                if let Some(remaining) = self.check_ra_remaining_duration(
                                    primary_model,
                                    &ra_config,
                                    retry_after_state,
                                    request_context,
                                ) {
                                    return Ok(ProviderSelectionResult::WaitAndRetrySameModel {
                                        wait_duration: remaining,
                                    });
                                }
                            }
                        }

                        // Check LB state for same_model: if blocked, skip to alternative
                        // (unlike RA which waits, LB returns AllProvidersExhaustedError)
                        if has_high_latency_config {
                            if let Some(hl_config) = provider
                                .retry_policy
                                .as_ref()
                                .and_then(|rp| rp.on_high_latency.as_ref())
                            {
                                if self.is_model_lb_blocked(
                                    primary_model,
                                    hl_config,
                                    latency_block_state,
                                    request_context,
                                ) {
                                    let remaining_secs = self
                                        .check_lb_remaining_duration(
                                            primary_model,
                                            hl_config,
                                            latency_block_state,
                                            request_context,
                                        )
                                        .map(|d| d.as_secs());
                                    info!(
                                        "Model {} skipped due to Latency_Block_State (same_model), remaining={}s (request_id={})",
                                        primary_model,
                                        remaining_secs.unwrap_or(0),
                                        request_context.request_id
                                    );
                                    return Err(AllProvidersExhaustedError {
                                        shortest_remaining_block_seconds: remaining_secs,
                                    });
                                }
                            }
                        }

                        Ok(ProviderSelectionResult::Selected(provider))
                    }
                    None => Err(AllProvidersExhaustedError {
                        shortest_remaining_block_seconds: None,
                    }),
                }
            }

            RetryStrategy::SameProvider | RetryStrategy::DifferentProvider => {
                let matches_strategy = |model_id: &str| -> bool {
                    let provider_prefix = extract_provider(model_id);
                    match strategy {
                        RetryStrategy::SameProvider => provider_prefix == primary_provider_prefix,
                        RetryStrategy::DifferentProvider => {
                            provider_prefix != primary_provider_prefix
                        }
                        _ => unreachable!(),
                    }
                };

                // Build a closure that checks if a model is RA-blocked.
                // Uses the primary provider's retry_after_config for scope/apply_to.
                let primary_ra_config = if has_retry_policy {
                    // Find the primary provider to get its RA config
                    all_providers
                        .iter()
                        .find(|p| p.model.as_deref() == Some(primary_model))
                        .and_then(|p| p.retry_policy.as_ref())
                        .map(|rp| rp.effective_retry_after_config())
                } else {
                    None
                };

                let is_ra_blocked = |model_id: &str| -> bool {
                    if let Some(ref ra_config) = primary_ra_config {
                        self.is_model_ra_blocked(
                            model_id,
                            ra_config,
                            retry_after_state,
                            request_context,
                        )
                    } else {
                        false
                    }
                };

                // Build a closure that checks if a model is LB-blocked.
                // Uses the primary provider's on_high_latency config for scope/apply_to.
                let primary_hl_config = if has_high_latency_config {
                    all_providers
                        .iter()
                        .find(|p| p.model.as_deref() == Some(primary_model))
                        .and_then(|p| p.retry_policy.as_ref())
                        .and_then(|rp| rp.on_high_latency.as_ref())
                        .cloned()
                } else {
                    None
                };

                let is_lb_blocked = |model_id: &str| -> bool {
                    if let Some(ref hl_config) = primary_hl_config {
                        self.is_model_lb_blocked(
                            model_id,
                            hl_config,
                            latency_block_state,
                            request_context,
                        )
                    } else {
                        false
                    }
                };

                let mut shortest_remaining: Option<u64> = None;

                // Phase 1: Try fallback_models in defined order (if non-empty).
                if !fallback_models.is_empty() {
                    for (position, fallback_model) in fallback_models.iter().enumerate() {
                        // Skip if already attempted.
                        if attempted.contains(fallback_model.as_str()) {
                            continue;
                        }

                        // Skip if it doesn't match the strategy filter.
                        if !matches_strategy(fallback_model) {
                            continue;
                        }

                        // Skip if RA-blocked.
                        if is_ra_blocked(fallback_model) {
                            // Log #4: Model skipped due to RA state
                            if let Some(ref ra_config) = primary_ra_config {
                                if let Some(remaining) = self.check_ra_remaining_duration(
                                    fallback_model,
                                    ra_config,
                                    retry_after_state,
                                    request_context,
                                ) {
                                    let secs = remaining.as_secs();
                                    info!(
                                        "Model {} skipped due to Retry_After_State, remaining={}s (request_id={})",
                                        fallback_model, secs, request_context.request_id
                                    );
                                    shortest_remaining = Some(
                                        shortest_remaining.map_or(secs, |s: u64| s.min(secs)),
                                    );
                                }
                            }
                            continue;
                        }

                        // Skip if LB-blocked (either RA or LB is sufficient to skip).
                        if is_lb_blocked(fallback_model) {
                            if let Some(ref hl_config) = primary_hl_config {
                                if let Some(remaining) = self.check_lb_remaining_duration(
                                    fallback_model,
                                    hl_config,
                                    latency_block_state,
                                    request_context,
                                ) {
                                    let secs = remaining.as_secs();
                                    info!(
                                        "Model {} skipped due to Latency_Block_State, remaining={}s (request_id={})",
                                        fallback_model, secs, request_context.request_id
                                    );
                                    shortest_remaining = Some(
                                        shortest_remaining.map_or(secs, |s: u64| s.min(secs)),
                                    );
                                }
                            }
                            continue;
                        }

                        // Find the corresponding provider in all_providers.
                        let provider = all_providers
                            .iter()
                            .find(|p| p.model.as_deref() == Some(fallback_model.as_str()));

                        match provider {
                            Some(p) => {
                                // Log #5: Fallback model selected
                                info!(
                                    "Fallback model selected: {} (position {} in fallback list) (request_id={})",
                                    fallback_model, position, request_context.request_id
                                );
                                return Ok(ProviderSelectionResult::Selected(p));
                            }
                            None => {
                                warn!(
                                    "Fallback model '{}' not found in Provider_List, skipping",
                                    fallback_model
                                );
                                continue;
                            }
                        }
                    }

                    // Log #6: All fallback models exhausted
                    info!(
                        "All fallback models exhausted, switching to Provider_List (request_id={})",
                        request_context.request_id
                    );
                }

                // Phase 2: Fall back to Provider_List ordering, excluding
                // already-attempted providers and models already covered by
                // fallback_models (they were either selected above or skipped).
                for p in all_providers.iter() {
                    if let Some(ref model_id) = p.model {
                        if !matches_strategy(model_id) || attempted.contains(model_id.as_str()) {
                            continue;
                        }
                        if is_ra_blocked(model_id) {
                            // Log #4: Model skipped due to RA state (Provider_List phase)
                            if let Some(ref ra_config) = primary_ra_config {
                                if let Some(remaining) = self.check_ra_remaining_duration(
                                    model_id,
                                    ra_config,
                                    retry_after_state,
                                    request_context,
                                ) {
                                    let secs = remaining.as_secs();
                                    info!(
                                        "Model {} skipped due to Retry_After_State, remaining={}s (request_id={})",
                                        model_id, secs, request_context.request_id
                                    );
                                    shortest_remaining = Some(
                                        shortest_remaining.map_or(secs, |s: u64| s.min(secs)),
                                    );
                                }
                            }
                            continue;
                        }
                        if is_lb_blocked(model_id) {
                            // Log: Model skipped due to LB state (Provider_List phase)
                            if let Some(ref hl_config) = primary_hl_config {
                                if let Some(remaining) = self.check_lb_remaining_duration(
                                    model_id,
                                    hl_config,
                                    latency_block_state,
                                    request_context,
                                ) {
                                    let secs = remaining.as_secs();
                                    info!(
                                        "Model {} skipped due to Latency_Block_State, remaining={}s (request_id={})",
                                        model_id, secs, request_context.request_id
                                    );
                                    shortest_remaining = Some(
                                        shortest_remaining.map_or(secs, |s: u64| s.min(secs)),
                                    );
                                }
                            }
                            continue;
                        }
                        return Ok(ProviderSelectionResult::Selected(p));
                    }
                }

                Err(AllProvidersExhaustedError {
                    shortest_remaining_block_seconds: shortest_remaining,
                })
            }
        }
    }

    /// Check if a model is RA-blocked considering both global and request-scoped state.
    fn is_model_ra_blocked(
        &self,
        model_id: &str,
        ra_config: &RetryAfterHandlingConfig,
        retry_after_state: &RetryAfterStateManager,
        request_context: &RequestContext,
    ) -> bool {
        let identifier = match ra_config.scope {
            BlockScope::Model => model_id.to_string(),
            BlockScope::Provider => extract_provider(model_id).to_string(),
        };

        match ra_config.apply_to {
            ApplyTo::Global => retry_after_state.is_blocked(&identifier),
            ApplyTo::Request => {
                if let Some(expires_at) = request_context.request_retry_after_state.get(&identifier)
                {
                    Instant::now() < *expires_at
                } else {
                    false
                }
            }
        }
    }

    /// Get the remaining RA block duration for a model, considering scope and apply_to.
    fn check_ra_remaining_duration(
        &self,
        model_id: &str,
        ra_config: &RetryAfterHandlingConfig,
        retry_after_state: &RetryAfterStateManager,
        request_context: &RequestContext,
    ) -> Option<Duration> {
        let identifier = match ra_config.scope {
            BlockScope::Model => model_id.to_string(),
            BlockScope::Provider => extract_provider(model_id).to_string(),
        };

        match ra_config.apply_to {
            ApplyTo::Global => retry_after_state.remaining_block_duration(&identifier),
            ApplyTo::Request => {
                let now = Instant::now();
                request_context
                    .request_retry_after_state
                    .get(&identifier)
                    .and_then(|expires_at| {
                        if now < *expires_at {
                            Some(*expires_at - now)
                        } else {
                            None
                        }
                    })
            }
        }
    }

    /// Check if a model is LB-blocked considering both global and request-scoped state.
    fn is_model_lb_blocked(
        &self,
        model_id: &str,
        hl_config: &HighLatencyConfig,
        latency_block_state: &LatencyBlockStateManager,
        request_context: &RequestContext,
    ) -> bool {
        let identifier = match hl_config.scope {
            BlockScope::Model => model_id.to_string(),
            BlockScope::Provider => extract_provider(model_id).to_string(),
        };

        match hl_config.apply_to {
            ApplyTo::Global => latency_block_state.is_blocked(&identifier),
            ApplyTo::Request => {
                if let Some(expires_at) =
                    request_context.request_latency_block_state.get(&identifier)
                {
                    Instant::now() < *expires_at
                } else {
                    false
                }
            }
        }
    }

    /// Get the remaining LB block duration for a model, considering scope and apply_to.
    fn check_lb_remaining_duration(
        &self,
        model_id: &str,
        hl_config: &HighLatencyConfig,
        latency_block_state: &LatencyBlockStateManager,
        request_context: &RequestContext,
    ) -> Option<Duration> {
        let identifier = match hl_config.scope {
            BlockScope::Model => model_id.to_string(),
            BlockScope::Provider => extract_provider(model_id).to_string(),
        };

        match hl_config.apply_to {
            ApplyTo::Global => latency_block_state.remaining_block_duration(&identifier),
            ApplyTo::Request => {
                let now = Instant::now();
                request_context
                    .request_latency_block_state
                    .get(&identifier)
                    .and_then(|expires_at| {
                        if now < *expires_at {
                            Some(*expires_at - now)
                        } else {
                            None
                        }
                    })
            }
        }
    }
}

