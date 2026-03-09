use std::time::{Duration, Instant};

use dashmap::DashMap;
use log::info;

use crate::configuration::{extract_provider, BlockScope};

/// Thread-safe global state manager for latency-based blocking.
///
/// Blocks expire only via `block_duration_seconds` — successful requests
/// do NOT remove existing blocks. There is no `remove_block()` method.
///
/// This manager handles ONLY global state (`apply_to: "global"`).
/// Request-scoped state (`apply_to: "request"`) is stored in
/// `RequestContext.request_latency_block_state` and managed by the orchestrator.
///
/// Entries use max-expiration semantics: if a new block is recorded for an
/// identifier that already has an entry, the expiration is updated only if
/// the new expiration is later than the existing one.
pub struct LatencyBlockStateManager {
    /// Global state: identifier (model ID or provider prefix) -> (expiration timestamp, measured_latency_ms)
    global_state: DashMap<String, (Instant, u64)>,
}

impl LatencyBlockStateManager {
    pub fn new() -> Self {
        Self {
            global_state: DashMap::new(),
        }
    }

    /// Record a latency block after min_triggers threshold is met.
    ///
    /// If an entry already exists for the identifier, updates only if the new
    /// expiration is later than the existing one (max-expiration semantics).
    /// The `measured_latency_ms` is always updated to the latest value when
    /// the expiration is extended.
    pub fn record_block(
        &self,
        identifier: &str,
        block_duration_seconds: u64,
        measured_latency_ms: u64,
    ) {
        let new_expiration = Instant::now() + Duration::from_secs(block_duration_seconds);

        self.global_state
            .entry(identifier.to_string())
            .and_modify(|existing| {
                if new_expiration > existing.0 {
                    existing.0 = new_expiration;
                    existing.1 = measured_latency_ms;
                }
            })
            .or_insert((new_expiration, measured_latency_ms));
    }

