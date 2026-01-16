use std::rc::Rc;

use crate::{configuration, llm_providers::LlmProviders};
use configuration::LlmProvider;
use rand::{seq::IteratorRandom, thread_rng};

#[derive(Debug)]
pub enum ProviderHint {
    Default,
    Name(String),
}

impl From<String> for ProviderHint {
    fn from(value: String) -> Self {
        match value.as_str() {
            "default" => ProviderHint::Default,
            _ => ProviderHint::Name(value),
        }
    }
}

pub fn get_llm_provider(
    llm_providers: &LlmProviders,
    provider_hint: Option<ProviderHint>,
) -> Rc<LlmProvider> {
    let maybe_provider = provider_hint.and_then(|hint| match hint {
        ProviderHint::Default => llm_providers.default(),
        // FIXME: should a non-existent name in the hint be more explicit? i.e, return a BAD_REQUEST?
        ProviderHint::Name(name) => llm_providers.get(&name),
    });

    if let Some(provider) = maybe_provider {
        return provider;
    }

    if llm_providers.default().is_some() {
        return llm_providers.default().unwrap();
    }

    let mut rng = thread_rng();
    llm_providers
        .iter()
        .filter(|(_, provider)| provider.internal != Some(true))
        .choose(&mut rng)
        .expect("There should always be at least one non-internal llm provider")
        .1
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::configuration::LlmProviderType;

    #[test]
    fn test_get_llm_provider_excludes_internal_providers() {
        let providers = vec![
            LlmProvider {
                name: "openai-gpt4".to_string(),
                provider_interface: LlmProviderType::OpenAI,
                model: Some("gpt-4".to_string()),
                internal: None,
                default: None,
                ..Default::default()
            },
            LlmProvider {
                name: "anthropic-claude".to_string(),
                provider_interface: LlmProviderType::Anthropic,
                model: Some("claude-3".to_string()),
                internal: Some(false),
                default: None,
                ..Default::default()
            },
            LlmProvider {
                name: "arch-router".to_string(),
                provider_interface: LlmProviderType::Arch,
                model: Some("Arch-Router".to_string()),
                internal: Some(true),
                default: None,
                ..Default::default()
            },
            LlmProvider {
                name: "plano-orchestrator".to_string(),
                provider_interface: LlmProviderType::Arch,
                model: Some("Plano-Orchestrator".to_string()),
                internal: Some(true),
                default: None,
                ..Default::default()
            },
        ];

        let llm_providers = LlmProviders::try_from(providers).unwrap();

        // Test multiple times to account for randomness
        for _ in 0..10 {
            let selected = get_llm_provider(&llm_providers, None);

            // Verify the selected provider is never internal
            assert_ne!(selected.internal, Some(true));

            // Verify it's one of the non-internal providers
            assert!(
                selected.name == "openai-gpt4" || selected.name == "anthropic-claude",
                "Selected provider '{}' should be one of the non-internal providers",
                selected.name
            );
        }
    }
}
