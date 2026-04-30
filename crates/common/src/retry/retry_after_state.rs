use std::time::{Duration, Instant};

use dashmap::DashMap;
use log::info;

use crate::configuration::{extract_provider, BlockScope};

/// Thread-safe global state manager for Retry-After header blocking.
///
/// This manager handles ONLY global state (`apply_to: "global"`).
/// Request-scoped state (`apply_to: "request"`) is stored in
/// `RequestContext.request_retry_after_state` and managed by the orchestrator.
///
/// Entries use max-expiration semantics: if a new Retry-After value is recorded
/// for an identifier that already has an entry, the expiration is updated only
/// if the new expiration is later than the existing one.
pub struct RetryAfterStateManager {
    /// Global state: identifier (model ID or provider prefix) -> expiration timestamp
    global_state: DashMap<String, Instant>,
}

impl RetryAfterStateManager {
    pub fn new() -> Self {
        Self {
            global_state: DashMap::new(),
        }
    }

    /// Record a Retry-After header, creating or updating the block entry.
    ///
    /// The `retry_after_seconds` value is capped at `max_retry_after_seconds`.
    /// Uses max-expiration semantics: if an entry already exists, the expiration
    /// is updated only if the new expiration is later.
    pub fn record(&self, identifier: &str, retry_after_seconds: u64, max_retry_after_seconds: u64) {
        let capped = retry_after_seconds.min(max_retry_after_seconds);
        let new_expiration = Instant::now() + Duration::from_secs(capped);

        self.global_state
            .entry(identifier.to_string())
            .and_modify(|existing| {
                if new_expiration > *existing {
                    *existing = new_expiration;
                }
            })
            .or_insert(new_expiration);
    }

    /// Check if an identifier is currently blocked.
    ///
    /// Lazily cleans up expired entries.
    pub fn is_blocked(&self, identifier: &str) -> bool {
        if let Some(entry) = self.global_state.get(identifier) {
            if Instant::now() < *entry {
                return true;
            }
            // Entry expired — drop the read guard before removing
            drop(entry);
            self.global_state.remove(identifier);
            info!("Retry_After_State expired: identifier={}", identifier);
        }
        false
    }

    /// Get remaining block duration for an identifier, if blocked.
    ///
    /// Returns `None` if the identifier is not blocked or the entry has expired.
    /// Lazily cleans up expired entries.
    pub fn remaining_block_duration(&self, identifier: &str) -> Option<Duration> {
        if let Some(entry) = self.global_state.get(identifier) {
            let now = Instant::now();
            if now < *entry {
                return Some(*entry - now);
            }
            // Entry expired — drop the read guard before removing
            drop(entry);
            self.global_state.remove(identifier);
            info!("Retry_After_State expired: identifier={}", identifier);
        }
        None
    }

    /// Check if a model is blocked, considering scope (model or provider).
    ///
    /// - `BlockScope::Model`: checks if the exact `model_id` is blocked.
    /// - `BlockScope::Provider`: extracts the provider prefix from `model_id`
    ///   and checks if that prefix is blocked.
    pub fn is_model_blocked(&self, model_id: &str, scope: BlockScope) -> bool {
        match scope {
            BlockScope::Model => self.is_blocked(model_id),
            BlockScope::Provider => {
                let provider = extract_provider(model_id);
                self.is_blocked(provider)
            }
        }
    }
}

