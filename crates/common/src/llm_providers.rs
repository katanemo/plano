use crate::configuration::LlmProvider;
use hermesllm::providers::ProviderId;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Debug)]
pub struct LlmProviders {
    providers: HashMap<String, Rc<LlmProvider>>,
    default: Option<Rc<LlmProvider>>,
    /// Wildcard providers: maps provider prefix to base provider config
    /// e.g., "openai" -> LlmProvider for "openai/*"
    wildcard_providers: HashMap<String, Rc<LlmProvider>>,
}

impl LlmProviders {
    pub fn iter(&self) -> std::collections::hash_map::Iter<'_, String, Rc<LlmProvider>> {
        self.providers.iter()
    }

    pub fn default(&self) -> Option<Rc<LlmProvider>> {
        self.default.clone()
    }

    pub fn get(&self, name: &str) -> Option<Rc<LlmProvider>> {
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
                return Some(Rc::new(specific_provider));
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

        for llm_provider in llm_providers_config {
            let llm_provider: Rc<LlmProvider> = Rc::new(llm_provider);

            if llm_provider.default.unwrap_or_default() {
                match llm_providers.default {
                    Some(_) => return Err(LlmProvidersNewError::MoreThanOneDefault),
                    None => llm_providers.default = Some(Rc::clone(&llm_provider)),
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
                    .insert(provider_prefix.to_string(), Rc::clone(&llm_provider));

                // Try to expand wildcard using ProviderId models
                if let Ok(provider_id) = ProviderId::try_from(provider_prefix) {
                    let models = provider_id.models();
                    if !models.is_empty() {
                        log::info!(
                            "Expanding wildcard provider '{}' to {} models",
                            provider_prefix,
                            models.len()
                        );

                        // Create a provider entry for each model
                        for model_name in models {
                            let full_model_id = format!("{}/{}", provider_prefix, model_name);

                            // Create a new provider with the specific model
                            let mut expanded_provider = (*llm_provider).clone();
                            expanded_provider.model = Some(model_name.clone());
                            expanded_provider.name = full_model_id.clone();

                            let expanded_rc = Rc::new(expanded_provider);

                            // Insert with full model ID as key
                            llm_providers
                                .providers
                                .insert(full_model_id.clone(), Rc::clone(&expanded_rc));

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
                // Non-wildcard provider - original behavior
                if llm_providers
                    .providers
                    .insert(name.clone(), Rc::clone(&llm_provider))
                    .is_some()
                {
                    return Err(LlmProvidersNewError::DuplicateName(name));
                }

                // also add model_id as key for provider lookup
                if let Some(model) = llm_provider.model.clone() {
                    if llm_providers
                        .providers
                        .insert(model, llm_provider)
                        .is_some()
                    {
                        return Err(LlmProvidersNewError::DuplicateName(name));
                    }
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
