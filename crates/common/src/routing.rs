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
        .filter(|(_, provider)| {
            provider
                .model
                .as_ref()
                .map(|m| !m.starts_with("Arch"))
                .unwrap_or(true)
        })
        .choose(&mut rng)
        .expect("There should always be at least one non-Arch llm provider")
        .1
        .clone()
}
