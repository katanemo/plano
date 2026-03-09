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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::{
        ApplyTo, BackoffConfig, BackoffApplyTo, BlockScope, HighLatencyConfig,
        LatencyMeasure, RetryAfterHandlingConfig,
        RetryPolicy, RetryStrategy, StatusCodeConfig, StatusCodeEntry,
        TimeoutRetryConfig,
    };
    use proptest::prelude::*;

    fn make_provider(model: &str, policy: Option<RetryPolicy>) -> LlmProvider {
        LlmProvider {
            model: Some(model.to_string()),
            retry_policy: policy,
            ..LlmProvider::default()
        }
    }

    fn basic_policy() -> RetryPolicy {
        RetryPolicy {
            fallback_models: vec![],
            default_strategy: RetryStrategy::DifferentProvider,
            default_max_attempts: 2,
            on_status_codes: vec![],
            on_timeout: None,
            on_high_latency: None,
            backoff: None,
            retry_after_handling: None,
            max_retry_duration_ms: None,
        }
    }

    #[test]
    fn test_valid_basic_policy_no_errors() {
        let providers = vec![
            make_provider("openai/gpt-4o", Some(basic_policy())),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_ok());
    }

    #[test]
    fn test_no_retry_policy_skipped() {
        let providers = vec![make_provider("openai/gpt-4o", None)];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_status_code_out_of_range() {
        let mut policy = basic_policy();
        policy.on_status_codes = vec![StatusCodeConfig {
            codes: vec![StatusCodeEntry::Single(600)],
            strategy: RetryStrategy::SameModel,
            max_attempts: 2,
        }];
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert!(errs.iter().any(|e| matches!(e, ValidationError::StatusCodeOutOfRange { code: 600, .. })));
    }

    #[test]
    fn test_status_code_range_inverted() {
        let mut policy = basic_policy();
        policy.on_status_codes = vec![StatusCodeConfig {
            codes: vec![StatusCodeEntry::Range("504-502".to_string())],
            strategy: RetryStrategy::SameModel,
            max_attempts: 2,
        }];
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert!(errs.iter().any(|e| matches!(e, ValidationError::StatusCodeRangeInverted { .. })));
    }

    #[test]
    fn test_backoff_max_ms_not_greater_than_base_ms() {
        let mut policy = basic_policy();
        policy.backoff = Some(BackoffConfig {
            apply_to: BackoffApplyTo::SameModel,
            base_ms: 5000,
            max_ms: 5000,
            jitter: true,
        });
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert!(errs.iter().any(|e| matches!(e, ValidationError::MaxMsNotGreaterThanBaseMs { .. })));
    }

    #[test]
    fn test_backoff_zero_base_ms() {
        let mut policy = basic_policy();
        policy.backoff = Some(BackoffConfig {
            apply_to: BackoffApplyTo::SameModel,
            base_ms: 0,
            max_ms: 5000,
            jitter: true,
        });
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert!(errs.iter().any(|e| matches!(e, ValidationError::NonPositiveValue { field, .. } if field == "backoff.base_ms")));
    }

    #[test]
    fn test_max_retry_duration_ms_zero() {
        let mut policy = basic_policy();
        policy.max_retry_duration_ms = Some(0);
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert!(errs.iter().any(|e| matches!(e, ValidationError::NonPositiveValue { field, .. } if field == "max_retry_duration_ms")));
    }

    #[test]
    fn test_single_provider_failover_warning() {
        let policy = basic_policy(); // default_strategy is DifferentProvider
        let providers = vec![make_provider("openai/gpt-4o", Some(policy))];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_ok());
        let warnings = result.unwrap();
        assert!(warnings.iter().any(|w| matches!(w, ValidationWarning::SingleProviderWithFailover { .. })));
    }

    #[test]
    fn test_overlapping_status_codes_warning() {
        let mut policy = basic_policy();
        policy.on_status_codes = vec![
            StatusCodeConfig {
                codes: vec![StatusCodeEntry::Single(429)],
                strategy: RetryStrategy::SameModel,
                max_attempts: 2,
            },
            StatusCodeConfig {
                codes: vec![StatusCodeEntry::Single(429)],
                strategy: RetryStrategy::DifferentProvider,
                max_attempts: 3,
            },
        ];
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_ok());
        let warnings = result.unwrap();
        assert!(warnings.iter().any(|w| matches!(w, ValidationWarning::OverlappingStatusCodes { code: 429, .. })));
    }

    #[test]
    fn test_backoff_apply_to_mismatch_warning() {
        let mut policy = basic_policy();
        policy.default_strategy = RetryStrategy::DifferentProvider;
        policy.backoff = Some(BackoffConfig {
            apply_to: BackoffApplyTo::SameModel,
            base_ms: 100,
            max_ms: 5000,
            jitter: true,
        });
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_ok());
        let warnings = result.unwrap();
        assert!(warnings.iter().any(|w| matches!(w, ValidationWarning::BackoffApplyToMismatch { .. })));
    }

    #[test]
    fn test_fallback_model_not_in_provider_list_warning() {
        let mut policy = basic_policy();
        policy.fallback_models = vec!["nonexistent/model".to_string()];
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_ok());
        let warnings = result.unwrap();
        assert!(warnings.iter().any(|w| matches!(w, ValidationWarning::FallbackModelNotInProviderList { fallback, .. } if fallback == "nonexistent/model")));
    }

    #[test]
    fn test_expand_status_codes_mixed() {
        let codes = vec![
            StatusCodeEntry::Single(429),
            StatusCodeEntry::Range("502-504".to_string()),
            StatusCodeEntry::Single(526),
        ];
        let result = ConfigValidator::expand_status_codes(&codes);
        assert!(result.is_ok());
        let expanded = result.unwrap();
        assert_eq!(expanded, vec![429, 502, 503, 504, 526]);
    }

    #[test]
    fn test_valid_range_expansion() {
        let codes = vec![StatusCodeEntry::Range("500-503".to_string())];
        let result = ConfigValidator::expand_status_codes(&codes);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), vec![500, 501, 502, 503]);
    }

    #[test]
    fn test_valid_policy_with_backoff_and_status_codes() {
        let mut policy = basic_policy();
        policy.default_strategy = RetryStrategy::SameModel;
        policy.on_status_codes = vec![
            StatusCodeConfig {
                codes: vec![StatusCodeEntry::Single(429), StatusCodeEntry::Range("502-504".to_string())],
                strategy: RetryStrategy::SameModel,
                max_attempts: 3,
            },
        ];
        policy.backoff = Some(BackoffConfig {
            apply_to: BackoffApplyTo::SameModel,
            base_ms: 100,
            max_ms: 5000,
            jitter: true,
        });
        policy.max_retry_duration_ms = Some(30000);
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    // ── P1 Validation Tests ───────────────────────────────────────────────

    #[test]
    fn test_on_timeout_zero_max_attempts_rejected() {
        let mut policy = basic_policy();
        policy.on_timeout = Some(TimeoutRetryConfig {
            strategy: RetryStrategy::DifferentProvider,
            max_attempts: 0,
        });
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert!(errs.iter().any(|e| matches!(
            e,
            ValidationError::NonPositiveValue { field, .. } if field == "on_timeout.max_attempts"
        )));
    }

    #[test]
    fn test_on_timeout_valid_max_attempts_accepted() {
        let mut policy = basic_policy();
        policy.on_timeout = Some(TimeoutRetryConfig {
            strategy: RetryStrategy::DifferentProvider,
            max_attempts: 2,
        });
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_ok());
    }

    #[test]
    fn test_retry_after_handling_zero_max_seconds_rejected() {
        let mut policy = basic_policy();
        policy.retry_after_handling = Some(RetryAfterHandlingConfig {
            scope: BlockScope::Model,
            apply_to: crate::configuration::ApplyTo::Global,
            max_retry_after_seconds: 0,
        });
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert!(errs.iter().any(|e| matches!(
            e,
            ValidationError::NonPositiveValue { field, .. }
                if field == "retry_after_handling.max_retry_after_seconds"
        )));
    }

    #[test]
    fn test_retry_after_handling_valid_max_seconds_accepted() {
        let mut policy = basic_policy();
        policy.retry_after_handling = Some(RetryAfterHandlingConfig {
            scope: BlockScope::Model,
            apply_to: crate::configuration::ApplyTo::Global,
            max_retry_after_seconds: 300,
        });
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_ok());
    }

    #[test]
    fn test_fallback_model_empty_string_rejected() {
        let mut policy = basic_policy();
        policy.fallback_models = vec!["".to_string()];
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert!(errs.iter().any(|e| matches!(
            e,
            ValidationError::InvalidFallbackModel { fallback, .. } if fallback.is_empty()
        )));
    }

    #[test]
    fn test_fallback_model_no_slash_rejected() {
        let mut policy = basic_policy();
        policy.fallback_models = vec!["just-a-model-name".to_string()];
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert!(errs.iter().any(|e| matches!(
            e,
            ValidationError::InvalidFallbackModel { fallback, .. } if fallback == "just-a-model-name"
        )));
    }

    #[test]
    fn test_fallback_model_valid_format_accepted() {
        let mut policy = basic_policy();
        policy.fallback_models = vec!["anthropic/claude-3".to_string()];
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_ok());
    }

    #[test]
    fn test_provider_scope_ra_with_same_model_strategy_warning() {
        let mut policy = basic_policy();
        policy.default_strategy = RetryStrategy::SameModel;
        policy.retry_after_handling = Some(RetryAfterHandlingConfig {
            scope: BlockScope::Provider,
            apply_to: crate::configuration::ApplyTo::Global,
            max_retry_after_seconds: 300,
        });
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_ok());
        let warnings = result.unwrap();
        assert!(warnings.iter().any(|w| matches!(
            w,
            ValidationWarning::ProviderScopeWithSameModel { .. }
        )));
    }

    #[test]
    fn test_model_scope_ra_with_same_model_no_warning() {
        let mut policy = basic_policy();
        policy.default_strategy = RetryStrategy::SameModel;
        policy.retry_after_handling = Some(RetryAfterHandlingConfig {
            scope: BlockScope::Model,
            apply_to: crate::configuration::ApplyTo::Global,
            max_retry_after_seconds: 300,
        });
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_ok());
        let warnings = result.unwrap();
        assert!(!warnings.iter().any(|w| matches!(
            w,
            ValidationWarning::ProviderScopeWithSameModel { .. }
        )));
    }

    // ── P2 Validation Tests ───────────────────────────────────────────────

    fn hl_config_valid() -> HighLatencyConfig {
        HighLatencyConfig {
            threshold_ms: 5000,
            measure: LatencyMeasure::Ttfb,
            min_triggers: 1,
            trigger_window_seconds: None,
            strategy: RetryStrategy::DifferentProvider,
            max_attempts: 2,
            block_duration_seconds: 300,
            scope: BlockScope::Model,
            apply_to: ApplyTo::Global,
        }
    }

    #[test]
    fn test_on_high_latency_valid_config_accepted() {
        let mut policy = basic_policy();
        policy.on_high_latency = Some(hl_config_valid());
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_ok());
    }

    #[test]
    fn test_on_high_latency_zero_threshold_ms_rejected() {
        let mut policy = basic_policy();
        let mut hl = hl_config_valid();
        hl.threshold_ms = 0;
        policy.on_high_latency = Some(hl);
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert!(errs.iter().any(|e| matches!(
            e,
            ValidationError::NonPositiveValue { field, .. }
                if field == "on_high_latency.threshold_ms"
        )));
    }

    #[test]
    fn test_on_high_latency_zero_max_attempts_rejected() {
        let mut policy = basic_policy();
        let mut hl = hl_config_valid();
        hl.max_attempts = 0;
        policy.on_high_latency = Some(hl);
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert!(errs.iter().any(|e| matches!(
            e,
            ValidationError::NonPositiveValue { field, .. }
                if field == "on_high_latency.max_attempts"
        )));
    }

    #[test]
    fn test_on_high_latency_zero_block_duration_rejected() {
        let mut policy = basic_policy();
        let mut hl = hl_config_valid();
        hl.block_duration_seconds = 0;
        policy.on_high_latency = Some(hl);
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert!(errs.iter().any(|e| matches!(
            e,
            ValidationError::NonPositiveValue { field, .. }
                if field == "on_high_latency.block_duration_seconds"
        )));
    }

    #[test]
    fn test_on_high_latency_min_triggers_gt1_without_window_rejected() {
        let mut policy = basic_policy();
        let mut hl = hl_config_valid();
        hl.min_triggers = 3;
        hl.trigger_window_seconds = None;
        policy.on_high_latency = Some(hl);
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert!(errs.iter().any(|e| matches!(
            e,
            ValidationError::LatencyMissingTriggerWindow { .. }
        )));
    }

    #[test]
    fn test_on_high_latency_min_triggers_gt1_with_window_accepted() {
        let mut policy = basic_policy();
        let mut hl = hl_config_valid();
        hl.min_triggers = 3;
        hl.trigger_window_seconds = Some(60);
        policy.on_high_latency = Some(hl);
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_ok());
    }

    #[test]
    fn test_on_high_latency_zero_trigger_window_rejected() {
        let mut policy = basic_policy();
        let mut hl = hl_config_valid();
        hl.trigger_window_seconds = Some(0);
        policy.on_high_latency = Some(hl);
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_err());
        let errs = result.unwrap_err();
        assert!(errs.iter().any(|e| matches!(
            e,
            ValidationError::NonPositiveTriggerWindow { .. }
        )));
    }

    #[test]
    fn test_on_high_latency_provider_scope_same_model_warning() {
        let mut policy = basic_policy();
        let mut hl = hl_config_valid();
        hl.scope = BlockScope::Provider;
        hl.strategy = RetryStrategy::SameModel;
        policy.on_high_latency = Some(hl);
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_ok());
        let warnings = result.unwrap();
        assert!(warnings.iter().any(|w| matches!(
            w,
            ValidationWarning::LatencyScopeStrategyMismatch { .. }
        )));
    }

    #[test]
    fn test_on_high_latency_model_scope_same_model_no_warning() {
        let mut policy = basic_policy();
        let mut hl = hl_config_valid();
        hl.scope = BlockScope::Model;
        hl.strategy = RetryStrategy::SameModel;
        policy.on_high_latency = Some(hl);
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_ok());
        let warnings = result.unwrap();
        assert!(!warnings.iter().any(|w| matches!(
            w,
            ValidationWarning::LatencyScopeStrategyMismatch { .. }
        )));
    }

    #[test]
    fn test_on_high_latency_threshold_below_1000_warning() {
        let mut policy = basic_policy();
        let mut hl = hl_config_valid();
        hl.threshold_ms = 500;
        policy.on_high_latency = Some(hl);
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_ok());
        let warnings = result.unwrap();
        assert!(warnings.iter().any(|w| matches!(
            w,
            ValidationWarning::AggressiveLatencyThreshold { threshold_ms: 500, .. }
        )));
    }

    #[test]
    fn test_on_high_latency_threshold_1000_no_warning() {
        let mut policy = basic_policy();
        let mut hl = hl_config_valid();
        hl.threshold_ms = 1000;
        policy.on_high_latency = Some(hl);
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_ok());
        let warnings = result.unwrap();
        assert!(!warnings.iter().any(|w| matches!(
            w,
            ValidationWarning::AggressiveLatencyThreshold { .. }
        )));
    }

    #[test]
    fn test_on_high_latency_total_measure_accepted() {
        let mut policy = basic_policy();
        let mut hl = hl_config_valid();
        hl.measure = LatencyMeasure::Total;
        policy.on_high_latency = Some(hl);
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_ok());
    }

    #[test]
    fn test_on_high_latency_request_apply_to_accepted() {
        let mut policy = basic_policy();
        let mut hl = hl_config_valid();
        hl.apply_to = ApplyTo::Request;
        policy.on_high_latency = Some(hl);
        let providers = vec![
            make_provider("openai/gpt-4o", Some(policy)),
            make_provider("anthropic/claude-3", None),
        ];
        let result = ConfigValidator::validate_retry_policies(&providers);
        assert!(result.is_ok());
    }

    // ── Strategies for invalid config generation ───────────────────────────

    /// Generates a status code outside the valid 100-599 range.
    fn arb_out_of_range_code() -> impl Strategy<Value = u16> {
        prop_oneof![
            (0u16..100u16),       // below 100
            (600u16..=u16::MAX),  // above 599
        ]
    }

    /// Generates a range string where start > end (both within valid range).
    fn arb_inverted_range() -> impl Strategy<Value = String> {
        (101u16..=599u16).prop_flat_map(|start| {
            (100u16..start).prop_map(move |end| format!("{}-{}", start, end))
        })
    }

    /// Generates a backoff config where max_ms <= base_ms.
    fn arb_backoff_max_lte_base() -> impl Strategy<Value = BackoffConfig> {
        (1u64..=10000u64).prop_flat_map(|base_ms| {
            (0u64..=base_ms).prop_map(move |max_ms| BackoffConfig {
                apply_to: BackoffApplyTo::Global,
                base_ms,
                max_ms,
                jitter: true,
            })
        })
    }

    /// Generates a backoff config where base_ms = 0.
    fn arb_backoff_zero_base() -> impl Strategy<Value = BackoffConfig> {
        (1u64..=10000u64).prop_map(|max_ms| BackoffConfig {
            apply_to: BackoffApplyTo::Global,
            base_ms: 0,
            max_ms,
            jitter: true,
        })
    }

    // Feature: retry-on-ratelimit, Property 3: Invalid Configuration Rejected
    // **Validates: Requirements 8.27**
    proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(100))]

        /// Property 3 – Case 1: Status codes outside 100-599 are rejected.
        #[test]
        fn prop_invalid_status_code_out_of_range(code in arb_out_of_range_code()) {
            let mut policy = basic_policy();
            policy.on_status_codes = vec![StatusCodeConfig {
                codes: vec![StatusCodeEntry::Single(code)],
                strategy: RetryStrategy::SameModel,
                max_attempts: 2,
            }];
            let providers = vec![
                make_provider("openai/gpt-4o", Some(policy)),
                make_provider("anthropic/claude-3", None),
            ];
            let result = ConfigValidator::validate_retry_policies(&providers);
            prop_assert!(result.is_err(), "Expected Err for out-of-range code {}", code);
        }

        /// Property 3 – Case 2: Range strings with start > end are rejected.
        #[test]
        fn prop_invalid_range_start_gt_end(range in arb_inverted_range()) {
            let mut policy = basic_policy();
            policy.on_status_codes = vec![StatusCodeConfig {
                codes: vec![StatusCodeEntry::Range(range.clone())],
                strategy: RetryStrategy::SameModel,
                max_attempts: 2,
            }];
            let providers = vec![
                make_provider("openai/gpt-4o", Some(policy)),
                make_provider("anthropic/claude-3", None),
            ];
            let result = ConfigValidator::validate_retry_policies(&providers);
            prop_assert!(result.is_err(), "Expected Err for inverted range {}", range);
        }

        /// Property 3 – Case 3: Backoff with max_ms <= base_ms is rejected.
        #[test]
        fn prop_invalid_backoff_max_lte_base(backoff in arb_backoff_max_lte_base()) {
            let mut policy = basic_policy();
            policy.backoff = Some(backoff.clone());
            let providers = vec![
                make_provider("openai/gpt-4o", Some(policy)),
                make_provider("anthropic/claude-3", None),
            ];
            let result = ConfigValidator::validate_retry_policies(&providers);
            prop_assert!(
                result.is_err(),
                "Expected Err for max_ms ({}) <= base_ms ({})",
                backoff.max_ms, backoff.base_ms
            );
        }

        /// Property 3 – Case 4: Backoff with base_ms = 0 is rejected.
        #[test]
        fn prop_invalid_backoff_zero_base(backoff in arb_backoff_zero_base()) {
            let mut policy = basic_policy();
            policy.backoff = Some(backoff);
            let providers = vec![
                make_provider("openai/gpt-4o", Some(policy)),
                make_provider("anthropic/claude-3", None),
            ];
            let result = ConfigValidator::validate_retry_policies(&providers);
            prop_assert!(result.is_err(), "Expected Err for base_ms = 0");
        }

        /// Property 3 – Case 5: max_retry_duration_ms = 0 is rejected.
        #[test]
        fn prop_invalid_max_retry_duration_zero(_dummy in Just(())) {
            let mut policy = basic_policy();
            policy.max_retry_duration_ms = Some(0);
            let providers = vec![
                make_provider("openai/gpt-4o", Some(policy)),
                make_provider("anthropic/claude-3", None),
            ];
            let result = ConfigValidator::validate_retry_policies(&providers);
            prop_assert!(result.is_err(), "Expected Err for max_retry_duration_ms = 0");
        }
    }
}
