use crate::configuration::LlmProvider;
use hermesllm::providers::ProviderId;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug)]
pub struct LlmProviders {
    providers: HashMap<String, Arc<LlmProvider>>,
    default: Option<Arc<LlmProvider>>,
    /// Wildcard providers: maps provider prefix to base provider config
    /// e.g., "openai" -> LlmProvider for "openai/*"
    wildcard_providers: HashMap<String, Arc<LlmProvider>>,
}

impl LlmProviders {
    pub fn iter(&self) -> std::collections::hash_map::Iter<'_, String, Arc<LlmProvider>> {
        self.providers.iter()
    }

    pub fn default(&self) -> Option<Arc<LlmProvider>> {
        self.default.clone()
    }
    /// Convert providers to OpenAI Models format for /v1/models endpoint
    /// Filters out internal models and duplicate entries (backward compatibility aliases)
    pub fn to_models(&self) -> hermesllm::apis::openai::Models {
        use hermesllm::apis::openai::{ModelDetail, ModelObject, Models};

        let data: Vec<ModelDetail> = self
            .providers
            .iter()
            .filter(|(key, provider)| {
                // Exclude internal models
                provider.internal != Some(true)
                // Only include canonical entries (key matches provider name)
                // This avoids duplicates from backward compatibility short names
                && *key == &provider.name
            })
            .map(|(name, provider)| ModelDetail {
                id: name.clone(),
                object: Some("model".to_string()),
                created: 0,
                owned_by: provider.to_provider_id().to_string(),
            })
            .collect();

        Models {
            object: ModelObject::List,
            data,
        }
    }
    pub fn get(&self, name: &str) -> Option<Arc<LlmProvider>> {
        // First try exact match
        if let Some(provider) = self.providers.get(name).cloned() {
            return Some(provider);
        }

        // If name contains '/', it could be:
        // 1. A full model ID like "openai/gpt-4" that we need to lookup
        // 2. A provider/model slug that should match a wildcard provider
        if let Some((provider_prefix, model_name)) = name.split_once('/') {
            // Try to find the expanded model entry (e.g., "openai/gpt-4")
            let full_model_id = format!("{}/{}", provider_prefix, model_name);
            if let Some(provider) = self.providers.get(&full_model_id).cloned() {
                return Some(provider);
            }

            // Try to find just the model name (for expanded wildcard entries)
            if let Some(provider) = self.providers.get(model_name).cloned() {
                return Some(provider);
            }

            // Fall back to wildcard match (e.g., "openai/*")
            if let Some(wildcard_provider) = self.wildcard_providers.get(provider_prefix) {
                // Create a new provider with the specific model from the slug
                let mut specific_provider = (**wildcard_provider).clone();
                specific_provider.model = Some(model_name.to_string());
                return Some(Arc::new(specific_provider));
            }
        }

        None
    }
}

#[derive(thiserror::Error, Debug)]
pub enum LlmProvidersNewError {
    #[error("There must be at least one LLM Provider")]
    EmptySource,
    #[error("There must be at most one default LLM Provider")]
    MoreThanOneDefault,
    #[error("\'{0}\' is not a unique name")]
    DuplicateName(String),
}

impl TryFrom<Vec<LlmProvider>> for LlmProviders {
    type Error = LlmProvidersNewError;

