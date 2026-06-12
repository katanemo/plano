use std::time::Duration;

use rand::Rng;

use crate::configuration::{extract_provider, BackoffApplyTo, BackoffConfig, RetryStrategy};

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
                if !Self::scope_matches(
                    config.apply_to,
                    current_strategy,
                    current_provider,
                    previous_provider,
                ) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::{BackoffApplyTo, BackoffConfig, RetryStrategy};
    use proptest::prelude::*;

    fn make_config(
        apply_to: BackoffApplyTo,
        base_ms: u64,
        max_ms: u64,
        jitter: bool,
    ) -> BackoffConfig {
        BackoffConfig {
            apply_to,
            base_ms,
            max_ms,
            jitter,
        }
    }

    #[test]
    fn no_backoff_config_returns_zero() {
        let calc = BackoffCalculator;
        let d = calc.calculate_delay(
            0,
            None,
            None,
            RetryStrategy::SameModel,
            "openai/gpt-4o",
            "openai/gpt-4o",
        );
        assert_eq!(d, Duration::ZERO);
    }

    #[test]
    fn no_backoff_config_with_retry_after() {
        let calc = BackoffCalculator;
        let d = calc.calculate_delay(
            0,
            None,
            Some(5),
            RetryStrategy::SameModel,
            "openai/gpt-4o",
            "openai/gpt-4o",
        );
        assert_eq!(d, Duration::from_secs(5));
    }

    #[test]
    fn exponential_backoff_no_jitter() {
        let calc = BackoffCalculator;
        let config = make_config(BackoffApplyTo::Global, 100, 5000, false);

        // attempt 0: min(100 * 2^0, 5000) = 100
        assert_eq!(
            calc.calculate_delay(0, Some(&config), None, RetryStrategy::SameModel, "a", "a"),
            Duration::from_millis(100)
        );
        // attempt 1: min(100 * 2^1, 5000) = 200
        assert_eq!(
            calc.calculate_delay(1, Some(&config), None, RetryStrategy::SameModel, "a", "a"),
            Duration::from_millis(200)
        );
        // attempt 2: min(100 * 2^2, 5000) = 400
        assert_eq!(
            calc.calculate_delay(2, Some(&config), None, RetryStrategy::SameModel, "a", "a"),
            Duration::from_millis(400)
        );
        // attempt 6: min(100 * 64, 5000) = 5000 (capped)
        assert_eq!(
            calc.calculate_delay(6, Some(&config), None, RetryStrategy::SameModel, "a", "a"),
            Duration::from_millis(5000)
        );
    }

    #[test]
    fn jitter_stays_within_bounds() {
        let calc = BackoffCalculator;
        let config = make_config(BackoffApplyTo::Global, 1000, 50000, true);

        for attempt in 0..5 {
            for _ in 0..20 {
                let d = calc.calculate_delay(
                    attempt,
                    Some(&config),
                    None,
                    RetryStrategy::SameModel,
                    "a",
                    "a",
                );
                let base = (1000u64.saturating_mul(1u64 << attempt)).min(50000);
                // jitter: delay * (0.5 + random(0, 0.5)) => [0.5*base, 1.0*base]
                assert!(
                    d.as_millis() >= (base as f64 * 0.5) as u128,
                    "delay {} too low for base {}",
                    d.as_millis(),
                    base
                );
                assert!(
                    d.as_millis() <= base as u128,
                    "delay {} too high for base {}",
                    d.as_millis(),
                    base
                );
            }
        }
    }

    #[test]
    fn scope_same_model_filters_different_providers() {
        let calc = BackoffCalculator;
        let config = make_config(BackoffApplyTo::SameModel, 100, 5000, false);

        // Same model -> backoff applies
        let d = calc.calculate_delay(
            0,
            Some(&config),
            None,
            RetryStrategy::SameModel,
            "openai/gpt-4o",
            "openai/gpt-4o",
        );
        assert_eq!(d, Duration::from_millis(100));

        // Different model, same provider -> no backoff
        let d = calc.calculate_delay(
            0,
            Some(&config),
            None,
            RetryStrategy::SameProvider,
            "openai/gpt-4o-mini",
            "openai/gpt-4o",
        );
        assert_eq!(d, Duration::ZERO);

        // Different provider -> no backoff
        let d = calc.calculate_delay(
            0,
            Some(&config),
            None,
            RetryStrategy::DifferentProvider,
            "anthropic/claude",
            "openai/gpt-4o",
        );
        assert_eq!(d, Duration::ZERO);
    }

    #[test]
    fn scope_same_provider_filters_different_providers() {
        let calc = BackoffCalculator;
        let config = make_config(BackoffApplyTo::SameProvider, 100, 5000, false);

        // Same provider -> backoff applies
        let d = calc.calculate_delay(
            0,
            Some(&config),
            None,
            RetryStrategy::SameProvider,
            "openai/gpt-4o-mini",
            "openai/gpt-4o",
        );
        assert_eq!(d, Duration::from_millis(100));

        // Same model (same provider) -> backoff applies
        let d = calc.calculate_delay(
            0,
            Some(&config),
            None,
            RetryStrategy::SameModel,
            "openai/gpt-4o",
            "openai/gpt-4o",
        );
        assert_eq!(d, Duration::from_millis(100));

        // Different provider -> no backoff
        let d = calc.calculate_delay(
            0,
            Some(&config),
            None,
            RetryStrategy::DifferentProvider,
            "anthropic/claude",
            "openai/gpt-4o",
        );
        assert_eq!(d, Duration::ZERO);
    }

    #[test]
    fn scope_global_always_applies() {
        let calc = BackoffCalculator;
        let config = make_config(BackoffApplyTo::Global, 100, 5000, false);

        let d = calc.calculate_delay(
            0,
            Some(&config),
            None,
            RetryStrategy::DifferentProvider,
            "anthropic/claude",
            "openai/gpt-4o",
        );
        assert_eq!(d, Duration::from_millis(100));
    }

    #[test]
    fn retry_after_wins_when_greater() {
        let calc = BackoffCalculator;
        let config = make_config(BackoffApplyTo::Global, 100, 5000, false);

        // retry_after = 10s >> backoff attempt 0 = 100ms
        let d = calc.calculate_delay(
            0,
            Some(&config),
            Some(10),
            RetryStrategy::SameModel,
            "a",
            "a",
        );
        assert_eq!(d, Duration::from_secs(10));
    }

    #[test]
    fn backoff_wins_when_greater() {
        let calc = BackoffCalculator;
        // base_ms=10000, attempt 0 -> 10000ms = 10s
        let config = make_config(BackoffApplyTo::Global, 10000, 50000, false);

        // retry_after = 5s < backoff = 10s
        let d = calc.calculate_delay(
            0,
            Some(&config),
            Some(5),
            RetryStrategy::SameModel,
            "a",
            "a",
        );
        assert_eq!(d, Duration::from_millis(10000));
    }

    #[test]
    fn scope_mismatch_still_honors_retry_after() {
        let calc = BackoffCalculator;
        let config = make_config(BackoffApplyTo::SameModel, 100, 5000, false);

        // Scope doesn't match (different providers) but retry_after is set
        let d = calc.calculate_delay(
            0,
            Some(&config),
            Some(3),
            RetryStrategy::DifferentProvider,
            "anthropic/claude",
            "openai/gpt-4o",
        );
        assert_eq!(d, Duration::from_secs(3));
    }

    #[test]
    fn large_attempt_number_saturates() {
        let calc = BackoffCalculator;
        let config = make_config(BackoffApplyTo::Global, 100, 5000, false);

        // Very large attempt number should saturate and cap at max_ms
        let d = calc.calculate_delay(63, Some(&config), None, RetryStrategy::SameModel, "a", "a");
        assert_eq!(d, Duration::from_millis(5000));
    }

    // --- Proptest strategies ---

    fn arb_provider() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("openai/gpt-4o".to_string()),
            Just("openai/gpt-4o-mini".to_string()),
            Just("anthropic/claude-3".to_string()),
            Just("azure/gpt-4o".to_string()),
            Just("google/gemini-pro".to_string()),
        ]
    }

    // Feature: retry-on-ratelimit, Property 12: Exponential Backoff Formula and Bounds
    // **Validates: Requirements 4.6, 4.7, 4.8, 4.9, 4.10, 4.11**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// Property 12 – Case 1: No-jitter delay equals min(base_ms * 2^attempt, max_ms) exactly.
        #[test]
        fn prop_backoff_no_jitter_exact(
            attempt in 0u32..20,
            base_ms in 1u64..10000,
            extra in 1u64..40001u64,
        ) {
            let max_ms = base_ms + extra;
            let config = make_config(BackoffApplyTo::Global, base_ms, max_ms, false);
            let calc = BackoffCalculator;
            let d = calc.calculate_delay(attempt, Some(&config), None, RetryStrategy::SameModel, "a", "a");

            let expected = if attempt >= 64 {
                max_ms
            } else {
                base_ms.saturating_mul(1u64 << attempt).min(max_ms)
            };
            prop_assert_eq!(d, Duration::from_millis(expected));
        }

        /// Property 12 – Case 2: Jitter delay is in [0.5 * computed_base, computed_base].
        #[test]
        fn prop_backoff_jitter_bounds(
            attempt in 0u32..20,
            base_ms in 1u64..10000,
            extra in 1u64..40001u64,
        ) {
            let max_ms = base_ms + extra;
            let config = make_config(BackoffApplyTo::Global, base_ms, max_ms, true);
            let calc = BackoffCalculator;
            let d = calc.calculate_delay(attempt, Some(&config), None, RetryStrategy::SameModel, "a", "a");

            let computed_base = if attempt >= 64 {
                max_ms
            } else {
                base_ms.saturating_mul(1u64 << attempt).min(max_ms)
            };
            let lower = (computed_base as f64 * 0.5) as u64;
            let upper = computed_base;
            prop_assert!(
                d.as_millis() >= lower as u128 && d.as_millis() <= upper as u128,
                "delay {}ms not in [{}, {}] for attempt={}, base_ms={}, max_ms={}",
                d.as_millis(), lower, upper, attempt, base_ms, max_ms
            );
        }

        /// Property 12 – Case 3: Delay is always <= max_ms.
        #[test]
        fn prop_backoff_delay_capped_at_max(
            attempt in 0u32..20,
            base_ms in 1u64..10000,
            extra in 1u64..40001u64,
            jitter in proptest::bool::ANY,
        ) {
            let max_ms = base_ms + extra;
            let config = make_config(BackoffApplyTo::Global, base_ms, max_ms, jitter);
            let calc = BackoffCalculator;
            let d = calc.calculate_delay(attempt, Some(&config), None, RetryStrategy::SameModel, "a", "a");

            prop_assert!(
                d.as_millis() <= max_ms as u128,
                "delay {}ms exceeds max_ms {} for attempt={}, base_ms={}, jitter={}",
                d.as_millis(), max_ms, attempt, base_ms, jitter
            );
        }
    }

    // Feature: retry-on-ratelimit, Property 13: Backoff Apply-To Scope Filtering
    // **Validates: Requirements 4.3, 4.4, 4.5, 4.12, 4.13**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// Property 13 – Case 1: SameModel apply_to with different providers → zero delay.
        #[test]
        fn prop_scope_same_model_different_providers_zero(
            attempt in 0u32..20,
            base_ms in 1u64..10000,
            extra in 1u64..40001u64,
            current in arb_provider(),
            previous in arb_provider(),
        ) {
            // Only test when providers are actually different models
            prop_assume!(current != previous);
            let max_ms = base_ms + extra;
            let config = make_config(BackoffApplyTo::SameModel, base_ms, max_ms, false);
            let calc = BackoffCalculator;
            let d = calc.calculate_delay(
                attempt, Some(&config), None,
                RetryStrategy::DifferentProvider, &current, &previous,
            );
            prop_assert_eq!(d, Duration::ZERO,
                "Expected zero delay for SameModel apply_to with different models: {} vs {}",
                current, previous
            );
        }

        /// Property 13 – Case 2: SameProvider apply_to with different provider prefixes → zero delay.
        #[test]
        fn prop_scope_same_provider_different_prefix_zero(
            attempt in 0u32..20,
            base_ms in 1u64..10000,
            extra in 1u64..40001u64,
            current in arb_provider(),
            previous in arb_provider(),
        ) {
            let current_prefix = extract_provider(&current);
            let previous_prefix = extract_provider(&previous);
            prop_assume!(current_prefix != previous_prefix);
            let max_ms = base_ms + extra;
            let config = make_config(BackoffApplyTo::SameProvider, base_ms, max_ms, false);
            let calc = BackoffCalculator;
            let d = calc.calculate_delay(
                attempt, Some(&config), None,
                RetryStrategy::DifferentProvider, &current, &previous,
            );
            prop_assert_eq!(d, Duration::ZERO,
                "Expected zero delay for SameProvider apply_to with different prefixes: {} vs {}",
                current_prefix, previous_prefix
            );
        }

        /// Property 13 – Case 3: Global apply_to always produces non-zero delay.
        #[test]
        fn prop_scope_global_always_nonzero(
            attempt in 0u32..20,
            base_ms in 1u64..10000,
            extra in 1u64..40001u64,
            current in arb_provider(),
            previous in arb_provider(),
        ) {
            let max_ms = base_ms + extra;
            let config = make_config(BackoffApplyTo::Global, base_ms, max_ms, false);
            let calc = BackoffCalculator;
            let d = calc.calculate_delay(
                attempt, Some(&config), None,
                RetryStrategy::DifferentProvider, &current, &previous,
            );
            prop_assert!(d > Duration::ZERO,
                "Expected non-zero delay for Global apply_to: current={}, previous={}",
                current, previous
            );
        }
    }
}
