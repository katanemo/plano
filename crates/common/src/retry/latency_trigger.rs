use std::time::Instant;

use dashmap::DashMap;

/// Thread-safe sliding window counter for tracking High_Latency_Events.
///
/// Maintains per-identifier timestamps of latency events within a configurable
/// sliding window. When the count of recent events meets or exceeds `min_triggers`,
/// the caller should create a `Latency_Block_State` entry and then call `reset()`.
pub struct LatencyTriggerCounter {
    /// model/provider identifier -> list of event timestamps within the window
    counters: DashMap<String, Vec<Instant>>,
}

impl LatencyTriggerCounter {
    pub fn new() -> Self {
        Self {
            counters: DashMap::new(),
        }
    }

    /// Record a High_Latency_Event. Returns true if `min_triggers` threshold
    /// is now met (caller should create a Latency_Block_State).
    ///
    /// Lazily discards events older than `trigger_window_seconds` before checking
    /// the count.
    pub fn record_event(
        &self,
        identifier: &str,
        min_triggers: u32,
        trigger_window_seconds: u64,
    ) -> bool {
        let now = Instant::now();
        let window = std::time::Duration::from_secs(trigger_window_seconds);

        let mut entry = self.counters.entry(identifier.to_string()).or_default();
        // Add current event
        entry.push(now);
        // Discard events older than the window
        entry.retain(|ts| now.duration_since(*ts) <= window);
        // Check threshold
        entry.len() >= min_triggers as usize
    }

    /// Reset the counter for an identifier (called after a block is created
    /// to prevent re-triggering on the same events).
    pub fn reset(&self, identifier: &str) {
        if let Some(mut entry) = self.counters.get_mut(identifier) {
            entry.clear();
        }
    }
}

impl Default for LatencyTriggerCounter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn test_record_event_returns_true_when_threshold_met() {
        let counter = LatencyTriggerCounter::new();
        assert!(!counter.record_event("model-a", 3, 60));
        assert!(!counter.record_event("model-a", 3, 60));
        assert!(counter.record_event("model-a", 3, 60));
    }

    #[test]
    fn test_record_event_single_trigger_always_fires() {
        let counter = LatencyTriggerCounter::new();
        assert!(counter.record_event("model-a", 1, 60));
    }

    #[test]
    fn test_events_expire_outside_window() {
        let counter = LatencyTriggerCounter::new();
        // Record 2 events
        counter.record_event("model-a", 3, 1);
        counter.record_event("model-a", 3, 1);
        // Wait for them to expire
        sleep(Duration::from_millis(1100));
        // Third event should not meet threshold since previous two expired
        assert!(!counter.record_event("model-a", 3, 1));
    }

    #[test]
    fn test_reset_clears_counter() {
        let counter = LatencyTriggerCounter::new();
        counter.record_event("model-a", 3, 60);
        counter.record_event("model-a", 3, 60);
        counter.reset("model-a");
        // After reset, need 3 fresh events again
        assert!(!counter.record_event("model-a", 3, 60));
        assert!(!counter.record_event("model-a", 3, 60));
        assert!(counter.record_event("model-a", 3, 60));
    }

    #[test]
    fn test_reset_nonexistent_identifier_is_noop() {
        let counter = LatencyTriggerCounter::new();
        // Should not panic
        counter.reset("nonexistent");
    }

    #[test]
    fn test_separate_identifiers_are_independent() {
        let counter = LatencyTriggerCounter::new();
        counter.record_event("model-a", 2, 60);
        counter.record_event("model-b", 2, 60);
        // model-a has 1 event, model-b has 1 event — neither at threshold of 2
        assert!(!counter.record_event("model-b", 3, 60));
        // model-a reaches threshold
        assert!(counter.record_event("model-a", 2, 60));
    }

    #[test]
    fn test_threshold_exceeded_still_returns_true() {
        let counter = LatencyTriggerCounter::new();
        assert!(counter.record_event("model-a", 1, 60));
        // Already past threshold, still returns true
        assert!(counter.record_event("model-a", 1, 60));
        assert!(counter.record_event("model-a", 1, 60));
    }

    // --- Property-based tests ---

    use proptest::prelude::*;

    // Feature: retry-on-ratelimit, Property 18: Latency Trigger Counter Sliding Window
    // **Validates: Requirements 2a.6, 2a.7, 2a.8, 2a.21, 14.1, 14.2, 14.3, 14.12**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// Property 18 – Case 1: Recording N events in quick succession (all within window)
        /// returns true iff N >= min_triggers.
        #[test]
        fn prop_sliding_window_threshold(
            min_triggers in 1u32..=10,
            trigger_window_seconds in 1u64..=60,
            num_events in 1u32..=20,
        ) {
            let counter = LatencyTriggerCounter::new();
            let identifier = "test-model";

            let mut last_result = false;
            for i in 1..=num_events {
                last_result = counter.record_event(identifier, min_triggers, trigger_window_seconds);
                // Before reaching threshold, should be false
                if i < min_triggers {
                    prop_assert!(!last_result, "Expected false at event {} with min_triggers {}", i, min_triggers);
                } else {
                    // At or past threshold, should be true
                    prop_assert!(last_result, "Expected true at event {} with min_triggers {}", i, min_triggers);
                }
            }

            // Final result should match whether we recorded enough events
            prop_assert_eq!(last_result, num_events >= min_triggers);
        }

        /// Property 18 – Case 2: After reset, counter starts fresh and previous events
        /// do not count toward the threshold.
        #[test]
        fn prop_reset_clears_counter(
            min_triggers in 2u32..=10,
            trigger_window_seconds in 1u64..=60,
            events_before_reset in 1u32..=10,
        ) {
            let counter = LatencyTriggerCounter::new();
            let identifier = "test-model";

            // Record some events before reset
            for _ in 0..events_before_reset {
                counter.record_event(identifier, min_triggers, trigger_window_seconds);
            }

            // Reset the counter
            counter.reset(identifier);

            // After reset, a single event should not meet threshold (min_triggers >= 2)
            let result = counter.record_event(identifier, min_triggers, trigger_window_seconds);
            prop_assert!(!result, "After reset, first event should not meet threshold of {}", min_triggers);

            // Need min_triggers - 1 more events to reach threshold again
            let mut final_result = result;
            for _ in 1..min_triggers {
                final_result = counter.record_event(identifier, min_triggers, trigger_window_seconds);
            }
            prop_assert!(final_result, "After reset + {} events, should meet threshold", min_triggers);
        }

        /// Property 18 – Case 3: Different identifiers are independent — events for one
        /// identifier do not affect the count for another.
        #[test]
        fn prop_identifiers_independent(
            min_triggers in 1u32..=10,
            trigger_window_seconds in 1u64..=60,
            events_a in 1u32..=20,
            events_b in 1u32..=20,
        ) {
            let counter = LatencyTriggerCounter::new();
            let id_a = "model-a";
            let id_b = "model-b";

            // Record events for identifier A
            let mut result_a = false;
            for _ in 0..events_a {
                result_a = counter.record_event(id_a, min_triggers, trigger_window_seconds);
            }

            // Record events for identifier B
            let mut result_b = false;
            for _ in 0..events_b {
                result_b = counter.record_event(id_b, min_triggers, trigger_window_seconds);
            }

            // Each identifier's result depends only on its own event count
            prop_assert_eq!(result_a, events_a >= min_triggers,
                "id_a: events={}, min_triggers={}", events_a, min_triggers);
            prop_assert_eq!(result_b, events_b >= min_triggers,
                "id_b: events={}, min_triggers={}", events_b, min_triggers);
        }
    }
} // mod tests
