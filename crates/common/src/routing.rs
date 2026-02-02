use std::sync::Arc;

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
) -> Result<Arc<LlmProvider>, String> {
    match provider_hint {
        Some(ProviderHint::Default) => llm_providers
            .default()
            .ok_or_else(|| "No default provider configured".to_string()),
        Some(ProviderHint::Name(name)) => llm_providers
            .get(&name)
            .ok_or_else(|| format!("Model '{}' not found in configured providers", name)),
        None => Err("No model specified in request".to_string()),
    }
}