    /// Check if an identifier is currently blocked.
    ///
    /// Lazily cleans up expired entries.
    pub fn is_blocked(&self, identifier: &str) -> bool {
        if let Some(entry) = self.global_state.get(identifier) {
            if Instant::now() < entry.0 {
                return true;
            }
            // Entry expired — drop the read guard before removing
            drop(entry);
            self.global_state.remove(identifier);
            info!("Latency_Block_State expired: identifier={}", identifier);
            info!(
                "metric.latency_block_expired: model={}",
                identifier
            );
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
            if now < entry.0 {
                return Some(entry.0 - now);
            }
            // Entry expired — drop the read guard before removing
            drop(entry);
            self.global_state.remove(identifier);
            info!("Latency_Block_State expired: identifier={}", identifier);
            info!(
                "metric.latency_block_expired: model={}",
                identifier
            );
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

impl Default for LatencyBlockStateManager {
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
        let mgr = LatencyBlockStateManager::new();
        assert!(!mgr.is_blocked("openai/gpt-4o"));
        assert!(mgr.remaining_block_duration("openai/gpt-4o").is_none());
    }

    #[test]
    fn test_record_block_and_is_blocked() {
        let mgr = LatencyBlockStateManager::new();
        mgr.record_block("openai/gpt-4o", 60, 5500);
        assert!(mgr.is_blocked("openai/gpt-4o"));
        assert!(!mgr.is_blocked("anthropic/claude"));
    }

    #[test]
    fn test_remaining_block_duration() {
        let mgr = LatencyBlockStateManager::new();
        mgr.record_block("openai/gpt-4o", 10, 5000);
        let remaining = mgr.remaining_block_duration("openai/gpt-4o").unwrap();
        assert!(remaining <= Duration::from_secs(11));
        assert!(remaining > Duration::from_secs(8));
    }

    #[test]
    fn test_expired_entry_cleaned_up_on_is_blocked() {
        let mgr = LatencyBlockStateManager::new();
        mgr.record_block("openai/gpt-4o", 0, 5000);
        thread::sleep(Duration::from_millis(10));
        assert!(!mgr.is_blocked("openai/gpt-4o"));
    }

    #[test]
    fn test_expired_entry_cleaned_up_on_remaining() {
        let mgr = LatencyBlockStateManager::new();
        mgr.record_block("openai/gpt-4o", 0, 5000);
        thread::sleep(Duration::from_millis(10));
        assert!(mgr.remaining_block_duration("openai/gpt-4o").is_none());
    }

    #[test]
    fn test_max_expiration_semantics_longer_wins() {
        let mgr = LatencyBlockStateManager::new();
        mgr.record_block("openai/gpt-4o", 10, 5000);
        let first_remaining = mgr.remaining_block_duration("openai/gpt-4o").unwrap();

        mgr.record_block("openai/gpt-4o", 60, 6000);
        let second_remaining = mgr.remaining_block_duration("openai/gpt-4o").unwrap();
        assert!(second_remaining > first_remaining);
    }

    #[test]
    fn test_max_expiration_semantics_shorter_does_not_overwrite() {
        let mgr = LatencyBlockStateManager::new();
        mgr.record_block("openai/gpt-4o", 60, 5000);
        let first_remaining = mgr.remaining_block_duration("openai/gpt-4o").unwrap();

        mgr.record_block("openai/gpt-4o", 5, 6000);
        let second_remaining = mgr.remaining_block_duration("openai/gpt-4o").unwrap();
        // Should still be close to the original 60s
        assert!(second_remaining > Duration::from_secs(50));
        let diff = if first_remaining > second_remaining {
            first_remaining - second_remaining
        } else {
            second_remaining - first_remaining
        };
        assert!(diff < Duration::from_secs(2));
    }

    #[test]
    fn test_is_model_blocked_model_scope() {
        let mgr = LatencyBlockStateManager::new();
        mgr.record_block("openai/gpt-4o", 60, 5000);

        assert!(mgr.is_model_blocked("openai/gpt-4o", BlockScope::Model));
        assert!(!mgr.is_model_blocked("openai/gpt-4o-mini", BlockScope::Model));
    }

    #[test]
    fn test_is_model_blocked_provider_scope() {
        let mgr = LatencyBlockStateManager::new();
        mgr.record_block("openai", 60, 5000);

        assert!(mgr.is_model_blocked("openai/gpt-4o", BlockScope::Provider));
        assert!(mgr.is_model_blocked("openai/gpt-4o-mini", BlockScope::Provider));
        assert!(!mgr.is_model_blocked("anthropic/claude", BlockScope::Provider));
    }

    #[test]
    fn test_multiple_identifiers_independent() {
        let mgr = LatencyBlockStateManager::new();
        mgr.record_block("openai/gpt-4o", 60, 5000);
        mgr.record_block("anthropic/claude", 30, 4000);

        assert!(mgr.is_blocked("openai/gpt-4o"));
        assert!(mgr.is_blocked("anthropic/claude"));
        assert!(!mgr.is_blocked("azure/gpt-4o"));
    }

    #[test]
    fn test_record_block_stores_measured_latency() {
        let mgr = LatencyBlockStateManager::new();
        mgr.record_block("openai/gpt-4o", 60, 5500);

        // Verify the entry exists and has the correct latency
        let entry = mgr.global_state.get("openai/gpt-4o").unwrap();
        assert_eq!(entry.1, 5500);
    }

    #[test]
    fn test_latency_updated_when_expiration_extended() {
        let mgr = LatencyBlockStateManager::new();
        mgr.record_block("openai/gpt-4o", 10, 5000);

        // Extend with longer duration and different latency
        mgr.record_block("openai/gpt-4o", 60, 7000);

        let entry = mgr.global_state.get("openai/gpt-4o").unwrap();
        assert_eq!(entry.1, 7000);
    }

    #[test]
    fn test_latency_not_updated_when_expiration_not_extended() {
        let mgr = LatencyBlockStateManager::new();
        mgr.record_block("openai/gpt-4o", 60, 5000);

        // Shorter duration — should NOT update
        mgr.record_block("openai/gpt-4o", 5, 9000);

        let entry = mgr.global_state.get("openai/gpt-4o").unwrap();
        // Latency should remain 5000 since expiration wasn't extended
        assert_eq!(entry.1, 5000);
    }

    #[test]
    fn test_zero_duration_block_expires_immediately() {
        let mgr = LatencyBlockStateManager::new();
        mgr.record_block("openai/gpt-4o", 0, 5000);
        thread::sleep(Duration::from_millis(5));
        assert!(!mgr.is_blocked("openai/gpt-4o"));
    }

    #[test]
    fn test_default_trait() {
        let mgr = LatencyBlockStateManager::default();
        assert!(!mgr.is_blocked("anything"));
    }

    // --- Property-based tests ---

    use proptest::prelude::*;

    fn arb_identifier() -> impl Strategy<Value = String> {
        prop_oneof![
            "[a-z]{3,8}/[a-z0-9\\-]{3,12}".prop_map(|s| s),
            "[a-z]{3,8}".prop_map(|s| s),
        ]
    }

    /// A single block recording: (block_duration_seconds, measured_latency_ms)
    fn arb_block_recording() -> impl Strategy<Value = (u64, u64)> {
        (1u64..=600, 100u64..=30_000)
    }

    // Feature: retry-on-ratelimit, Property 22: Latency Block State Max Expiration Update
    // **Validates: Requirements 14.15**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// Property 22 – Case 1: After recording multiple blocks for the same identifier
        /// with different durations, the remaining block duration reflects the maximum
        /// duration recorded (max-expiration semantics).
        #[test]
        fn prop_latency_block_max_expiration_update(
            identifier in arb_identifier(),
            recordings in prop::collection::vec(arb_block_recording(), 2..=10),
        ) {
            let mgr = LatencyBlockStateManager::new();

            for &(duration, latency) in &recordings {
                mgr.record_block(&identifier, duration, latency);
            }

            let max_duration = recordings.iter().map(|&(d, _)| d).max().unwrap();

            // The identifier should still be blocked
            let remaining = mgr.remaining_block_duration(&identifier);
            prop_assert!(
                remaining.is_some(),
                "Identifier {} should be blocked after {} recordings (max_duration={}s)",
                identifier, recordings.len(), max_duration
            );

            let remaining_secs = remaining.unwrap().as_secs();

            // Remaining should be close to max_duration (allow 2s tolerance for execution time)
            prop_assert!(
                remaining_secs >= max_duration.saturating_sub(2),
                "Remaining {}s should reflect the max duration ({}s), not a smaller value. Recordings: {:?}",
                remaining_secs, max_duration, recordings
            );

            prop_assert!(
                remaining_secs <= max_duration + 1,
                "Remaining {}s should not exceed max duration {}s + tolerance. Recordings: {:?}",
                remaining_secs, max_duration, recordings
            );
        }

        /// Property 22 – Case 2: measured_latency_ms is updated when expiration is extended
        /// but NOT when a shorter duration is recorded.
        #[test]
        fn prop_latency_block_measured_latency_update_semantics(
            identifier in arb_identifier(),
            first_duration in 10u64..=300,
            first_latency in 100u64..=30_000,
            extra_duration in 1u64..=300,
            longer_latency in 100u64..=30_000,
            shorter_duration in 1u64..=9,
            shorter_latency in 100u64..=30_000,
        ) {
            let mgr = LatencyBlockStateManager::new();

            // Record initial block
            mgr.record_block(&identifier, first_duration, first_latency);
            {
                let entry = mgr.global_state.get(&identifier).unwrap();
                prop_assert_eq!(entry.1, first_latency);
            }

            // Record a longer duration — latency SHOULD be updated
            let longer_duration = first_duration + extra_duration;
            mgr.record_block(&identifier, longer_duration, longer_latency);
            {
                let entry = mgr.global_state.get(&identifier).unwrap();
                prop_assert_eq!(
                    entry.1, longer_latency,
                    "Latency should be updated to {} when expiration is extended (duration {} > {})",
                    longer_latency, longer_duration, first_duration
                );
            }

            // Record a shorter duration — latency should NOT be updated
            mgr.record_block(&identifier, shorter_duration, shorter_latency);
            {
                let entry = mgr.global_state.get(&identifier).unwrap();
                prop_assert_eq!(
                    entry.1, longer_latency,
                    "Latency should remain {} (not {}) when shorter duration {} < {} doesn't extend expiration",
                    longer_latency, shorter_latency, shorter_duration, longer_duration
                );
            }
        }
    }
}

