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

