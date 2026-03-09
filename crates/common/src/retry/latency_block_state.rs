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