    fn try_from(llm_providers_config: Vec<LlmProvider>) -> Result<Self, Self::Error> {
        if llm_providers_config.is_empty() {
            return Err(LlmProvidersNewError::EmptySource);
        }

        let mut llm_providers = LlmProviders {
            providers: HashMap::new(),
            default: None,
            wildcard_providers: HashMap::new(),
        };

        // Track specific (non-wildcard) provider names to detect true duplicates
        let mut specific_provider_names = std::collections::HashSet::new();

        // Track specific models that should be excluded from wildcard expansion
        // Maps provider_prefix -> Set of model names (e.g., "anthropic" -> {"claude-sonnet-4-20250514"})
        let mut specific_models_by_provider: HashMap<String, std::collections::HashSet<String>> =
            HashMap::new();

        // First pass: collect all specific model configurations
        for llm_provider in &llm_providers_config {
            let is_wildcard = llm_provider
                .model
                .as_ref()
                .map(|m| m == "*" || m.ends_with("/*"))
                .unwrap_or(false);

            if !is_wildcard {
                // Check if this is a provider/model format
                if let Some((provider_prefix, model_name)) = llm_provider.name.split_once('/') {
                    specific_models_by_provider
                        .entry(provider_prefix.to_string())
                        .or_default()
                        .insert(model_name.to_string());
                }
            }
        }

        for llm_provider in llm_providers_config {
            let llm_provider: Arc<LlmProvider> = Arc::new(llm_provider);

            if llm_provider.default.unwrap_or_default() {
                match llm_providers.default {
                    Some(_) => return Err(LlmProvidersNewError::MoreThanOneDefault),
                    None => llm_providers.default = Some(Arc::clone(&llm_provider)),
                }
            }

            let name = llm_provider.name.clone();

            // Check if this is a wildcard provider (model is "*" or ends with "/*")
            let is_wildcard = llm_provider
                .model
                .as_ref()
                .map(|m| m == "*" || m.ends_with("/*"))
                .unwrap_or(false);

            if is_wildcard {
                // Extract provider prefix from name
                // e.g., "openai/*" -> "openai"
                let provider_prefix = name.trim_end_matches("/*").trim_end_matches('*');

                // For wildcard providers, we:
                // 1. Store the base config in wildcard_providers for runtime matching
                // 2. Optionally expand to all known models if available

                llm_providers
                    .wildcard_providers
                    .insert(provider_prefix.to_string(), Arc::clone(&llm_provider));

                // Try to expand wildcard using ProviderId models
                if let Ok(provider_id) = ProviderId::try_from(provider_prefix) {
                    let models = provider_id.models();

                    // Get the set of specific models to exclude for this provider
                    let models_to_exclude = specific_models_by_provider
                        .get(provider_prefix)
                        .cloned()
                        .unwrap_or_default();

                    if !models.is_empty() {
                        let excluded_count = models_to_exclude.len();
                        let total_models = models.len();

                        log::info!(
                            "Expanding wildcard provider '{}' to {} models{}",
                            provider_prefix,
                            total_models - excluded_count,
                            if excluded_count > 0 {
                                format!(" (excluding {} specifically configured)", excluded_count)
                            } else {
                                String::new()
                            }
                        );

                        // Create a provider entry for each model (except those specifically configured)
                        for model_name in models {
                            // Skip this model if it has a specific configuration
                            if models_to_exclude.contains(&model_name) {
                                log::debug!(
                                    "Skipping wildcard expansion for '{}/{}' - specific configuration exists",
                                    provider_prefix,
                                    model_name
                                );
                                continue;
                            }

                            let full_model_id = format!("{}/{}", provider_prefix, model_name);

                            // Create a new provider with the specific model
                            let mut expanded_provider = (*llm_provider).clone();
                            expanded_provider.model = Some(model_name.clone());
                            expanded_provider.name = full_model_id.clone();

                            let expanded_rc = Arc::new(expanded_provider);

                            // Insert with full model ID as key
                            llm_providers
                                .providers
                                .insert(full_model_id.clone(), Arc::clone(&expanded_rc));

                            // Also insert with just model name for backward compatibility
                            llm_providers.providers.insert(model_name, expanded_rc);
                        }
                    }
                } else {
                    log::warn!(
                        "Wildcard provider '{}' specified but no models found in registry. \
                         Will match dynamically at runtime.",
                        provider_prefix
                    );
                }
            } else {
                // Non-wildcard provider - specific configuration
                // Check for duplicate specific entries (not allowed)
                if specific_provider_names.contains(&name) {
                    return Err(LlmProvidersNewError::DuplicateName(name));
                }
                specific_provider_names.insert(name.clone());

                // This specific configuration takes precedence over any wildcard expansion
                // The wildcard expansion already excluded this model (see first pass above)

                log::debug!("Processing specific provider configuration: {}", name);

                // Insert with the provider name as key
                llm_providers
                    .providers
                    .insert(name.clone(), Arc::clone(&llm_provider));

                // Also add model_id as key for provider lookup
                if let Some(model) = llm_provider.model.clone() {
                    llm_providers.providers.insert(model, llm_provider);
                }
            }
        }

        Ok(llm_providers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::LlmProviderType;

    fn create_test_provider(name: &str, model: Option<String>) -> LlmProvider {
        LlmProvider {
            name: name.to_string(),
            model,
            access_key: None,
            endpoint: None,
            cluster_name: None,
            provider_interface: LlmProviderType::OpenAI,
            default: None,
            base_url_path_prefix: None,
            port: None,
            rate_limits: None,
            usage: None,
            routing_preferences: None,
            internal: None,
            stream: None,
            passthrough_auth: None,
        }
    }

    #[test]
    fn test_static_provider_lookup() {
        // Test 1: Statically defined provider - should be findable by model or provider name
        let providers = vec![create_test_provider("my-openai", Some("gpt-4".to_string()))];
        let llm_providers = LlmProviders::try_from(providers).unwrap();

        // Should find by model name
        let result = llm_providers.get("gpt-4");
        assert!(result.is_some());
        assert_eq!(result.unwrap().name, "my-openai");

        // Should also find by provider name
        let result = llm_providers.get("my-openai");
        assert!(result.is_some());
        assert_eq!(result.unwrap().name, "my-openai");
    }

    #[test]
    fn test_wildcard_provider_with_known_model() {
        // Test 2: Wildcard provider that expands to OpenAI models
        let providers = vec![create_test_provider("openai/*", Some("*".to_string()))];
        let llm_providers = LlmProviders::try_from(providers).unwrap();

        // Should find via expanded wildcard entry
        let result = llm_providers.get("openai/gpt-4");
        let provider = result.unwrap();
        assert_eq!(provider.name, "openai/gpt-4");
        assert_eq!(provider.model.as_ref().unwrap(), "gpt-4");

        // Should also be able to find by just model name (from expansion)
        let result = llm_providers.get("gpt-4");
        assert_eq!(result.unwrap().model.as_ref().unwrap(), "gpt-4");
    }

    #[test]
    fn test_custom_wildcard_provider_with_full_slug() {
        // Test 3: Custom wildcard provider with full slug offered
        let providers = vec![create_test_provider(
            "custom-provider/*",
            Some("*".to_string()),
        )];
        let llm_providers = LlmProviders::try_from(providers).unwrap();

        // Should match via wildcard fallback and extract model name from slug
        let result = llm_providers.get("custom-provider/custom-model");
        let provider = result.unwrap();
        assert_eq!(provider.model.as_ref().unwrap(), "custom-model");

        // Wildcard should be stored
        assert!(llm_providers
            .wildcard_providers
            .contains_key("custom-provider"));
    }
}