impl Default for RetryAfterStateManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_new_manager_has_no_blocks() {
        let mgr = RetryAfterStateManager::new();
        assert!(!mgr.is_blocked("openai/gpt-4o"));
        assert!(mgr.remaining_block_duration("openai/gpt-4o").is_none());
    }

    #[test]
    fn test_record_and_is_blocked() {
        let mgr = RetryAfterStateManager::new();
        mgr.record("openai/gpt-4o", 60, 300);
        assert!(mgr.is_blocked("openai/gpt-4o"));
        assert!(!mgr.is_blocked("anthropic/claude"));
    }

    #[test]
    fn test_record_caps_at_max() {
        let mgr = RetryAfterStateManager::new();
        // Retry-After of 600 seconds, but max is 300
        mgr.record("openai/gpt-4o", 600, 300);
        let remaining = mgr.remaining_block_duration("openai/gpt-4o").unwrap();
        // Should be capped at ~300 seconds (allow some tolerance)
        assert!(remaining <= Duration::from_secs(301));
        assert!(remaining > Duration::from_secs(298));
    }

    #[test]
    fn test_remaining_block_duration() {
        let mgr = RetryAfterStateManager::new();
        mgr.record("openai/gpt-4o", 10, 300);
        let remaining = mgr.remaining_block_duration("openai/gpt-4o").unwrap();
        assert!(remaining <= Duration::from_secs(11));
        assert!(remaining > Duration::from_secs(8));
    }

    #[test]
    fn test_expired_entry_cleaned_up_on_is_blocked() {
        let mgr = RetryAfterStateManager::new();
        // Record with 0 seconds — effectively expires immediately
        mgr.record("openai/gpt-4o", 0, 300);
        // Sleep briefly to ensure expiration
        thread::sleep(Duration::from_millis(10));
        assert!(!mgr.is_blocked("openai/gpt-4o"));
    }

    #[test]
    fn test_expired_entry_cleaned_up_on_remaining() {
        let mgr = RetryAfterStateManager::new();
        mgr.record("openai/gpt-4o", 0, 300);
        thread::sleep(Duration::from_millis(10));
        assert!(mgr.remaining_block_duration("openai/gpt-4o").is_none());
    }

    #[test]
    fn test_max_expiration_semantics_longer_wins() {
        let mgr = RetryAfterStateManager::new();
        mgr.record("openai/gpt-4o", 10, 300);
        let first_remaining = mgr.remaining_block_duration("openai/gpt-4o").unwrap();

        // Record a longer duration — should update
        mgr.record("openai/gpt-4o", 60, 300);
        let second_remaining = mgr.remaining_block_duration("openai/gpt-4o").unwrap();
        assert!(second_remaining > first_remaining);
    }

    #[test]
    fn test_max_expiration_semantics_shorter_does_not_overwrite() {
        let mgr = RetryAfterStateManager::new();
        mgr.record("openai/gpt-4o", 60, 300);
        let first_remaining = mgr.remaining_block_duration("openai/gpt-4o").unwrap();

        // Record a shorter duration — should NOT overwrite
        mgr.record("openai/gpt-4o", 5, 300);
        let second_remaining = mgr.remaining_block_duration("openai/gpt-4o").unwrap();
        // The remaining should still be close to the original 60s
        assert!(second_remaining > Duration::from_secs(50));
        // Allow small timing variance
        let diff = if first_remaining > second_remaining {
            first_remaining - second_remaining
        } else {
            second_remaining - first_remaining
        };
        assert!(diff < Duration::from_secs(2));
    }

    #[test]
    fn test_is_model_blocked_model_scope() {
        let mgr = RetryAfterStateManager::new();
        mgr.record("openai/gpt-4o", 60, 300);

        assert!(mgr.is_model_blocked("openai/gpt-4o", BlockScope::Model));
        assert!(!mgr.is_model_blocked("openai/gpt-4o-mini", BlockScope::Model));
    }

    #[test]
    fn test_is_model_blocked_provider_scope() {
        let mgr = RetryAfterStateManager::new();
        // Block at provider level by recording with provider prefix
        mgr.record("openai", 60, 300);

        // Both openai models should be blocked
        assert!(mgr.is_model_blocked("openai/gpt-4o", BlockScope::Provider));
        assert!(mgr.is_model_blocked("openai/gpt-4o-mini", BlockScope::Provider));
        // Anthropic should not be blocked
        assert!(!mgr.is_model_blocked("anthropic/claude", BlockScope::Provider));
    }

    #[test]
    fn test_model_scope_does_not_block_other_models() {
        let mgr = RetryAfterStateManager::new();
        mgr.record("openai/gpt-4o", 60, 300);

        // Model scope: only exact match is blocked
        assert!(mgr.is_model_blocked("openai/gpt-4o", BlockScope::Model));
        assert!(!mgr.is_model_blocked("openai/gpt-4o-mini", BlockScope::Model));
    }

    #[test]
    fn test_multiple_identifiers_independent() {
        let mgr = RetryAfterStateManager::new();
        mgr.record("openai/gpt-4o", 60, 300);
        mgr.record("anthropic/claude", 30, 300);

        assert!(mgr.is_blocked("openai/gpt-4o"));
        assert!(mgr.is_blocked("anthropic/claude"));
        assert!(!mgr.is_blocked("azure/gpt-4o"));
    }

    #[test]
    fn test_record_with_zero_seconds() {
        let mgr = RetryAfterStateManager::new();
        mgr.record("openai/gpt-4o", 0, 300);
        // With 0 seconds, the entry expires at Instant::now() + 0,
        // which is effectively immediately
        thread::sleep(Duration::from_millis(5));
        assert!(!mgr.is_blocked("openai/gpt-4o"));
    }

    #[test]
    fn test_max_retry_after_seconds_zero_caps_to_zero() {
        let mgr = RetryAfterStateManager::new();
        // Even with retry_after_seconds=60, max=0 caps to 0
        mgr.record("openai/gpt-4o", 60, 0);
        thread::sleep(Duration::from_millis(5));
        assert!(!mgr.is_blocked("openai/gpt-4o"));
    }

    #[test]
    fn test_default_trait() {
        let mgr = RetryAfterStateManager::default();
        assert!(!mgr.is_blocked("anything"));
    }

    // --- Proptest strategies ---

    use proptest::prelude::*;

    fn arb_provider_prefix() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("openai".to_string()),
            Just("anthropic".to_string()),
            Just("azure".to_string()),
            Just("google".to_string()),
            Just("cohere".to_string()),
        ]
    }

    fn arb_model_suffix() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("gpt-4o".to_string()),
            Just("gpt-4o-mini".to_string()),
            Just("claude-3".to_string()),
            Just("gemini-pro".to_string()),
        ]
    }

    fn arb_model_id() -> impl Strategy<Value = String> {
        (arb_provider_prefix(), arb_model_suffix())
            .prop_map(|(prefix, suffix)| format!("{}/{}", prefix, suffix))
    }

    fn arb_scope() -> impl Strategy<Value = BlockScope> {
        prop_oneof![Just(BlockScope::Model), Just(BlockScope::Provider),]
    }

    // Feature: retry-on-ratelimit, Property 15: Retry_After_State Scope Behavior
    // **Validates: Requirements 11.5, 11.6, 11.7, 11.8, 12.9, 12.10, 13.10, 13.11**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// Property 15 – Case 1: Model scope blocks only the exact model_id.
        #[test]
        fn prop_model_scope_blocks_exact_model_only(
            model_id in arb_model_id(),
            other_model_id in arb_model_id(),
            retry_after in 1u64..300,
        ) {
            prop_assume!(model_id != other_model_id);

            let mgr = RetryAfterStateManager::new();
            // Record with the exact model_id (model scope records the full model ID)
            mgr.record(&model_id, retry_after, 300);

            // The exact model should be blocked
            prop_assert!(
                mgr.is_model_blocked(&model_id, BlockScope::Model),
                "Model {} should be blocked with Model scope after recording",
                model_id
            );

            // A different model should NOT be blocked (even if same provider)
            prop_assert!(
                !mgr.is_model_blocked(&other_model_id, BlockScope::Model),
                "Model {} should NOT be blocked when {} was recorded with Model scope",
                other_model_id, model_id
            );
        }

        /// Property 15 – Case 2: Provider scope blocks all models from the same provider.
        #[test]
        fn prop_provider_scope_blocks_all_same_provider_models(
            provider in arb_provider_prefix(),
            suffix1 in arb_model_suffix(),
            suffix2 in arb_model_suffix(),
            other_provider in arb_provider_prefix(),
            other_suffix in arb_model_suffix(),
            retry_after in 1u64..300,
        ) {
            let model1 = format!("{}/{}", provider, suffix1);
            let model2 = format!("{}/{}", provider, suffix2);
            let other_model = format!("{}/{}", other_provider, other_suffix);
            prop_assume!(provider != other_provider);

            let mgr = RetryAfterStateManager::new();
            // Record at provider level (provider scope records the provider prefix)
            mgr.record(&provider, retry_after, 300);

            // Both models from the same provider should be blocked
            prop_assert!(
                mgr.is_model_blocked(&model1, BlockScope::Provider),
                "Model {} should be blocked with Provider scope after recording provider {}",
                model1, provider
            );
            prop_assert!(
                mgr.is_model_blocked(&model2, BlockScope::Provider),
                "Model {} should be blocked with Provider scope after recording provider {}",
                model2, provider
            );

            // Model from a different provider should NOT be blocked
            prop_assert!(
                !mgr.is_model_blocked(&other_model, BlockScope::Provider),
                "Model {} should NOT be blocked when provider {} was recorded",
                other_model, provider
            );
        }

        /// Property 15 – Case 3: Global state is visible across different "requests"
        /// (same manager instance is shared).
        #[test]
        fn prop_global_state_shared_across_requests(
            model_id in arb_model_id(),
            scope in arb_scope(),
            retry_after in 1u64..300,
        ) {
            let mgr = RetryAfterStateManager::new();

            // Determine the identifier to record based on scope
            let identifier = match scope {
                BlockScope::Model => model_id.clone(),
                BlockScope::Provider => extract_provider(&model_id).to_string(),
            };
            mgr.record(&identifier, retry_after, 300);

            // Simulate "different requests" by checking from the same manager instance.
            // Global state means any check against the same manager sees the block.
            // Check 1 (simulating request A)
            let blocked_a = mgr.is_model_blocked(&model_id, scope);
            // Check 2 (simulating request B)
            let blocked_b = mgr.is_model_blocked(&model_id, scope);

            prop_assert!(
                blocked_a && blocked_b,
                "Global state should be visible to all requests: request_a={}, request_b={}",
                blocked_a, blocked_b
            );
        }

        /// Property 15 – Case 4: Request-scoped state (HashMap) is isolated per request.
        /// Two separate HashMaps don't share state.
        #[test]
        fn prop_request_scoped_state_isolated(
            model_id in arb_model_id(),
            retry_after in 1u64..300,
        ) {
            use std::collections::HashMap;
            use std::time::Instant;

            // Simulate request-scoped state using separate HashMaps
            // (as RequestContext.request_retry_after_state would be)
            let mut request_a_state: HashMap<String, Instant> = HashMap::new();
            let mut request_b_state: HashMap<String, Instant> = HashMap::new();

            // Request A records a Retry-After entry
            let expiration = Instant::now() + Duration::from_secs(retry_after);
            request_a_state.insert(model_id.clone(), expiration);

            // Request A should see the block
            let a_blocked = request_a_state
                .get(&model_id)
                .map_or(false, |exp| Instant::now() < *exp);

            // Request B should NOT see the block (separate HashMap)
            let b_blocked = request_b_state
                .get(&model_id)
                .map_or(false, |exp| Instant::now() < *exp);

            prop_assert!(
                a_blocked,
                "Request A should see its own block for {}",
                model_id
            );
            prop_assert!(
                !b_blocked,
                "Request B should NOT see Request A's block for {}",
                model_id
            );

            // Recording in request B should not affect request A
            let expiration_b = Instant::now() + Duration::from_secs(retry_after);
            request_b_state.insert(model_id.clone(), expiration_b);

            // Both should now be blocked independently
            let a_still_blocked = request_a_state
                .get(&model_id)
                .map_or(false, |exp| Instant::now() < *exp);
            let b_now_blocked = request_b_state
                .get(&model_id)
                .map_or(false, |exp| Instant::now() < *exp);

            prop_assert!(a_still_blocked, "Request A should still be blocked");
            prop_assert!(b_now_blocked, "Request B should now be blocked independently");
        }
    }

    // Feature: retry-on-ratelimit, Property 16: Retry_After_State Max Expiration Update
    // **Validates: Requirements 12.11**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// Property 16: Recording multiple Retry-After values for the same identifier
        /// should result in the expiration reflecting the maximum value, not the most recent.
        #[test]
        fn prop_max_expiration_update(
            identifier in arb_model_id(),
            // Generate 2..=10 Retry-After values, each between 1 and 600 seconds
            retry_after_values in prop::collection::vec(1u64..=600, 2..=10),
            max_cap in 300u64..=600,
        ) {
            let mgr = RetryAfterStateManager::new();

            // Record all values for the same identifier
            for &val in &retry_after_values {
                mgr.record(&identifier, val, max_cap);
            }

            // The effective maximum is the max of all capped values
            let effective_max = retry_after_values
                .iter()
                .map(|&v| v.min(max_cap))
                .max()
                .unwrap();

            // The remaining block duration should be close to the effective maximum
            let remaining = mgr.remaining_block_duration(&identifier);
            prop_assert!(
                remaining.is_some(),
                "Identifier {} should still be blocked after recording {} values (effective_max={}s)",
                identifier, retry_after_values.len(), effective_max
            );

            let remaining_secs = remaining.unwrap().as_secs();

            // The remaining duration should be within a reasonable tolerance of the
            // effective maximum (allow up to 2 seconds for test execution time).
            // It must be at least (effective_max - 2) to prove the max won.
            prop_assert!(
                remaining_secs >= effective_max.saturating_sub(2),
                "Remaining {}s should reflect the max ({}s), not a smaller value. Values: {:?}",
                remaining_secs, effective_max, retry_after_values
            );

            // It should not exceed the effective max (plus small tolerance for timing)
            prop_assert!(
                remaining_secs <= effective_max + 1,
                "Remaining {}s should not exceed effective max {}s + tolerance. Values: {:?}",
                remaining_secs, effective_max, retry_after_values
            );
        }
    }
}
