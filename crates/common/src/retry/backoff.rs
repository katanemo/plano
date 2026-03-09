use std::time::Duration;

use rand::Rng;

use crate::configuration::{BackoffApplyTo, BackoffConfig, RetryStrategy, extract_provider};

/// Calculator for exponential backoff delays with jitter and scope filtering.
pub struct BackoffCalculator;

impl BackoffCalculator {
    /// Calculate the delay before the next retry attempt.
    ///
    /// Returns the greater of the computed backoff delay and the Retry-After delay.
    /// Returns zero when the backoff `apply_to` scope doesn't match the
    /// current/previous provider relationship (unless retry_after_seconds is set).
    pub fn calculate_delay(
        &self,
        attempt_number: u32,
        backoff_config: Option<&BackoffConfig>,
        retry_after_seconds: Option<u64>,
        current_strategy: RetryStrategy,
        current_provider: &str,
        previous_provider: &str,
    ) -> Duration {
        let backoff_delay = match backoff_config {
            Some(config) => {
                if !Self::scope_matches(config.apply_to, current_strategy, current_provider, previous_provider) {
                    Duration::ZERO
                } else {
                    Self::compute_backoff(attempt_number, config)
                }
            }
            None => Duration::ZERO,
        };

        let retry_after_delay = retry_after_seconds
            .map(|s| Duration::from_secs(s))
            .unwrap_or(Duration::ZERO);

        backoff_delay.max(retry_after_delay)
    }

    /// Check whether the backoff `apply_to` scope matches the current retry context.
    fn scope_matches(
        apply_to: BackoffApplyTo,
        _current_strategy: RetryStrategy,
        current_provider: &str,
        previous_provider: &str,
    ) -> bool {
        let current_prefix = extract_provider(current_provider);
        let previous_prefix = extract_provider(previous_provider);

        match apply_to {
            BackoffApplyTo::SameModel => current_provider == previous_provider,
            BackoffApplyTo::SameProvider => current_prefix == previous_prefix,
            BackoffApplyTo::Global => true,
        }
    }

    /// Compute exponential backoff: min(base_ms * 2^attempt, max_ms), with optional jitter.
    fn compute_backoff(attempt_number: u32, config: &BackoffConfig) -> Duration {
        let exp_delay = if attempt_number >= 64 {
            config.max_ms
        } else {
            config.base_ms.saturating_mul(1u64 << attempt_number)
        };
        let capped = exp_delay.min(config.max_ms);

        let final_ms = if config.jitter {
            let mut rng = rand::thread_rng();
            let jitter_factor: f64 = 0.5 + rng.gen::<f64>() * 0.5;
            ((capped as f64) * jitter_factor) as u64
        } else {
            capped
        };

        Duration::from_millis(final_ms)
    }
}

