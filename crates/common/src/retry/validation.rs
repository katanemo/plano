use std::collections::HashSet;

use crate::configuration::{
    BackoffApplyTo, BlockScope, LlmProvider, RetryStrategy, StatusCodeEntry,
};
use crate::retry::{ValidationError, ValidationWarning};

/// Validates retry policy configurations across all model providers.
pub struct ConfigValidator;

impl ConfigValidator {
    /// Validate all retry_policy configurations across all model providers.
    /// Returns Ok(warnings) on success, Err(errors) on failure.
    pub fn validate_retry_policies(
        providers: &[LlmProvider],
    ) -> Result<Vec<ValidationWarning>, Vec<ValidationError>> {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        let all_models: HashSet<String> = providers
            .iter()
            .filter_map(|p| p.model.clone())
            .collect();

        for provider in providers {
            let model_id = provider
                .model
                .as_deref()
                .unwrap_or(&provider.name);

            let policy = match &provider.retry_policy {
                Some(p) => p,
                None => continue,
            };

            // Validate on_status_codes entries
            let mut all_seen_codes: Vec<u16> = Vec::new();
            for sc_config in &policy.on_status_codes {
                for entry in &sc_config.codes {
                    match entry {
                        StatusCodeEntry::Single(code) => {
                            if *code < 100 || *code > 599 {
                                errors.push(ValidationError::StatusCodeOutOfRange {
                                    model: model_id.to_string(),
                                    code: *code,
                                });
                            }
                        }
                        StatusCodeEntry::Range(range_str) => {
                            match entry.expand() {
                                Ok(codes) => {
                                    for code in &codes {
                                        if *code < 100 || *code > 599 {
                                            errors.push(ValidationError::StatusCodeOutOfRange {
                                                model: model_id.to_string(),
                                                code: *code,
                                            });
                                        }
                                    }
                                }
                                Err(_) => {
                                    // Check if it's an inverted range or invalid format
                                    let parts: Vec<&str> = range_str.split('-').collect();
                                    if parts.len() == 2 {
                                        if let (Ok(start), Ok(end)) = (
                                            parts[0].trim().parse::<u16>(),
                                            parts[1].trim().parse::<u16>(),
                                        ) {
                                            if start > end {
                                                errors.push(ValidationError::StatusCodeRangeInverted {
                                                    model: model_id.to_string(),
                                                    range: range_str.clone(),
                                                });
                                            } else {
                                                errors.push(ValidationError::StatusCodeRangeInvalid {
                                                    model: model_id.to_string(),
                                                    range: range_str.clone(),
                                                });
                                            }
                                        } else {
                                            errors.push(ValidationError::StatusCodeRangeInvalid {
                                                model: model_id.to_string(),
                                                range: range_str.clone(),
                                            });
                                        }
                                    } else {
                                        errors.push(ValidationError::StatusCodeRangeInvalid {
                                            model: model_id.to_string(),
                                            range: range_str.clone(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }

                // Collect expanded codes for overlap detection
                if let Ok(expanded) = Self::expand_status_codes(&sc_config.codes) {
                    for code in &expanded {
                        if all_seen_codes.contains(code) {
                            warnings.push(ValidationWarning::OverlappingStatusCodes {
                                model: model_id.to_string(),
                                code: *code,
                            });
                        }
                    }
                    all_seen_codes.extend(expanded);
                }
            }

            // Validate backoff config
            if let Some(backoff) = &policy.backoff {
                if backoff.base_ms == 0 {
                    errors.push(ValidationError::NonPositiveValue {
                        model: model_id.to_string(),
                        field: "backoff.base_ms".to_string(),
                    });
                }
                if backoff.max_ms <= backoff.base_ms {
                    errors.push(ValidationError::MaxMsNotGreaterThanBaseMs {
                        model: model_id.to_string(),
                        base_ms: backoff.base_ms,
                        max_ms: backoff.max_ms,
                    });
                }

                // Warn on backoff apply_to mismatch with default strategy
                match (backoff.apply_to, policy.default_strategy) {
                    (BackoffApplyTo::SameModel, RetryStrategy::DifferentProvider) => {
                        warnings.push(ValidationWarning::BackoffApplyToMismatch {
                            model: model_id.to_string(),
                            apply_to: "same_model".to_string(),
                            strategy: "different_provider".to_string(),
                        });
                    }
                    (BackoffApplyTo::SameProvider, RetryStrategy::SameModel) => {
                        warnings.push(ValidationWarning::BackoffApplyToMismatch {
                            model: model_id.to_string(),
                            apply_to: "same_provider".to_string(),
                            strategy: "same_model".to_string(),
                        });
                    }
                    _ => {}
                }
            }

            // Validate max_retry_duration_ms
            if let Some(max_dur) = policy.max_retry_duration_ms {
                if max_dur == 0 {
                    errors.push(ValidationError::NonPositiveValue {
                        model: model_id.to_string(),
                        field: "max_retry_duration_ms".to_string(),
                    });
                }
            }

            // Warn: single provider with failover strategy
            if providers.len() == 1 {
                match policy.default_strategy {
                    RetryStrategy::SameProvider | RetryStrategy::DifferentProvider => {
                        warnings.push(ValidationWarning::SingleProviderWithFailover {
                            model: model_id.to_string(),
                            strategy: format!("{:?}", policy.default_strategy)
                                .to_ascii_lowercase(),
                        });
                    }
                    _ => {}
                }
            }

            // Warn: fallback model not in Provider_List
            for fallback in &policy.fallback_models {
                if !all_models.contains(fallback) {
                    warnings.push(ValidationWarning::FallbackModelNotInProviderList {
                        model: model_id.to_string(),
                        fallback: fallback.clone(),
                    });
                }
            }

            // ── P1 Validations ─────────────────────────────────────────────

            // Validate on_timeout: max_attempts must be > 0
            if let Some(ref timeout_config) = policy.on_timeout {
                if timeout_config.max_attempts == 0 {
                    errors.push(ValidationError::NonPositiveValue {
                        model: model_id.to_string(),
                        field: "on_timeout.max_attempts".to_string(),
                    });
                }
            }

            // Validate retry_after_handling: max_retry_after_seconds must be > 0
            if let Some(ref ra_config) = policy.retry_after_handling {
                if ra_config.max_retry_after_seconds == 0 {
                    errors.push(ValidationError::NonPositiveValue {
                        model: model_id.to_string(),
                        field: "retry_after_handling.max_retry_after_seconds".to_string(),
                    });
                }
            }

            // Validate fallback_models entries: non-empty and contain "/"
            for fallback in &policy.fallback_models {
                if fallback.is_empty() || !fallback.contains('/') {
                    errors.push(ValidationError::InvalidFallbackModel {
                        model: model_id.to_string(),
                        fallback: fallback.clone(),
                    });
                }
            }

            // Warn: provider-scope RA with same_model strategy
            if let Some(ref ra_config) = policy.retry_after_handling {
                if ra_config.scope == BlockScope::Provider
                    && policy.default_strategy == RetryStrategy::SameModel
                {
                    warnings.push(ValidationWarning::ProviderScopeWithSameModel {
                        model: model_id.to_string(),
                    });
                }
            }

            // ── P2 Validations ─────────────────────────────────────────────

            if let Some(ref hl_config) = policy.on_high_latency {
                // threshold_ms must be positive
                if hl_config.threshold_ms == 0 {
                    errors.push(ValidationError::NonPositiveValue {
                        model: model_id.to_string(),
                        field: "on_high_latency.threshold_ms".to_string(),
                    });
                }

                // max_attempts must be > 0
                if hl_config.max_attempts == 0 {
                    errors.push(ValidationError::NonPositiveValue {
                        model: model_id.to_string(),
                        field: "on_high_latency.max_attempts".to_string(),
                    });
                }

                // block_duration_seconds must be positive
                if hl_config.block_duration_seconds == 0 {
                    errors.push(ValidationError::NonPositiveValue {
                        model: model_id.to_string(),
                        field: "on_high_latency.block_duration_seconds".to_string(),
                    });
                }

                // min_triggers > 1 requires trigger_window_seconds
                if hl_config.min_triggers > 1 && hl_config.trigger_window_seconds.is_none() {
                    errors.push(ValidationError::LatencyMissingTriggerWindow {
                        model: model_id.to_string(),
                    });
                }

                // trigger_window_seconds must be positive when specified
                if let Some(tw) = hl_config.trigger_window_seconds {
                    if tw == 0 {
                        errors.push(ValidationError::NonPositiveTriggerWindow {
                            model: model_id.to_string(),
                        });
                    }
                }

                // Warn: provider-scope latency with same_model strategy
                if hl_config.scope == BlockScope::Provider
                    && hl_config.strategy == RetryStrategy::SameModel
                {
                    warnings.push(ValidationWarning::LatencyScopeStrategyMismatch {
                        model: model_id.to_string(),
                    });
                }

                // Warn: aggressive latency threshold (< 1000ms)
                if hl_config.threshold_ms > 0 && hl_config.threshold_ms < 1000 {
                    warnings.push(ValidationWarning::AggressiveLatencyThreshold {
                        model: model_id.to_string(),
                        threshold_ms: hl_config.threshold_ms,
                    });
                }
            }
        }

        if errors.is_empty() {
            Ok(warnings)
        } else {
            Err(errors)
        }
    }

    /// Parse and expand status code entries (integers + range strings).
    pub fn expand_status_codes(
        codes: &[StatusCodeEntry],
    ) -> Result<Vec<u16>, ValidationError> {
        let mut result = Vec::new();
        for entry in codes {
            match entry.expand() {
                Ok(expanded) => result.extend(expanded),
                Err(msg) => {
                    return Err(ValidationError::StatusCodeRangeInvalid {
                        model: String::new(),
                        range: msg,
                    });
                }
            }
        }
        Ok(result)
    }
}

