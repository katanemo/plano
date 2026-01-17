use std::rc::Rc;

use crate::{configuration, llm_providers::LlmProviders};
use configuration::LlmProvider;

#[derive(Debug, Clone)]
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
) -> Result<Rc<LlmProvider>, String> {
    match provider_hint {
        Some(ProviderHint::Default) => llm_providers
            .default()
            .ok_or_else(|| "No default provider configured".to_string()),
        Some(ProviderHint::Name(name)) => llm_providers
            .get(&name)
            .ok_or_else(|| format!("Model '{}' not found in configured providers", name)),
        None => {
            // No hint provided - must have a default configured
            llm_providers
                .default()
                .ok_or_else(|| "No model specified and no default provider configured".to_string())
        }
    }
}
