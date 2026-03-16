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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::{extract_provider, LlmProviderType};
    use proptest::prelude::*;

    fn make_provider(model: &str) -> LlmProvider {
        LlmProvider {
            name: model.to_string(),
            provider_interface: LlmProviderType::OpenAI,
            access_key: None,
            model: Some(model.to_string()),
            default: None,
            stream: None,
            endpoint: None,
            port: None,
            rate_limits: None,
            usage: None,
            routing_preferences: None,
            cluster_name: None,
            base_url_path_prefix: None,
            internal: None,
            passthrough_auth: None,
            retry_policy: None,
        }
    }

    fn stub_context() -> RequestContext {
        use std::collections::HashMap;
        use hyper::HeaderMap;
        use super::super::RequestSignature;

        let sig = RequestSignature::new(b"test", &HeaderMap::new(), false, "test".to_string());
        RequestContext {
            request_id: "test-req".to_string(),
            attempted_providers: HashSet::new(),
            retry_start_time: None,
            attempt_number: 0,
            request_retry_after_state: HashMap::new(),
            request_latency_block_state: HashMap::new(),
            request_signature: sig,
            errors: Vec::new(),
        }
    }

    #[test]
    fn same_model_returns_matching_provider() {
        let providers = vec![
            make_provider("openai/gpt-4o"),
            make_provider("openai/gpt-4o-mini"),
        ];
        let attempted = HashSet::new();
        let ctx = stub_context();
        let selector = ProviderSelector;

        let result = selector.select(
            RetryStrategy::SameModel,
            "openai/gpt-4o",
            &[],
            &providers,
            &attempted,
            &RetryAfterStateManager::new(),
            &LatencyBlockStateManager::new(),
            &ctx,
            false,
            false,
        );

        match result.unwrap() {
            ProviderSelectionResult::Selected(p) => {
                assert_eq!(p.model.as_deref(), Some("openai/gpt-4o"));
            }
            _ => panic!("expected Selected"),
        }
    }

    #[test]
    fn same_model_exhausted_when_already_attempted() {
        let providers = vec![make_provider("openai/gpt-4o")];
        let mut attempted = HashSet::new();
        attempted.insert("openai/gpt-4o".to_string());
        let ctx = stub_context();
        let selector = ProviderSelector;

        let result = selector.select(
            RetryStrategy::SameModel,
            "openai/gpt-4o",
            &[],
            &providers,
            &attempted,
            &RetryAfterStateManager::new(),
            &LatencyBlockStateManager::new(),
            &ctx,
            false,
            false,
        );

        assert!(result.is_err());
    }

    #[test]
    fn same_provider_filters_by_prefix() {
        let providers = vec![
            make_provider("openai/gpt-4o"),
            make_provider("anthropic/claude-3-5-sonnet"),
            make_provider("openai/gpt-4o-mini"),
        ];
        let mut attempted = HashSet::new();
        attempted.insert("openai/gpt-4o".to_string());
        let ctx = stub_context();
        let selector = ProviderSelector;

        let result = selector.select(
            RetryStrategy::SameProvider,
            "openai/gpt-4o",
            &[],
            &providers,
            &attempted,
            &RetryAfterStateManager::new(),
            &LatencyBlockStateManager::new(),
            &ctx,
            false,
            false,
        );

        match result.unwrap() {
            ProviderSelectionResult::Selected(p) => {
                assert_eq!(p.model.as_deref(), Some("openai/gpt-4o-mini"));
            }
            _ => panic!("expected Selected"),
        }
    }

    #[test]
    fn different_provider_filters_by_different_prefix() {
        let providers = vec![
            make_provider("openai/gpt-4o"),
            make_provider("openai/gpt-4o-mini"),
            make_provider("anthropic/claude-3-5-sonnet"),
        ];
        let attempted = HashSet::new();
        let ctx = stub_context();
        let selector = ProviderSelector;

        let result = selector.select(
            RetryStrategy::DifferentProvider,
            "openai/gpt-4o",
            &[],
            &providers,
            &attempted,
            &RetryAfterStateManager::new(),
            &LatencyBlockStateManager::new(),
            &ctx,
            false,
            false,
        );

        match result.unwrap() {
            ProviderSelectionResult::Selected(p) => {
                assert_eq!(p.model.as_deref(), Some("anthropic/claude-3-5-sonnet"));
            }
            _ => panic!("expected Selected"),
        }
    }

    #[test]
    fn different_provider_exhausted_when_all_same_prefix() {
        let providers = vec![
            make_provider("openai/gpt-4o"),
            make_provider("openai/gpt-4o-mini"),
        ];
        let attempted = HashSet::new();
        let ctx = stub_context();
        let selector = ProviderSelector;

        let result = selector.select(
            RetryStrategy::DifferentProvider,
            "openai/gpt-4o",
            &[],
            &providers,
            &attempted,
            &RetryAfterStateManager::new(),
            &LatencyBlockStateManager::new(),
            &ctx,
            false,
            false,
        );

        assert!(result.is_err());
    }

    #[test]
    fn respects_provider_list_ordering() {
        let providers = vec![
            make_provider("anthropic/claude-3-opus"),
            make_provider("anthropic/claude-3-5-sonnet"),
            make_provider("openai/gpt-4o"),
        ];
        let attempted = HashSet::new();
        let ctx = stub_context();
        let selector = ProviderSelector;

        // different_provider from openai should pick the first anthropic in list order
        let result = selector.select(
            RetryStrategy::DifferentProvider,
            "openai/gpt-4o",
            &[],
            &providers,
            &attempted,
            &RetryAfterStateManager::new(),
            &LatencyBlockStateManager::new(),
            &ctx,
            false,
            false,
        );

        match result.unwrap() {
            ProviderSelectionResult::Selected(p) => {
                assert_eq!(p.model.as_deref(), Some("anthropic/claude-3-opus"));
            }
            _ => panic!("expected Selected"),
        }
    }

    #[test]
    fn skips_attempted_and_picks_next() {
        let providers = vec![
            make_provider("anthropic/claude-3-opus"),
            make_provider("anthropic/claude-3-5-sonnet"),
            make_provider("openai/gpt-4o"),
        ];
        let mut attempted = HashSet::new();
        attempted.insert("anthropic/claude-3-opus".to_string());
        let ctx = stub_context();
        let selector = ProviderSelector;

        let result = selector.select(
            RetryStrategy::DifferentProvider,
            "openai/gpt-4o",
            &[],
            &providers,
            &attempted,
            &RetryAfterStateManager::new(),
            &LatencyBlockStateManager::new(),
            &ctx,
            false,
            false,
        );

        match result.unwrap() {
            ProviderSelectionResult::Selected(p) => {
                assert_eq!(p.model.as_deref(), Some("anthropic/claude-3-5-sonnet"));
            }
            _ => panic!("expected Selected"),
        }
    }

    #[test]
    fn all_providers_exhausted_returns_error() {
        let providers = vec![
            make_provider("openai/gpt-4o"),
            make_provider("anthropic/claude-3-5-sonnet"),
        ];
        let mut attempted = HashSet::new();
        attempted.insert("openai/gpt-4o".to_string());
        attempted.insert("anthropic/claude-3-5-sonnet".to_string());
        let ctx = stub_context();
        let selector = ProviderSelector;

        let result = selector.select(
            RetryStrategy::DifferentProvider,
            "openai/gpt-4o",
            &[],
            &providers,
            &attempted,
            &RetryAfterStateManager::new(),
            &LatencyBlockStateManager::new(),
            &ctx,
            false,
            false,
        );

        assert!(result.is_err());
    }

    // ── Fallback models tests (Task 13.1) ─────────────────────────────────

    #[test]
    fn fallback_models_tried_in_order_before_provider_list() {
        // Provider_List has anthropic first, but fallback_models says try azure first.
        let providers = vec![
            make_provider("openai/gpt-4o"),
            make_provider("anthropic/claude-3-5-sonnet"),
            make_provider("azure/gpt-4o"),
        ];
        let mut attempted = HashSet::new();
        attempted.insert("openai/gpt-4o".to_string());
        let ctx = stub_context();
        let selector = ProviderSelector;

        let fallback_models = vec![
            "azure/gpt-4o".to_string(),
            "anthropic/claude-3-5-sonnet".to_string(),
        ];

        let result = selector.select(
            RetryStrategy::DifferentProvider,
            "openai/gpt-4o",
            &fallback_models,
            &providers,
            &attempted,
            &RetryAfterStateManager::new(),
            &LatencyBlockStateManager::new(),
            &ctx,
            false,
            false,
        );

        // Should pick azure/gpt-4o (first in fallback_models) not anthropic (first in Provider_List)
        match result.unwrap() {
            ProviderSelectionResult::Selected(p) => {
                assert_eq!(p.model.as_deref(), Some("azure/gpt-4o"));
            }
            _ => panic!("expected Selected"),
        }
    }

    #[test]
    fn fallback_models_skips_attempted_picks_next() {
        let providers = vec![
            make_provider("openai/gpt-4o"),
            make_provider("anthropic/claude-3-5-sonnet"),
            make_provider("azure/gpt-4o"),
        ];
        let mut attempted = HashSet::new();
        attempted.insert("openai/gpt-4o".to_string());
        attempted.insert("anthropic/claude-3-5-sonnet".to_string());
        let ctx = stub_context();
        let selector = ProviderSelector;

        let fallback_models = vec![
            "anthropic/claude-3-5-sonnet".to_string(),
            "azure/gpt-4o".to_string(),
        ];

        let result = selector.select(
            RetryStrategy::DifferentProvider,
            "openai/gpt-4o",
            &fallback_models,
            &providers,
            &attempted,
            &RetryAfterStateManager::new(),
            &LatencyBlockStateManager::new(),
            &ctx,
            false,
            false,
        );

        match result.unwrap() {
            ProviderSelectionResult::Selected(p) => {
                assert_eq!(p.model.as_deref(), Some("azure/gpt-4o"));
            }
            _ => panic!("expected Selected"),
        }
    }

    #[test]
    fn fallback_models_exhausted_falls_back_to_provider_list() {
        let providers = vec![
            make_provider("openai/gpt-4o"),
            make_provider("anthropic/claude-3-5-sonnet"),
            make_provider("azure/gpt-4o"),
        ];
        let mut attempted = HashSet::new();
        attempted.insert("openai/gpt-4o".to_string());
        attempted.insert("anthropic/claude-3-5-sonnet".to_string());
        let ctx = stub_context();
        let selector = ProviderSelector;

        // Fallback list only has anthropic (already attempted)
        let fallback_models = vec!["anthropic/claude-3-5-sonnet".to_string()];

        let result = selector.select(
            RetryStrategy::DifferentProvider,
            "openai/gpt-4o",
            &fallback_models,
            &providers,
            &attempted,
            &RetryAfterStateManager::new(),
            &LatencyBlockStateManager::new(),
            &ctx,
            false,
            false,
        );

        // Should fall back to Provider_List and find azure/gpt-4o
        match result.unwrap() {
            ProviderSelectionResult::Selected(p) => {
                assert_eq!(p.model.as_deref(), Some("azure/gpt-4o"));
            }
            _ => panic!("expected Selected"),
        }
    }

    #[test]
    fn fallback_models_not_in_provider_list_skipped() {
        let providers = vec![
            make_provider("openai/gpt-4o"),
            make_provider("anthropic/claude-3-5-sonnet"),
        ];
        let mut attempted = HashSet::new();
        attempted.insert("openai/gpt-4o".to_string());
        let ctx = stub_context();
        let selector = ProviderSelector;

        // "azure/gpt-4o" is in fallback_models but NOT in Provider_List
        let fallback_models = vec![
            "azure/gpt-4o".to_string(),
            "anthropic/claude-3-5-sonnet".to_string(),
        ];

        let result = selector.select(
            RetryStrategy::DifferentProvider,
            "openai/gpt-4o",
            &fallback_models,
            &providers,
            &attempted,
            &RetryAfterStateManager::new(),
            &LatencyBlockStateManager::new(),
            &ctx,
            false,
            false,
        );

        // azure/gpt-4o skipped (not in Provider_List), picks anthropic from fallback list
        match result.unwrap() {
            ProviderSelectionResult::Selected(p) => {
                assert_eq!(p.model.as_deref(), Some("anthropic/claude-3-5-sonnet"));
            }
            _ => panic!("expected Selected"),
        }
    }

    #[test]
    fn fallback_models_strategy_filtering_same_provider() {
        let providers = vec![
            make_provider("openai/gpt-4o"),
            make_provider("openai/gpt-4o-mini"),
            make_provider("anthropic/claude-3-5-sonnet"),
        ];
        let mut attempted = HashSet::new();
        attempted.insert("openai/gpt-4o".to_string());
        let ctx = stub_context();
        let selector = ProviderSelector;

        // Fallback list has anthropic first, but strategy is same_provider
        let fallback_models = vec![
            "anthropic/claude-3-5-sonnet".to_string(),
            "openai/gpt-4o-mini".to_string(),
        ];

        let result = selector.select(
            RetryStrategy::SameProvider,
            "openai/gpt-4o",
            &fallback_models,
            &providers,
            &attempted,
            &RetryAfterStateManager::new(),
            &LatencyBlockStateManager::new(),
            &ctx,
            false,
            false,
        );

        // anthropic filtered out by same_provider strategy, picks openai/gpt-4o-mini
        match result.unwrap() {
            ProviderSelectionResult::Selected(p) => {
                assert_eq!(p.model.as_deref(), Some("openai/gpt-4o-mini"));
            }
            _ => panic!("expected Selected"),
        }
    }

    #[test]
    fn fallback_models_strategy_filtering_different_provider() {
        let providers = vec![
            make_provider("openai/gpt-4o"),
            make_provider("openai/gpt-4o-mini"),
            make_provider("anthropic/claude-3-5-sonnet"),
        ];
        let mut attempted = HashSet::new();
        attempted.insert("openai/gpt-4o".to_string());
        let ctx = stub_context();
        let selector = ProviderSelector;

        // Fallback list has openai/gpt-4o-mini first, but strategy is different_provider
        let fallback_models = vec![
            "openai/gpt-4o-mini".to_string(),
            "anthropic/claude-3-5-sonnet".to_string(),
        ];

        let result = selector.select(
            RetryStrategy::DifferentProvider,
            "openai/gpt-4o",
            &fallback_models,
            &providers,
            &attempted,
            &RetryAfterStateManager::new(),
            &LatencyBlockStateManager::new(),
            &ctx,
            false,
            false,
        );

        // openai/gpt-4o-mini filtered out by different_provider strategy, picks anthropic
        match result.unwrap() {
            ProviderSelectionResult::Selected(p) => {
                assert_eq!(p.model.as_deref(), Some("anthropic/claude-3-5-sonnet"));
            }
            _ => panic!("expected Selected"),
        }
    }

    #[test]
    fn same_model_ignores_fallback_models() {
        let providers = vec![
            make_provider("openai/gpt-4o"),
            make_provider("anthropic/claude-3-5-sonnet"),
        ];
        let attempted = HashSet::new();
        let ctx = stub_context();
        let selector = ProviderSelector;

        let fallback_models = vec!["anthropic/claude-3-5-sonnet".to_string()];

        let result = selector.select(
            RetryStrategy::SameModel,
            "openai/gpt-4o",
            &fallback_models,
            &providers,
            &attempted,
            &RetryAfterStateManager::new(),
            &LatencyBlockStateManager::new(),
            &ctx,
            false,
            false,
        );

        // SameModel always returns the primary model, ignoring fallback_models
        match result.unwrap() {
            ProviderSelectionResult::Selected(p) => {
                assert_eq!(p.model.as_deref(), Some("openai/gpt-4o"));
            }
            _ => panic!("expected Selected"),
        }
    }

    #[test]
    fn fallback_all_exhausted_and_provider_list_exhausted() {
        let providers = vec![
            make_provider("openai/gpt-4o"),
            make_provider("anthropic/claude-3-5-sonnet"),
        ];
        let mut attempted = HashSet::new();
        attempted.insert("openai/gpt-4o".to_string());
        attempted.insert("anthropic/claude-3-5-sonnet".to_string());
        let ctx = stub_context();
        let selector = ProviderSelector;

        let fallback_models = vec!["anthropic/claude-3-5-sonnet".to_string()];

        let result = selector.select(
            RetryStrategy::DifferentProvider,
            "openai/gpt-4o",
            &fallback_models,
            &providers,
            &attempted,
            &RetryAfterStateManager::new(),
            &LatencyBlockStateManager::new(),
            &ctx,
            false,
            false,
        );

        assert!(result.is_err());
    }

    #[test]
    fn empty_fallback_models_uses_provider_list() {
        // Verify backward compatibility: empty fallback_models behaves like P0
        let providers = vec![
            make_provider("openai/gpt-4o"),
            make_provider("anthropic/claude-3-5-sonnet"),
            make_provider("azure/gpt-4o"),
        ];
        let mut attempted = HashSet::new();
        attempted.insert("openai/gpt-4o".to_string());
        let ctx = stub_context();
        let selector = ProviderSelector;

        let result = selector.select(
            RetryStrategy::DifferentProvider,
            "openai/gpt-4o",
            &[],
            &providers,
            &attempted,
            &RetryAfterStateManager::new(),
            &LatencyBlockStateManager::new(),
            &ctx,
            false,
            false,
        );

        // Should pick anthropic (first different-provider in Provider_List order)
        match result.unwrap() {
            ProviderSelectionResult::Selected(p) => {
                assert_eq!(p.model.as_deref(), Some("anthropic/claude-3-5-sonnet"));
            }
            _ => panic!("expected Selected"),
        }
    }

    // ── Retry-After state integration tests (Task 17.1) ──────────────────

    use crate::configuration::{HighLatencyConfig, LatencyMeasure, RetryPolicy, RetryAfterHandlingConfig};

    fn make_provider_with_retry_policy(model: &str, ra_config: Option<RetryAfterHandlingConfig>) -> LlmProvider {
        let mut p = make_provider(model);
        p.retry_policy = Some(RetryPolicy {
            fallback_models: vec![],
            default_strategy: RetryStrategy::DifferentProvider,
            default_max_attempts: 2,
            on_status_codes: vec![],
            on_timeout: None,
            on_high_latency: None,
            backoff: None,
            retry_after_handling: ra_config,
            max_retry_duration_ms: None,
        });
        p
    }

    #[test]
    fn same_model_global_ra_block_returns_wait_and_retry() {
        // When same_model strategy and model is globally RA-blocked,
        // select() should return WaitAndRetrySameModel with remaining duration.
        let providers = vec![
            make_provider_with_retry_policy("openai/gpt-4o", None), // defaults: scope=Model, apply_to=Global
        ];
        let attempted = HashSet::new();
        let ctx = stub_context();
        let selector = ProviderSelector;
        let ra_state = RetryAfterStateManager::new();

        // Block the model globally for 60 seconds
        ra_state.record("openai/gpt-4o", 60, 300);

        let result = selector.select(
            RetryStrategy::SameModel,
            "openai/gpt-4o",
            &[],
            &providers,
            &attempted,
            &ra_state,
            &LatencyBlockStateManager::new(),
            &ctx,
            true,
            false,
        );

        match result.unwrap() {
            ProviderSelectionResult::WaitAndRetrySameModel { wait_duration } => {
                // Should have a positive remaining duration
                assert!(wait_duration.as_secs() > 0, "wait_duration should be positive");
                assert!(wait_duration.as_secs() <= 60, "wait_duration should be <= 60s");
            }
            _ => panic!("expected WaitAndRetrySameModel"),
        }
    }

    #[test]
    fn same_model_no_ra_block_returns_selected() {
        // When same_model strategy and model is NOT RA-blocked,
        // select() should return Selected.
        let providers = vec![
            make_provider_with_retry_policy("openai/gpt-4o", None),
        ];
        let attempted = HashSet::new();
        let ctx = stub_context();
        let selector = ProviderSelector;
        let ra_state = RetryAfterStateManager::new();

        let result = selector.select(
            RetryStrategy::SameModel,
            "openai/gpt-4o",
            &[],
            &providers,
            &attempted,
            &ra_state,
            &LatencyBlockStateManager::new(),
            &ctx,
            true,
            false,
        );

        match result.unwrap() {
            ProviderSelectionResult::Selected(p) => {
                assert_eq!(p.model.as_deref(), Some("openai/gpt-4o"));
            }
            _ => panic!("expected Selected"),
        }
    }

    #[test]
    fn same_model_ra_block_ignored_when_has_retry_policy_false() {
        // When has_retry_policy is false, RA state should not be checked.
        let providers = vec![
            make_provider_with_retry_policy("openai/gpt-4o", None),
        ];
        let attempted = HashSet::new();
        let ctx = stub_context();
        let selector = ProviderSelector;
        let ra_state = RetryAfterStateManager::new();

        // Block the model globally
        ra_state.record("openai/gpt-4o", 60, 300);

        let result = selector.select(
            RetryStrategy::SameModel,
            "openai/gpt-4o",
            &[],
            &providers,
            &attempted,
            &ra_state,
            &LatencyBlockStateManager::new(),
            &ctx,
            false, // has_retry_policy = false
            false,
        );

        // Should return Selected despite the block, because has_retry_policy is false
        match result.unwrap() {
            ProviderSelectionResult::Selected(p) => {
                assert_eq!(p.model.as_deref(), Some("openai/gpt-4o"));
            }
            _ => panic!("expected Selected when has_retry_policy is false"),
        }
    }

    #[test]
    fn same_model_request_scoped_ra_block_returns_wait_and_retry() {
        // When same_model strategy and model is request-scoped RA-blocked,
        // select() should return WaitAndRetrySameModel.
        let ra_config = RetryAfterHandlingConfig {
            scope: BlockScope::Model,
            apply_to: ApplyTo::Request,
            max_retry_after_seconds: 300,
        };
        let providers = vec![
            make_provider_with_retry_policy("openai/gpt-4o", Some(ra_config)),
        ];
        let attempted = HashSet::new();
        let mut ctx = stub_context();
        // Add request-scoped block
        ctx.request_retry_after_state.insert(
            "openai/gpt-4o".to_string(),
            Instant::now() + Duration::from_secs(30),
        );
        let selector = ProviderSelector;
        let ra_state = RetryAfterStateManager::new();

        let result = selector.select(
            RetryStrategy::SameModel,
            "openai/gpt-4o",
            &[],
            &providers,
            &attempted,
            &ra_state,
            &LatencyBlockStateManager::new(),
            &ctx,
            true,
            false,
        );

        match result.unwrap() {
            ProviderSelectionResult::WaitAndRetrySameModel { wait_duration } => {
                assert!(wait_duration.as_secs() > 0);
                assert!(wait_duration.as_secs() <= 30);
            }
            _ => panic!("expected WaitAndRetrySameModel for request-scoped block"),
        }
    }

    #[test]
    fn different_provider_skips_ra_blocked_candidate() {
        // When different_provider strategy and a candidate is RA-blocked,
        // it should be skipped and the next eligible candidate selected.
        let providers = vec![
            make_provider_with_retry_policy("openai/gpt-4o", None),
            make_provider("anthropic/claude-3-5-sonnet"),
            make_provider("azure/gpt-4o"),
        ];
        let mut attempted = HashSet::new();
        attempted.insert("openai/gpt-4o".to_string());
        let ctx = stub_context();
        let selector = ProviderSelector;
        let ra_state = RetryAfterStateManager::new();

        // Block anthropic model globally (scope: Model by default)
        ra_state.record("anthropic/claude-3-5-sonnet", 60, 300);

        let result = selector.select(
            RetryStrategy::DifferentProvider,
            "openai/gpt-4o",
            &[],
            &providers,
            &attempted,
            &ra_state,
            &LatencyBlockStateManager::new(),
            &ctx,
            true,
            false,
        );

        // Should skip anthropic (blocked) and pick azure
        match result.unwrap() {
            ProviderSelectionResult::Selected(p) => {
                assert_eq!(p.model.as_deref(), Some("azure/gpt-4o"));
            }
            _ => panic!("expected Selected"),
        }
    }

    #[test]
    fn provider_scope_blocks_all_models_from_provider() {
        // When scope is Provider, blocking "openai" should block all openai/* models.
        let ra_config = RetryAfterHandlingConfig {
            scope: BlockScope::Provider,
            apply_to: ApplyTo::Global,
            max_retry_after_seconds: 300,
        };
        let providers = vec![
            make_provider_with_retry_policy("openai/gpt-4o", Some(ra_config)),
            make_provider("openai/gpt-4o-mini"),
            make_provider("anthropic/claude-3-5-sonnet"),
        ];
        let mut attempted = HashSet::new();
        attempted.insert("openai/gpt-4o".to_string());
        let ctx = stub_context();
        let selector = ProviderSelector;
        let ra_state = RetryAfterStateManager::new();

        // Block at provider level: "openai"
        ra_state.record("openai", 60, 300);

        let result = selector.select(
            RetryStrategy::SameProvider,
            "openai/gpt-4o",
            &[],
            &providers,
            &attempted,
            &ra_state,
            &LatencyBlockStateManager::new(),
            &ctx,
            true,
            false,
        );

        // openai/gpt-4o-mini should be blocked because provider "openai" is blocked
        // No same-provider candidates available → error
        assert!(result.is_err());
    }

    #[test]
    fn fallback_model_ra_blocked_skipped() {
        // When a fallback model is RA-blocked, it should be skipped.
        let providers = vec![
            make_provider_with_retry_policy("openai/gpt-4o", None),
            make_provider("anthropic/claude-3-5-sonnet"),
            make_provider("azure/gpt-4o"),
        ];
        let mut attempted = HashSet::new();
        attempted.insert("openai/gpt-4o".to_string());
        let ctx = stub_context();
        let selector = ProviderSelector;
        let ra_state = RetryAfterStateManager::new();

        // Block anthropic model
        ra_state.record("anthropic/claude-3-5-sonnet", 60, 300);

        let fallback_models = vec![
            "anthropic/claude-3-5-sonnet".to_string(),
            "azure/gpt-4o".to_string(),
        ];

        let result = selector.select(
            RetryStrategy::DifferentProvider,
            "openai/gpt-4o",
            &fallback_models,
            &providers,
            &attempted,
            &ra_state,
            &LatencyBlockStateManager::new(),
            &ctx,
            true,
            false,
        );

        // anthropic blocked → skip to azure
        match result.unwrap() {
            ProviderSelectionResult::Selected(p) => {
                assert_eq!(p.model.as_deref(), Some("azure/gpt-4o"));
            }
            _ => panic!("expected Selected"),
        }
    }

    #[test]
    fn all_candidates_ra_blocked_returns_error_with_shortest_remaining() {
        // When all candidates are RA-blocked, return error with shortest_remaining_block_seconds.
        let providers = vec![
            make_provider_with_retry_policy("openai/gpt-4o", None),
            make_provider("anthropic/claude-3-5-sonnet"),
            make_provider("azure/gpt-4o"),
        ];
        let mut attempted = HashSet::new();
        attempted.insert("openai/gpt-4o".to_string());
        let ctx = stub_context();
        let selector = ProviderSelector;
        let ra_state = RetryAfterStateManager::new();

        // Block both alternative providers
        ra_state.record("anthropic/claude-3-5-sonnet", 60, 300);
        ra_state.record("azure/gpt-4o", 30, 300);

        let fallback_models = vec![
            "anthropic/claude-3-5-sonnet".to_string(),
            "azure/gpt-4o".to_string(),
        ];

        let result = selector.select(
            RetryStrategy::DifferentProvider,
            "openai/gpt-4o",
            &fallback_models,
            &providers,
            &attempted,
            &ra_state,
            &LatencyBlockStateManager::new(),
            &ctx,
            true,
            false,
        );

        match result {
            Err(e) => {
                // shortest_remaining should be set (azure has 30s, anthropic has 60s)
                assert!(e.shortest_remaining_block_seconds.is_some());
                let shortest = e.shortest_remaining_block_seconds.unwrap();
                assert!(shortest <= 30, "shortest remaining should be <= 30s, got {}", shortest);
            }
            Ok(_) => panic!("expected AllProvidersExhaustedError"),
        }
    }

    #[test]
    fn same_model_provider_scope_global_ra_block_returns_wait() {
        // When same_model strategy with provider-scope RA block,
        // blocking the provider should trigger WaitAndRetrySameModel.
        let ra_config = RetryAfterHandlingConfig {
            scope: BlockScope::Provider,
            apply_to: ApplyTo::Global,
            max_retry_after_seconds: 300,
        };
        let providers = vec![
            make_provider_with_retry_policy("openai/gpt-4o", Some(ra_config)),
        ];
        let attempted = HashSet::new();
        let ctx = stub_context();
        let selector = ProviderSelector;
        let ra_state = RetryAfterStateManager::new();

        // Block at provider level
        ra_state.record("openai", 45, 300);

        let result = selector.select(
            RetryStrategy::SameModel,
            "openai/gpt-4o",
            &[],
            &providers,
            &attempted,
            &ra_state,
            &LatencyBlockStateManager::new(),
            &ctx,
            true,
            false,
        );

        match result.unwrap() {
            ProviderSelectionResult::WaitAndRetrySameModel { wait_duration } => {
                assert!(wait_duration.as_secs() > 0);
                assert!(wait_duration.as_secs() <= 45);
            }
            _ => panic!("expected WaitAndRetrySameModel for provider-scope block"),
        }
    }

    // ── Latency Block state integration tests (Task 23.1) ────────────────

    fn make_hl_config(scope: BlockScope, apply_to: ApplyTo) -> HighLatencyConfig {
        HighLatencyConfig {
            threshold_ms: 5000,
            measure: LatencyMeasure::Ttfb,
            min_triggers: 1,
            trigger_window_seconds: None,
            strategy: RetryStrategy::DifferentProvider,
            max_attempts: 2,
            block_duration_seconds: 300,
            scope,
            apply_to,
        }
    }

    fn make_provider_with_hl_config(
        model: &str,
        ra_config: Option<RetryAfterHandlingConfig>,
        hl_config: Option<HighLatencyConfig>,
    ) -> LlmProvider {
        let mut p = make_provider(model);
        p.retry_policy = Some(RetryPolicy {
            fallback_models: vec![],
            default_strategy: RetryStrategy::DifferentProvider,
            default_max_attempts: 2,
            on_status_codes: vec![],
            on_timeout: None,
            on_high_latency: hl_config,
            backoff: None,
            retry_after_handling: ra_config,
            max_retry_duration_ms: None,
        });
        p
    }

    #[test]
    fn same_model_lb_block_returns_error_not_wait() {
        // For same_model strategy with LB block: return AllProvidersExhaustedError
        // (skip to alternative), NOT WaitAndRetrySameModel (unlike RA).
        let hl_config = make_hl_config(BlockScope::Model, ApplyTo::Global);
        let providers = vec![make_provider_with_hl_config(
            "openai/gpt-4o",
            None,
            Some(hl_config),
        )];
        let attempted = HashSet::new();
        let ctx = stub_context();
        let selector = ProviderSelector;
        let lb_state = LatencyBlockStateManager::new();

        // Block the model globally via LB
        lb_state.record_block("openai/gpt-4o", 60, 6000);

        let result = selector.select(
            RetryStrategy::SameModel,
            "openai/gpt-4o",
            &[],
            &providers,
            &attempted,
            &RetryAfterStateManager::new(),
            &lb_state,
            &ctx,
            true,
            true,
        );

        // Should return AllProvidersExhaustedError, NOT WaitAndRetrySameModel
        match result {
            Err(e) => {
                assert!(
                    e.shortest_remaining_block_seconds.is_some(),
                    "should include remaining block seconds"
                );
                let secs = e.shortest_remaining_block_seconds.unwrap();
                assert!(secs > 0 && secs <= 60);
            }
            Ok(ProviderSelectionResult::WaitAndRetrySameModel { .. }) => {
                panic!("LB block on same_model should NOT return WaitAndRetrySameModel");
            }
            Ok(ProviderSelectionResult::Selected(_)) => {
                panic!("LB-blocked model should not be Selected");
            }
        }
    }

    #[test]
    fn same_model_no_lb_block_returns_selected() {
        // When same_model strategy and model is NOT LB-blocked, returns Selected.
        let hl_config = make_hl_config(BlockScope::Model, ApplyTo::Global);
        let providers = vec![make_provider_with_hl_config(
            "openai/gpt-4o",
            None,
            Some(hl_config),
        )];
        let attempted = HashSet::new();
        let ctx = stub_context();
        let selector = ProviderSelector;

        let result = selector.select(
            RetryStrategy::SameModel,
            "openai/gpt-4o",
            &[],
            &providers,
            &attempted,
            &RetryAfterStateManager::new(),
            &LatencyBlockStateManager::new(),
            &ctx,
            true,
            true,
        );

        match result.unwrap() {
            ProviderSelectionResult::Selected(p) => {
                assert_eq!(p.model.as_deref(), Some("openai/gpt-4o"));
            }
            _ => panic!("expected Selected when not LB-blocked"),
        }
    }

    #[test]
    fn same_model_lb_block_ignored_when_has_high_latency_config_false() {
        // When has_high_latency_config is false, LB state should not be checked.
        let hl_config = make_hl_config(BlockScope::Model, ApplyTo::Global);
        let providers = vec![make_provider_with_hl_config(
            "openai/gpt-4o",
            None,
            Some(hl_config),
        )];
        let attempted = HashSet::new();
        let ctx = stub_context();
        let selector = ProviderSelector;
        let lb_state = LatencyBlockStateManager::new();

        lb_state.record_block("openai/gpt-4o", 60, 6000);

        let result = selector.select(
            RetryStrategy::SameModel,
            "openai/gpt-4o",
            &[],
            &providers,
            &attempted,
            &RetryAfterStateManager::new(),
            &lb_state,
            &ctx,
            false,
            false, // has_high_latency_config = false
        );

        match result.unwrap() {
            ProviderSelectionResult::Selected(p) => {
                assert_eq!(p.model.as_deref(), Some("openai/gpt-4o"));
            }
            _ => panic!("expected Selected when has_high_latency_config is false"),
        }
    }

    #[test]
    fn same_model_request_scoped_lb_block_returns_error() {
        // When same_model strategy and model is request-scoped LB-blocked,
        // returns AllProvidersExhaustedError.
        let hl_config = make_hl_config(BlockScope::Model, ApplyTo::Request);
        let providers = vec![make_provider_with_hl_config(
            "openai/gpt-4o",
            None,
            Some(hl_config),
        )];
        let attempted = HashSet::new();
        let mut ctx = stub_context();
        // Add request-scoped LB block
        ctx.request_latency_block_state.insert(
            "openai/gpt-4o".to_string(),
            Instant::now() + Duration::from_secs(30),
        );
        let selector = ProviderSelector;

        let result = selector.select(
            RetryStrategy::SameModel,
            "openai/gpt-4o",
            &[],
            &providers,
            &attempted,
            &RetryAfterStateManager::new(),
            &LatencyBlockStateManager::new(),
            &ctx,
            true,
            true,
        );

        assert!(result.is_err(), "request-scoped LB block should return error for same_model");
    }

    #[test]
    fn different_provider_skips_lb_blocked_candidate() {
        // When different_provider strategy and a candidate is LB-blocked,
        // it should be skipped and the next eligible candidate selected.
        let hl_config = make_hl_config(BlockScope::Model, ApplyTo::Global);
        let providers = vec![
            make_provider_with_hl_config("openai/gpt-4o", None, Some(hl_config)),
            make_provider("anthropic/claude-3-5-sonnet"),
            make_provider("azure/gpt-4o"),
        ];
        let mut attempted = HashSet::new();
        attempted.insert("openai/gpt-4o".to_string());
        let ctx = stub_context();
        let selector = ProviderSelector;
        let lb_state = LatencyBlockStateManager::new();

        // Block anthropic model globally via LB
        lb_state.record_block("anthropic/claude-3-5-sonnet", 60, 6000);

        let result = selector.select(
            RetryStrategy::DifferentProvider,
            "openai/gpt-4o",
            &[],
            &providers,
            &attempted,
            &RetryAfterStateManager::new(),
            &lb_state,
            &ctx,
            true,
            true,
        );

        // Should skip anthropic (LB-blocked) and pick azure
        match result.unwrap() {
            ProviderSelectionResult::Selected(p) => {
                assert_eq!(p.model.as_deref(), Some("azure/gpt-4o"));
            }
            _ => panic!("expected Selected"),
        }
    }

    #[test]
    fn provider_scope_lb_blocks_all_models_from_provider() {
        // When LB scope is Provider, blocking "openai" should block all openai/* models.
        let hl_config = make_hl_config(BlockScope::Provider, ApplyTo::Global);
        let providers = vec![
            make_provider_with_hl_config("openai/gpt-4o", None, Some(hl_config)),
            make_provider("openai/gpt-4o-mini"),
            make_provider("anthropic/claude-3-5-sonnet"),
        ];
        let mut attempted = HashSet::new();
        attempted.insert("openai/gpt-4o".to_string());
        let ctx = stub_context();
        let selector = ProviderSelector;
        let lb_state = LatencyBlockStateManager::new();

        // Block at provider level: "openai"
        lb_state.record_block("openai", 60, 6000);

        let result = selector.select(
            RetryStrategy::SameProvider,
            "openai/gpt-4o",
            &[],
            &providers,
            &attempted,
            &RetryAfterStateManager::new(),
            &lb_state,
            &ctx,
            true,
            true,
        );

        // openai/gpt-4o-mini should be blocked because provider "openai" is LB-blocked
        assert!(result.is_err());
    }

    #[test]
    fn fallback_model_lb_blocked_skipped() {
        // When a fallback model is LB-blocked, it should be skipped.
        let hl_config = make_hl_config(BlockScope::Model, ApplyTo::Global);
        let providers = vec![
            make_provider_with_hl_config("openai/gpt-4o", None, Some(hl_config)),
            make_provider("anthropic/claude-3-5-sonnet"),
            make_provider("azure/gpt-4o"),
        ];
        let mut attempted = HashSet::new();
        attempted.insert("openai/gpt-4o".to_string());
        let ctx = stub_context();
        let selector = ProviderSelector;
        let lb_state = LatencyBlockStateManager::new();

        // Block anthropic model via LB
        lb_state.record_block("anthropic/claude-3-5-sonnet", 60, 6000);

        let fallback_models = vec![
            "anthropic/claude-3-5-sonnet".to_string(),
            "azure/gpt-4o".to_string(),
        ];

        let result = selector.select(
            RetryStrategy::DifferentProvider,
            "openai/gpt-4o",
            &fallback_models,
            &providers,
            &attempted,
            &RetryAfterStateManager::new(),
            &lb_state,
            &ctx,
            true,
            true,
        );

        // anthropic LB-blocked → skip to azure
        match result.unwrap() {
            ProviderSelectionResult::Selected(p) => {
                assert_eq!(p.model.as_deref(), Some("azure/gpt-4o"));
            }
            _ => panic!("expected Selected"),
        }
    }

    #[test]
    fn both_ra_and_lb_block_skips_candidate() {
        // When both RA and LB block a candidate, skip it (either block is sufficient).
        let hl_config = make_hl_config(BlockScope::Model, ApplyTo::Global);
        let providers = vec![
            make_provider_with_hl_config("openai/gpt-4o", None, Some(hl_config)),
            make_provider("anthropic/claude-3-5-sonnet"),
            make_provider("azure/gpt-4o"),
        ];
        let mut attempted = HashSet::new();
        attempted.insert("openai/gpt-4o".to_string());
        let ctx = stub_context();
        let selector = ProviderSelector;
        let ra_state = RetryAfterStateManager::new();
        let lb_state = LatencyBlockStateManager::new();

        // Block anthropic via BOTH RA and LB
        ra_state.record("anthropic/claude-3-5-sonnet", 60, 300);
        lb_state.record_block("anthropic/claude-3-5-sonnet", 60, 6000);

        let result = selector.select(
            RetryStrategy::DifferentProvider,
            "openai/gpt-4o",
            &[],
            &providers,
            &attempted,
            &ra_state,
            &lb_state,
            &ctx,
            true,
            true,
        );

        // Should skip anthropic (both blocked) and pick azure
        match result.unwrap() {
            ProviderSelectionResult::Selected(p) => {
                assert_eq!(p.model.as_deref(), Some("azure/gpt-4o"));
            }
            _ => panic!("expected Selected"),
        }
    }

    #[test]
    fn ra_only_block_still_skips_when_lb_not_blocked() {
        // When only RA blocks a candidate (LB does not), still skip it.
        let hl_config = make_hl_config(BlockScope::Model, ApplyTo::Global);
        let providers = vec![
            make_provider_with_hl_config("openai/gpt-4o", None, Some(hl_config)),
            make_provider("anthropic/claude-3-5-sonnet"),
            make_provider("azure/gpt-4o"),
        ];
        let mut attempted = HashSet::new();
        attempted.insert("openai/gpt-4o".to_string());
        let ctx = stub_context();
        let selector = ProviderSelector;
        let ra_state = RetryAfterStateManager::new();

        // Block anthropic via RA only
        ra_state.record("anthropic/claude-3-5-sonnet", 60, 300);

        let result = selector.select(
            RetryStrategy::DifferentProvider,
            "openai/gpt-4o",
            &[],
            &providers,
            &attempted,
            &ra_state,
            &LatencyBlockStateManager::new(),
            &ctx,
            true,
            true,
        );

        // Should skip anthropic (RA-blocked) and pick azure
        match result.unwrap() {
            ProviderSelectionResult::Selected(p) => {
                assert_eq!(p.model.as_deref(), Some("azure/gpt-4o"));
            }
            _ => panic!("expected Selected"),
        }
    }

    #[test]
    fn lb_only_block_still_skips_when_ra_not_blocked() {
        // When only LB blocks a candidate (RA does not), still skip it.
        let hl_config = make_hl_config(BlockScope::Model, ApplyTo::Global);
        let providers = vec![
            make_provider_with_hl_config("openai/gpt-4o", None, Some(hl_config)),
            make_provider("anthropic/claude-3-5-sonnet"),
            make_provider("azure/gpt-4o"),
        ];
        let mut attempted = HashSet::new();
        attempted.insert("openai/gpt-4o".to_string());
        let ctx = stub_context();
        let selector = ProviderSelector;
        let lb_state = LatencyBlockStateManager::new();

        // Block anthropic via LB only
        lb_state.record_block("anthropic/claude-3-5-sonnet", 60, 6000);

        let result = selector.select(
            RetryStrategy::DifferentProvider,
            "openai/gpt-4o",
            &[],
            &providers,
            &attempted,
            &RetryAfterStateManager::new(),
            &lb_state,
            &ctx,
            true,
            true,
        );

        // Should skip anthropic (LB-blocked) and pick azure
        match result.unwrap() {
            ProviderSelectionResult::Selected(p) => {
                assert_eq!(p.model.as_deref(), Some("azure/gpt-4o"));
            }
            _ => panic!("expected Selected"),
        }
    }

    #[test]
    fn all_candidates_lb_blocked_returns_error_with_shortest_remaining() {
        // When all candidates are LB-blocked, return error with shortest_remaining_block_seconds.
        let hl_config = make_hl_config(BlockScope::Model, ApplyTo::Global);
        let providers = vec![
            make_provider_with_hl_config("openai/gpt-4o", None, Some(hl_config)),
            make_provider("anthropic/claude-3-5-sonnet"),
            make_provider("azure/gpt-4o"),
        ];
        let mut attempted = HashSet::new();
        attempted.insert("openai/gpt-4o".to_string());
        let ctx = stub_context();
        let selector = ProviderSelector;
        let lb_state = LatencyBlockStateManager::new();

        // Block both alternative providers via LB
        lb_state.record_block("anthropic/claude-3-5-sonnet", 60, 6000);
        lb_state.record_block("azure/gpt-4o", 30, 6000);

        let result = selector.select(
            RetryStrategy::DifferentProvider,
            "openai/gpt-4o",
            &[],
            &providers,
            &attempted,
            &RetryAfterStateManager::new(),
            &lb_state,
            &ctx,
            true,
            true,
        );

        match result {
            Err(e) => {
                assert!(e.shortest_remaining_block_seconds.is_some());
                let shortest = e.shortest_remaining_block_seconds.unwrap();
                assert!(shortest <= 30, "shortest remaining should be <= 30s, got {}", shortest);
            }
            Ok(_) => panic!("expected AllProvidersExhaustedError"),
        }
    }

    #[test]
    fn same_model_both_ra_and_lb_blocked_ra_takes_precedence() {
        // When same_model and both RA and LB block the model,
        // RA check happens first → returns WaitAndRetrySameModel.
        let hl_config = make_hl_config(BlockScope::Model, ApplyTo::Global);
        let ra_config = RetryAfterHandlingConfig {
            scope: BlockScope::Model,
            apply_to: ApplyTo::Global,
            max_retry_after_seconds: 300,
        };
        let providers = vec![make_provider_with_hl_config(
            "openai/gpt-4o",
            Some(ra_config),
            Some(hl_config),
        )];
        let attempted = HashSet::new();
        let ctx = stub_context();
        let selector = ProviderSelector;
        let ra_state = RetryAfterStateManager::new();
        let lb_state = LatencyBlockStateManager::new();

        // Block via both RA and LB
        ra_state.record("openai/gpt-4o", 60, 300);
        lb_state.record_block("openai/gpt-4o", 60, 6000);

        let result = selector.select(
            RetryStrategy::SameModel,
            "openai/gpt-4o",
            &[],
            &providers,
            &attempted,
            &ra_state,
            &lb_state,
            &ctx,
            true,
            true,
        );

        // RA check happens first → WaitAndRetrySameModel
        match result.unwrap() {
            ProviderSelectionResult::WaitAndRetrySameModel { wait_duration } => {
                assert!(wait_duration.as_secs() > 0);
            }
            _ => panic!("expected WaitAndRetrySameModel when both RA and LB block same_model"),
        }
    }

    #[test]
    fn same_model_provider_scope_lb_block_returns_error() {
        // When same_model strategy with provider-scope LB block,
        // blocking the provider should return AllProvidersExhaustedError.
        let hl_config = make_hl_config(BlockScope::Provider, ApplyTo::Global);
        let providers = vec![make_provider_with_hl_config(
            "openai/gpt-4o",
            None,
            Some(hl_config),
        )];
        let attempted = HashSet::new();
        let ctx = stub_context();
        let selector = ProviderSelector;
        let lb_state = LatencyBlockStateManager::new();

        // Block at provider level
        lb_state.record_block("openai", 45, 6000);

        let result = selector.select(
            RetryStrategy::SameModel,
            "openai/gpt-4o",
            &[],
            &providers,
            &attempted,
            &RetryAfterStateManager::new(),
            &lb_state,
            &ctx,
            true,
            true,
        );

        match result {
            Err(e) => {
                assert!(e.shortest_remaining_block_seconds.is_some());
                let secs = e.shortest_remaining_block_seconds.unwrap();
                assert!(secs > 0 && secs <= 45);
            }
            Ok(_) => panic!("expected AllProvidersExhaustedError for provider-scope LB block"),
        }
    }

    // --- Proptest strategies ---

    /// Generates a provider prefix from a fixed set.
    fn arb_prefix() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("openai".to_string()),
            Just("anthropic".to_string()),
            Just("azure".to_string()),
        ]
    }

    /// Generates a model identifier like "openai/gpt-4o".
    fn arb_model_id() -> impl Strategy<Value = String> {
        (arb_prefix(), prop_oneof![
            Just("model-a".to_string()),
            Just("model-b".to_string()),
            Just("model-c".to_string()),
        ])
        .prop_map(|(prefix, model)| format!("{}/{}", prefix, model))
    }

    /// Generates a non-empty list of providers (1..=6).
    fn arb_provider_list() -> impl Strategy<Value = Vec<LlmProvider>> {
        proptest::collection::vec(arb_model_id(), 1..=6)
            .prop_map(|ids| ids.into_iter().map(|id| make_provider(&id)).collect())
    }



    // Feature: retry-on-ratelimit, Property 11: Strategy-Correct Provider Selection
    // **Validates: Requirements 3.10, 3.11, 3.12, 3.13, 6.2, 6.3, 6.4**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// Property 11 – Case 1: SameModel returns the provider whose model matches primary_model.
        #[test]
        fn prop_same_model_returns_matching_or_exhausted(
            providers in arb_provider_list(),
            attempted_indices in proptest::collection::hash_set(0usize..6, 0..=3),
        ) {
            let primary_model = providers[0].model.as_deref().unwrap();
            let primary_model_owned = primary_model.to_string();
            let attempted: HashSet<String> = attempted_indices
                .into_iter()
                .filter_map(|i| providers.get(i).and_then(|p| p.model.clone()))
                .collect();
            let ctx = stub_context();
            let selector = ProviderSelector;

            let result = selector.select(
                RetryStrategy::SameModel,
                &primary_model_owned,
                &[],
                &providers,
                &attempted,
                &RetryAfterStateManager::new(),
                &LatencyBlockStateManager::new(),
                &ctx,
                false,
                false,
            );

            match result {
                Ok(ProviderSelectionResult::Selected(p)) => {
                    // SameModel: selected provider's model must equal primary_model
                    prop_assert_eq!(
                        p.model.as_deref(),
                        Some(primary_model_owned.as_str()),
                        "SameModel selected a different model: {:?} vs {}",
                        p.model, primary_model_owned
                    );
                }
                Ok(ProviderSelectionResult::WaitAndRetrySameModel { .. }) => {
                    // Acceptable in P1/P2 when RA-blocked; not expected in P0 but valid.
                }
                Err(_) => {
                    // All matching candidates must have been attempted
                    let has_unattempted = providers.iter().any(|p| {
                        p.model.as_deref() == Some(primary_model_owned.as_str())
                            && !attempted.contains(&primary_model_owned)
                    });
                    prop_assert!(
                        !has_unattempted,
                        "SameModel returned Err but unattempted candidate exists"
                    );
                }
            }
        }

        /// Property 11 – Case 2: SameProvider returns a provider with the same prefix as primary_model.
        #[test]
        fn prop_same_provider_selects_matching_prefix(
            providers in arb_provider_list(),
            attempted_indices in proptest::collection::hash_set(0usize..6, 0..=3),
        ) {
            let primary_model = providers[0].model.as_deref().unwrap();
            let primary_model_owned = primary_model.to_string();
            let primary_prefix = extract_provider(&primary_model_owned).to_string();
            let attempted: HashSet<String> = attempted_indices
                .into_iter()
                .filter_map(|i| providers.get(i).and_then(|p| p.model.clone()))
                .collect();
            let ctx = stub_context();
            let selector = ProviderSelector;

            let result = selector.select(
                RetryStrategy::SameProvider,
                &primary_model_owned,
                &[],
                &providers,
                &attempted,
                &RetryAfterStateManager::new(),
                &LatencyBlockStateManager::new(),
                &ctx,
                false,
                false,
            );

            match result {
                Ok(ProviderSelectionResult::Selected(p)) => {
                    let selected_model = p.model.as_deref().unwrap();
                    let selected_prefix = extract_provider(selected_model);
                    prop_assert_eq!(
                        selected_prefix, primary_prefix.as_str(),
                        "SameProvider selected different prefix: {} vs {}",
                        selected_prefix, primary_prefix
                    );
                    prop_assert!(
                        !attempted.contains(selected_model),
                        "SameProvider selected an already-attempted provider: {}",
                        selected_model
                    );
                }
                Ok(ProviderSelectionResult::WaitAndRetrySameModel { .. }) => {
                    // Not expected for SameProvider in P0, but valid variant.
                }
                Err(_) => {
                    // All same-prefix candidates must have been attempted
                    let has_unattempted = providers.iter().any(|p| {
                        if let Some(ref m) = p.model {
                            extract_provider(m) == primary_prefix
                                && !attempted.contains(m.as_str())
                        } else {
                            false
                        }
                    });
                    prop_assert!(
                        !has_unattempted,
                        "SameProvider returned Err but unattempted same-prefix candidate exists"
                    );
                }
            }
        }

        /// Property 11 – Case 3: DifferentProvider returns a provider with a different prefix than primary_model.
        #[test]
        fn prop_different_provider_selects_different_prefix(
            providers in arb_provider_list(),
            attempted_indices in proptest::collection::hash_set(0usize..6, 0..=3),
        ) {
            let primary_model = providers[0].model.as_deref().unwrap();
            let primary_model_owned = primary_model.to_string();
            let primary_prefix = extract_provider(&primary_model_owned).to_string();
            let attempted: HashSet<String> = attempted_indices
                .into_iter()
                .filter_map(|i| providers.get(i).and_then(|p| p.model.clone()))
                .collect();
            let ctx = stub_context();
            let selector = ProviderSelector;

            let result = selector.select(
                RetryStrategy::DifferentProvider,
                &primary_model_owned,
                &[],
                &providers,
                &attempted,
                &RetryAfterStateManager::new(),
                &LatencyBlockStateManager::new(),
                &ctx,
                false,
                false,
            );

            match result {
                Ok(ProviderSelectionResult::Selected(p)) => {
                    let selected_model = p.model.as_deref().unwrap();
                    let selected_prefix = extract_provider(selected_model);
                    prop_assert_ne!(
                        selected_prefix, primary_prefix.as_str(),
                        "DifferentProvider selected same prefix: {} vs {}",
                        selected_prefix, primary_prefix
                    );
                    prop_assert!(
                        !attempted.contains(selected_model),
                        "DifferentProvider selected an already-attempted provider: {}",
                        selected_model
                    );
                }
                Ok(ProviderSelectionResult::WaitAndRetrySameModel { .. }) => {
                    // Not expected for DifferentProvider, but valid variant.
                }
                Err(_) => {
                    // All different-prefix candidates must have been attempted
                    let has_unattempted = providers.iter().any(|p| {
                        if let Some(ref m) = p.model {
                            extract_provider(m) != primary_prefix
                                && !attempted.contains(m.as_str())
                        } else {
                            false
                        }
                    });
                    prop_assert!(
                        !has_unattempted,
                        "DifferentProvider returned Err but unattempted different-prefix candidate exists"
                    );
                }
            }
        }
    }

    // Feature: retry-on-ratelimit, Property 10: Fallback Models Priority Ordering
    // **Validates: Requirements 3.10, 3.11, 3.12, 3.13, 6.2, 6.3, 6.4**
    //
    // For any provider selection where fallback_models is non-empty, the selector
    // must try models from fallback_models in their defined order before considering
    // models from the general Provider_List. A model should only be skipped if it
    // has already been attempted or is blocked.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_fallback_models_priority_ordering(
            all_providers in arb_provider_list(),
            fallback_indices in proptest::collection::vec(0usize..6, 0..=4),
            attempted_indices in proptest::collection::hash_set(0usize..6, 0..=3),
            strategy in prop_oneof![
                Just(RetryStrategy::SameProvider),
                Just(RetryStrategy::DifferentProvider),
            ],
        ) {
            // Use first provider as primary model.
            let primary_model = all_providers[0].model.as_deref().unwrap().to_string();
            let primary_prefix = extract_provider(&primary_model).to_string();

            // Build fallback_models from indices into all_providers (may reference
            // models not in all_providers if index is out of range — that's fine,
            // those get skipped).
            let fallback_models: Vec<String> = fallback_indices
                .iter()
                .filter_map(|&i| all_providers.get(i).and_then(|p| p.model.clone()))
                .collect();

            // Build attempted set from indices.
            let attempted: HashSet<String> = attempted_indices
                .iter()
                .filter_map(|&i| all_providers.get(i).and_then(|p| p.model.clone()))
                .collect();

            let ctx = stub_context();
            let selector = ProviderSelector;

            let result = selector.select(
                strategy,
                &primary_model,
                &fallback_models,
                &all_providers,
                &attempted,
                &RetryAfterStateManager::new(),
                &LatencyBlockStateManager::new(),
                &ctx,
                false,
                false,
            );

            // Determine which fallback models are eligible: present in
            // all_providers, match strategy, and not attempted.
            let matches_strategy = |model_id: &str| -> bool {
                let prefix = extract_provider(model_id);
                match strategy {
                    RetryStrategy::SameProvider => prefix == primary_prefix,
                    RetryStrategy::DifferentProvider => prefix != primary_prefix,
                    _ => unreachable!(),
                }
            };

            let first_eligible_fallback: Option<&str> = fallback_models.iter().find_map(|fm| {
                if attempted.contains(fm.as_str()) {
                    return None;
                }
                if !matches_strategy(fm) {
                    return None;
                }
                // Must exist in all_providers.
                if all_providers.iter().any(|p| p.model.as_deref() == Some(fm.as_str())) {
                    Some(fm.as_str())
                } else {
                    None
                }
            });

            // First eligible Provider_List candidate (not in fallback, or any
            // eligible candidate from Provider_List order).
            let first_eligible_provider_list: Option<&str> = all_providers.iter().find_map(|p| {
                if let Some(ref m) = p.model {
                    if matches_strategy(m) && !attempted.contains(m.as_str()) {
                        Some(m.as_str())
                    } else {
                        None
                    }
                } else {
                    None
                }
            });

            match result {
                Ok(ProviderSelectionResult::Selected(p)) => {
                    let selected = p.model.as_deref().unwrap();

                    if let Some(expected_fallback) = first_eligible_fallback {
                        // If there's an eligible fallback, it MUST be selected
                        // (priority over Provider_List).
                        prop_assert_eq!(
                            selected, expected_fallback,
                            "Expected first eligible fallback '{}' but got '{}'. \
                             fallback_models={:?}, attempted={:?}, strategy={:?}",
                            expected_fallback, selected, fallback_models, attempted, strategy
                        );
                    } else {
                        // No eligible fallback → must come from Provider_List.
                        // The selected model must match strategy and not be attempted.
                        prop_assert!(
                            matches_strategy(selected),
                            "Selected '{}' doesn't match strategy {:?}",
                            selected, strategy
                        );
                        prop_assert!(
                            !attempted.contains(selected),
                            "Selected '{}' was already attempted",
                            selected
                        );
                        // Should be the first eligible from Provider_List order.
                        if let Some(expected_pl) = first_eligible_provider_list {
                            prop_assert_eq!(
                                selected, expected_pl,
                                "Expected first Provider_List candidate '{}' but got '{}'",
                                expected_pl, selected
                            );
                        }
                    }
                }
                Ok(ProviderSelectionResult::WaitAndRetrySameModel { .. }) => {
                    // Not expected for SameProvider/DifferentProvider, but valid variant.
                }
                Err(_) => {
                    // No eligible candidate at all — verify that's correct.
                    prop_assert!(
                        first_eligible_fallback.is_none(),
                        "Returned Err but eligible fallback exists: {:?}",
                        first_eligible_fallback
                    );
                    prop_assert!(
                        first_eligible_provider_list.is_none(),
                        "Returned Err but eligible Provider_List candidate exists: {:?}",
                        first_eligible_provider_list
                    );
                }
            }
        }
    }

    // Feature: retry-on-ratelimit, Property 7: Cooldown Exclusion Invariant (CP-1)
    // **Validates: Requirements 6.5, 11.5, 11.6, 12.6, 12.7, 13.1, 13.3, 13.4, 13.9, CP-1**
    //
    // For any model/provider with an active Retry_After_State entry (expires_at > now),
    // that model/provider must NOT be selected by ProviderSelector. For same_model strategy,
    // WaitAndRetrySameModel is returned instead. Once expired, the model must be eligible again.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// Property 7 – Case 1: Blocked models are never returned as Selected
        /// for SameProvider / DifferentProvider strategies.
        #[test]
        fn prop_cooldown_exclusion_blocked_never_selected(
            all_providers in proptest::collection::vec(arb_model_id(), 2..=6)
                .prop_map(|ids| {
                    ids.into_iter()
                        .map(|id| make_provider_with_retry_policy(&id, None))
                        .collect::<Vec<_>>()
                }),
            // Indices of providers to block via RA state
            block_indices in proptest::collection::hash_set(0usize..6, 1..=3),
            strategy in prop_oneof![
                Just(RetryStrategy::SameProvider),
                Just(RetryStrategy::DifferentProvider),
            ],
        ) {
            let primary_model = all_providers[0].model.as_deref().unwrap().to_string();
            let attempted = HashSet::new();
            let ctx = stub_context();
            let selector = ProviderSelector;
            let ra_state = RetryAfterStateManager::new();

            // Block selected providers with a long RA duration
            let blocked_models: HashSet<String> = block_indices
                .iter()
                .filter_map(|&i| all_providers.get(i).and_then(|p| p.model.clone()))
                .collect();

            for model_id in &blocked_models {
                ra_state.record(model_id, 600, 600);
            }

            let result = selector.select(
                strategy,
                &primary_model,
                &[],
                &all_providers,
                &attempted,
                &ra_state,
                &LatencyBlockStateManager::new(),
                &ctx,
                true,  // has_retry_policy = true to enable RA checks
                false,
            );

            match result {
                Ok(ProviderSelectionResult::Selected(p)) => {
                    let selected = p.model.as_deref().unwrap();
                    prop_assert!(
                        !blocked_models.contains(selected),
                        "Blocked model '{}' was returned as Selected! blocked={:?}",
                        selected, blocked_models
                    );
                }
                Ok(ProviderSelectionResult::WaitAndRetrySameModel { .. }) => {
                    // Not expected for SameProvider/DifferentProvider, but acceptable.
                }
                Err(_) => {
                    // All eligible candidates were blocked or exhausted — valid.
                }
            }
        }

        /// Property 7 – Case 2: For same_model strategy with RA block,
        /// WaitAndRetrySameModel is returned (not Selected).
        #[test]
        fn prop_cooldown_exclusion_same_model_returns_wait(
            model_id in arb_model_id(),
            block_seconds in 1u64..=300,
        ) {
            let providers = vec![
                make_provider_with_retry_policy(&model_id, None),
            ];
            let attempted = HashSet::new();
            let ctx = stub_context();
            let selector = ProviderSelector;
            let ra_state = RetryAfterStateManager::new();

            // Block the model
            ra_state.record(&model_id, block_seconds, 300);

            let result = selector.select(
                RetryStrategy::SameModel,
                &model_id,
                &[],
                &providers,
                &attempted,
                &ra_state,
                &LatencyBlockStateManager::new(),
                &ctx,
                true,
                false,
            );

            match result {
                Ok(ProviderSelectionResult::WaitAndRetrySameModel { wait_duration }) => {
                    // Duration must be positive and bounded by block_seconds
                    let capped = block_seconds.min(300);
                    prop_assert!(
                        wait_duration.as_secs() <= capped,
                        "wait_duration {}s exceeds capped block {}s",
                        wait_duration.as_secs(), capped
                    );
                    prop_assert!(
                        !wait_duration.is_zero(),
                        "wait_duration should be positive for an active block"
                    );
                }
                Ok(ProviderSelectionResult::Selected(_)) => {
                    prop_assert!(false, "Blocked model should not be Selected for same_model strategy");
                }
                Err(_) => {
                    prop_assert!(false, "same_model with blocked model should return WaitAndRetrySameModel, not Err");
                }
            }
        }

        /// Property 7 – Case 3: Blocked models in fallback_models are skipped.
        #[test]
        fn prop_cooldown_exclusion_fallback_blocked_skipped(
            all_providers in proptest::collection::vec(arb_model_id(), 3..=6)
                .prop_map(|ids| {
                    ids.into_iter()
                        .map(|id| make_provider_with_retry_policy(&id, None))
                        .collect::<Vec<_>>()
                }),
            // Block the first 1-2 fallback candidates
            num_blocked in 1usize..=2,
            strategy in prop_oneof![
                Just(RetryStrategy::SameProvider),
                Just(RetryStrategy::DifferentProvider),
            ],
        ) {
            let primary_model = all_providers[0].model.as_deref().unwrap().to_string();
            let attempted = HashSet::new();
            let ctx = stub_context();
            let selector = ProviderSelector;
            let ra_state = RetryAfterStateManager::new();

            // Build fallback_models from providers (skip primary)
            let fallback_models: Vec<String> = all_providers[1..]
                .iter()
                .filter_map(|p| p.model.clone())
                .collect();

            // Block the first num_blocked fallback models
            let blocked_models: HashSet<String> = fallback_models
                .iter()
                .take(num_blocked.min(fallback_models.len()))
                .cloned()
                .collect();

            for model_id in &blocked_models {
                ra_state.record(model_id, 600, 600);
            }

            let result = selector.select(
                strategy,
                &primary_model,
                &fallback_models,
                &all_providers,
                &attempted,
                &ra_state,
                &LatencyBlockStateManager::new(),
                &ctx,
                true,
                false,
            );

            match result {
                Ok(ProviderSelectionResult::Selected(p)) => {
                    let selected = p.model.as_deref().unwrap();
                    prop_assert!(
                        !blocked_models.contains(selected),
                        "Blocked fallback model '{}' was selected! blocked={:?}",
                        selected, blocked_models
                    );
                }
                Ok(ProviderSelectionResult::WaitAndRetrySameModel { .. }) => {
                    // Not expected for these strategies, but acceptable.
                }
                Err(_) => {
                    // All eligible candidates blocked or exhausted — valid.
                }
            }
        }

        /// Property 7 – Case 4: After RA expiration, model becomes selectable again.
        /// We use a 0-second block which expires immediately.
        #[test]
        fn prop_cooldown_exclusion_unblocked_after_expiration(
            model_id in arb_model_id(),
            strategy in prop_oneof![
                Just(RetryStrategy::SameModel),
                Just(RetryStrategy::SameProvider),
                Just(RetryStrategy::DifferentProvider),
            ],
        ) {
            let providers = vec![
                make_provider_with_retry_policy(&model_id, None),
            ];
            let attempted = HashSet::new();
            let ctx = stub_context();
            let selector = ProviderSelector;
            let ra_state = RetryAfterStateManager::new();

            // Record with 0 seconds — expires immediately
            ra_state.record(&model_id, 0, 300);

            // The model should NOT be blocked (expired immediately)
            prop_assert!(
                !ra_state.is_blocked(&model_id),
                "Model should not be blocked after 0-second RA record"
            );

            let result = selector.select(
                strategy,
                &model_id,
                &[],
                &providers,
                &attempted,
                &ra_state,
                &LatencyBlockStateManager::new(),
                &ctx,
                true,
                false,
            );

            // For any strategy, the model should be selectable (not blocked)
            match result {
                Ok(ProviderSelectionResult::Selected(p)) => {
                    prop_assert_eq!(
                        p.model.as_deref(),
                        Some(model_id.as_str()),
                        "Expected the unblocked model to be selected"
                    );
                }
                Ok(ProviderSelectionResult::WaitAndRetrySameModel { .. }) => {
                    prop_assert!(false, "Expired RA should not trigger WaitAndRetrySameModel");
                }
                Err(_) => {
                    // For DifferentProvider strategy, the single provider may not match
                    // (same prefix as primary). This is a strategy mismatch, not a block issue.
                    // Only fail if strategy should have matched.
                    match strategy {
                        RetryStrategy::SameModel | RetryStrategy::SameProvider => {
                            prop_assert!(false, "Unblocked model should be selectable for {:?}", strategy);
                        }
                        RetryStrategy::DifferentProvider => {
                            // Expected: single provider can't match "different provider" strategy.
                        }
                    }
                }
            }
        }
    }

    // Feature: retry-on-ratelimit, Property 19: Latency Block Exclusion During Provider Selection
    // **Validates: Requirements 6.7, 6.8, 15.1, 15.3, 15.4, 15.12, 15.13**
    //
    // For any model/provider with an active Latency_Block_State entry (expires_at > now),
    // that model/provider must be skipped during provider selection (both initial and retry).
    // When both Retry_After_State and Latency_Block_State exist for the same identifier,
    // the candidate must be skipped if either state indicates blocking.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// Property 19 – Case 1: LB-blocked models are never returned as Selected
        /// for SameProvider / DifferentProvider strategies.
        #[test]
        fn prop_lb_blocked_never_selected(
            all_providers in proptest::collection::vec(arb_model_id(), 2..=6)
                .prop_map(|ids| {
                    ids.into_iter()
                        .map(|id| make_provider_with_hl_config(
                            &id,
                            None,
                            Some(make_hl_config(BlockScope::Model, ApplyTo::Global)),
                        ))
                        .collect::<Vec<_>>()
                }),
            block_indices in proptest::collection::hash_set(0usize..6, 1..=3),
            strategy in prop_oneof![
                Just(RetryStrategy::SameProvider),
                Just(RetryStrategy::DifferentProvider),
            ],
        ) {
            let primary_model = all_providers[0].model.as_deref().unwrap().to_string();
            let attempted = HashSet::new();
            let ctx = stub_context();
            let selector = ProviderSelector;
            let lb_state = LatencyBlockStateManager::new();

            // Block selected providers with a long LB duration
            let blocked_models: HashSet<String> = block_indices
                .iter()
                .filter_map(|&i| all_providers.get(i).and_then(|p| p.model.clone()))
                .collect();

            for model_id in &blocked_models {
                lb_state.record_block(model_id, 600, 8000);
            }

            let result = selector.select(
                strategy,
                &primary_model,
                &[],
                &all_providers,
                &attempted,
                &RetryAfterStateManager::new(),
                &lb_state,
                &ctx,
                false,
                true, // has_high_latency_config = true to enable LB checks
            );

            match result {
                Ok(ProviderSelectionResult::Selected(p)) => {
                    let selected = p.model.as_deref().unwrap();
                    prop_assert!(
                        !blocked_models.contains(selected),
                        "LB-blocked model '{}' was returned as Selected! blocked={:?}",
                        selected, blocked_models
                    );
                }
                Ok(ProviderSelectionResult::WaitAndRetrySameModel { .. }) => {
                    // Not expected for SameProvider/DifferentProvider, but acceptable.
                }
                Err(_) => {
                    // All eligible candidates were blocked or exhausted — valid.
                }
            }
        }

        /// Property 19 – Case 2: For same_model strategy with LB block,
        /// AllProvidersExhaustedError is returned (skip to alternative, not wait).
        #[test]
        fn prop_lb_blocked_same_model_returns_error(
            model_id in arb_model_id(),
            block_seconds in 1u64..=300,
        ) {
            let providers = vec![
                make_provider_with_hl_config(
                    &model_id,
                    None,
                    Some(make_hl_config(BlockScope::Model, ApplyTo::Global)),
                ),
            ];
            let attempted = HashSet::new();
            let ctx = stub_context();
            let selector = ProviderSelector;
            let lb_state = LatencyBlockStateManager::new();

            // Block the model
            lb_state.record_block(&model_id, block_seconds, 8000);

            let result = selector.select(
                RetryStrategy::SameModel,
                &model_id,
                &[],
                &providers,
                &attempted,
                &RetryAfterStateManager::new(),
                &lb_state,
                &ctx,
                false,
                true,
            );

            match result {
                Err(_) => {
                    // Expected: same_model with LB block returns error (skip to alternative)
                }
                Ok(ProviderSelectionResult::Selected(_)) => {
                    prop_assert!(false, "LB-blocked model should not be Selected for same_model strategy");
                }
                Ok(ProviderSelectionResult::WaitAndRetrySameModel { .. }) => {
                    prop_assert!(false, "LB block should return error, not WaitAndRetrySameModel (unlike RA)");
                }
            }
        }

        /// Property 19 – Case 3: When both RA and LB exist for the same identifier,
        /// the candidate is skipped if either blocks.
        #[test]
        fn prop_both_ra_and_lb_either_blocks_skips(
            all_providers in proptest::collection::vec(arb_model_id(), 2..=6)
                .prop_map(|ids| {
                    ids.into_iter()
                        .map(|id| make_provider_with_hl_config(
                            &id,
                            None,
                            Some(make_hl_config(BlockScope::Model, ApplyTo::Global)),
                        ))
                        .collect::<Vec<_>>()
                }),
            block_index in 0usize..6,
            // Which state(s) to block: 0 = RA only, 1 = LB only, 2 = both
            block_type in 0u8..3,
            strategy in prop_oneof![
                Just(RetryStrategy::SameProvider),
                Just(RetryStrategy::DifferentProvider),
            ],
        ) {
            let primary_model = all_providers[0].model.as_deref().unwrap().to_string();
            let attempted = HashSet::new();
            let ctx = stub_context();
            let selector = ProviderSelector;
            let ra_state = RetryAfterStateManager::new();
            let lb_state = LatencyBlockStateManager::new();

            // Pick a model to block (clamped to valid index)
            let target_index = block_index % all_providers.len();
            let target_model = all_providers[target_index].model.as_deref().unwrap().to_string();

            match block_type {
                0 => {
                    // RA only
                    ra_state.record(&target_model, 600, 600);
                }
                1 => {
                    // LB only
                    lb_state.record_block(&target_model, 600, 8000);
                }
                _ => {
                    // Both RA and LB
                    ra_state.record(&target_model, 600, 600);
                    lb_state.record_block(&target_model, 600, 8000);
                }
            }

            let result = selector.select(
                strategy,
                &primary_model,
                &[],
                &all_providers,
                &attempted,
                &ra_state,
                &lb_state,
                &ctx,
                true,  // has_retry_policy = true to enable RA checks
                true,  // has_high_latency_config = true to enable LB checks
            );

            match result {
                Ok(ProviderSelectionResult::Selected(p)) => {
                    let selected = p.model.as_deref().unwrap();
                    prop_assert!(
                        selected != target_model,
                        "Blocked model '{}' was selected despite block_type={}! \
                         (0=RA, 1=LB, 2=both)",
                        target_model, block_type
                    );
                }
                Ok(ProviderSelectionResult::WaitAndRetrySameModel { .. }) => {
                    // Not expected for SameProvider/DifferentProvider.
                }
                Err(_) => {
                    // All eligible candidates blocked or exhausted — valid.
                }
            }
        }

        /// Property 19 – Case 4: After LB expiration, model becomes selectable again.
        /// We use a 0-second block which expires immediately.
        #[test]
        fn prop_lb_unblocked_after_expiration(
            model_id in arb_model_id(),
            strategy in prop_oneof![
                Just(RetryStrategy::SameModel),
                Just(RetryStrategy::SameProvider),
                Just(RetryStrategy::DifferentProvider),
            ],
        ) {
            let providers = vec![
                make_provider_with_hl_config(
                    &model_id,
                    None,
                    Some(make_hl_config(BlockScope::Model, ApplyTo::Global)),
                ),
            ];
            let attempted = HashSet::new();
            let ctx = stub_context();
            let selector = ProviderSelector;
            let lb_state = LatencyBlockStateManager::new();

            // Record with 0 seconds — expires immediately
            lb_state.record_block(&model_id, 0, 8000);

            // The model should NOT be blocked (expired immediately)
            prop_assert!(
                !lb_state.is_blocked(&model_id),
                "Model should not be blocked after 0-second LB record"
            );

            let result = selector.select(
                strategy,
                &model_id,
                &[],
                &providers,
                &attempted,
                &RetryAfterStateManager::new(),
                &lb_state,
                &ctx,
                false,
                true,
            );

            match result {
                Ok(ProviderSelectionResult::Selected(p)) => {
                    prop_assert_eq!(
                        p.model.as_deref(),
                        Some(model_id.as_str()),
                        "Expected the unblocked model to be selected"
                    );
                }
                Ok(ProviderSelectionResult::WaitAndRetrySameModel { .. }) => {
                    prop_assert!(false, "Expired LB should not trigger WaitAndRetrySameModel");
                }
                Err(_) => {
                    match strategy {
                        RetryStrategy::SameModel | RetryStrategy::SameProvider => {
                            prop_assert!(false, "Unblocked model should be selectable for {:?}", strategy);
                        }
                        RetryStrategy::DifferentProvider => {
                            // Expected: single provider can't match "different provider" strategy.
                        }
                    }
                }
            }
        }
    }

    // Feature: retry-on-ratelimit, Property 9: Cooldown Applies to Initial Provider Selection (CP-3)
    // **Validates: Requirements 13.1, 13.12, CP-3**
    //
    // For any new request (not a retry) targeting a model that has an active
    // Retry_After_State entry with apply_to: "global", the ProviderSelector must
    // skip that model during initial provider selection and route to an alternative
    // model, without first attempting the blocked model.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// Property 9 – Case 1: Default model is globally RA-blocked →
        /// new request with same_model strategy gets WaitAndRetrySameModel.
        #[test]
        fn prop_initial_selection_cooldown_same_model(
            model_id in arb_model_id(),
            block_seconds in 1u64..=300,
        ) {
            let providers = vec![
                make_provider_with_retry_policy(&model_id, Some(RetryAfterHandlingConfig {
                    scope: BlockScope::Model,
                    apply_to: ApplyTo::Global,
                    max_retry_after_seconds: 300,
                })),
            ];
            // Empty attempted set = brand new request (initial selection)
            let attempted = HashSet::new();
            let ctx = stub_context();
            let selector = ProviderSelector;
            let ra_state = RetryAfterStateManager::new();

            // Block the default model globally
            ra_state.record(&model_id, block_seconds, 300);

            let result = selector.select(
                RetryStrategy::SameModel,
                &model_id,
                &[],
                &providers,
                &attempted,
                &ra_state,
                &LatencyBlockStateManager::new(),
                &ctx,
                true,
                false,
            );

            // For same_model with global RA block, must return WaitAndRetrySameModel
            match result {
                Ok(ProviderSelectionResult::WaitAndRetrySameModel { wait_duration }) => {
                    let capped = block_seconds.min(300);
                    prop_assert!(
                        !wait_duration.is_zero(),
                        "wait_duration should be positive for an active block"
                    );
                    prop_assert!(
                        wait_duration.as_secs() <= capped,
                        "wait_duration {}s exceeds capped block {}s",
                        wait_duration.as_secs(), capped
                    );
                }
                Ok(ProviderSelectionResult::Selected(_)) => {
                    prop_assert!(false,
                        "Globally RA-blocked model should NOT be Selected on initial request \
                         with same_model strategy; expected WaitAndRetrySameModel"
                    );
                }
                Err(_) => {
                    prop_assert!(false,
                        "same_model with globally blocked model should return \
                         WaitAndRetrySameModel, not AllProvidersExhausted"
                    );
                }
            }
        }

        /// Property 9 – Case 2: Default model is globally RA-blocked →
        /// new request with different_provider strategy skips it and picks alternative.
        #[test]
        fn prop_initial_selection_cooldown_different_provider(
            _primary_prefix in arb_prefix(),
            alt_prefix in arb_prefix().prop_filter("must differ from primary",
                |p| p != "openai"), // we'll force primary to "openai"
            block_seconds in 1u64..=300,
        ) {
            let primary_model = format!("openai/model-a");
            let alt_model = format!("{}/model-b", alt_prefix);

            // Ensure alt is actually a different provider
            if extract_provider(&alt_model) == extract_provider(&primary_model) {
                // Skip this case — proptest will generate others
                return Ok(());
            }

            let providers = vec![
                make_provider_with_retry_policy(&primary_model, Some(RetryAfterHandlingConfig {
                    scope: BlockScope::Model,
                    apply_to: ApplyTo::Global,
                    max_retry_after_seconds: 300,
                })),
                make_provider_with_retry_policy(&alt_model, None),
            ];
            // Empty attempted set = brand new request
            let attempted = HashSet::new();
            let ctx = stub_context();
            let selector = ProviderSelector;
            let ra_state = RetryAfterStateManager::new();

            // Block the primary/default model globally
            ra_state.record(&primary_model, block_seconds, 300);

            let result = selector.select(
                RetryStrategy::DifferentProvider,
                &primary_model,
                &[],
                &providers,
                &attempted,
                &ra_state,
                &LatencyBlockStateManager::new(),
                &ctx,
                true,
                false,
            );

            match result {
                Ok(ProviderSelectionResult::Selected(p)) => {
                    let selected = p.model.as_deref().unwrap();
                    // Must NOT be the blocked primary model
                    prop_assert_ne!(
                        selected, primary_model.as_str(),
                        "Blocked primary model was selected on initial request!"
                    );
                    // Must be from a different provider (strategy constraint)
                    prop_assert_ne!(
                        extract_provider(selected),
                        extract_provider(&primary_model),
                        "DifferentProvider selected same provider prefix"
                    );
                }
                Ok(ProviderSelectionResult::WaitAndRetrySameModel { .. }) => {
                    prop_assert!(false,
                        "DifferentProvider strategy should not return WaitAndRetrySameModel"
                    );
                }
                Err(_) => {
                    // Only valid if the alt model also happens to be same provider
                    // (filtered out above) — should not happen.
                    prop_assert!(false,
                        "Should have selected alternative provider, not exhausted"
                    );
                }
            }
        }

        /// Property 9 – Case 3: Default model is globally RA-blocked →
        /// new request with same_provider strategy skips it and picks same-provider alternative.
        #[test]
        fn prop_initial_selection_cooldown_same_provider(
            prefix in arb_prefix(),
            block_seconds in 1u64..=300,
        ) {
            let primary_model = format!("{}/model-a", prefix);
            let alt_model = format!("{}/model-b", prefix);

            let providers = vec![
                make_provider_with_retry_policy(&primary_model, Some(RetryAfterHandlingConfig {
                    scope: BlockScope::Model,
                    apply_to: ApplyTo::Global,
                    max_retry_after_seconds: 300,
                })),
                make_provider_with_retry_policy(&alt_model, None),
            ];
            // Empty attempted set = brand new request
            let attempted = HashSet::new();
            let ctx = stub_context();
            let selector = ProviderSelector;
            let ra_state = RetryAfterStateManager::new();

            // Block the primary/default model globally (model-scope, not provider-scope)
            ra_state.record(&primary_model, block_seconds, 300);

            let result = selector.select(
                RetryStrategy::SameProvider,
                &primary_model,
                &[],
                &providers,
                &attempted,
                &ra_state,
                &LatencyBlockStateManager::new(),
                &ctx,
                true,
                false,
            );

            match result {
                Ok(ProviderSelectionResult::Selected(p)) => {
                    let selected = p.model.as_deref().unwrap();
                    // Must NOT be the blocked primary model
                    prop_assert_ne!(
                        selected, primary_model.as_str(),
                        "Blocked primary model was selected on initial request!"
                    );
                    // Must be from the same provider (strategy constraint)
                    prop_assert_eq!(
                        extract_provider(selected),
                        extract_provider(&primary_model),
                        "SameProvider selected different provider prefix"
                    );
                    // Should be the alternative model
                    prop_assert_eq!(
                        selected, alt_model.as_str(),
                        "Expected the alternative same-provider model"
                    );
                }
                Ok(ProviderSelectionResult::WaitAndRetrySameModel { .. }) => {
                    prop_assert!(false,
                        "SameProvider strategy should not return WaitAndRetrySameModel"
                    );
                }
                Err(_) => {
                    prop_assert!(false,
                        "Should have selected same-provider alternative, not exhausted"
                    );
                }
            }
        }
    }
}

