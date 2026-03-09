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
    pub fn record(
        &self,
        identifier: &str,
        retry_after_seconds: u64,
        max_retry_after_seconds: u64,
    ) {
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

