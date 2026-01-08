use crate::configuration::LlmProvider;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Debug)]
pub struct LlmProviders {
    providers: HashMap<String, Rc<LlmProvider>>,
    default: Option<Rc<LlmProvider>>,
}

impl LlmProviders {
    pub fn iter(&self) -> std::collections::hash_map::Iter<'_, String, Rc<LlmProvider>> {
        self.providers.iter()
    }

    pub fn default(&self) -> Option<Rc<LlmProvider>> {
        self.default.clone()
    }

    pub fn get(&self, name: &str) -> Option<Rc<LlmProvider>> {
        self.providers.get(name).cloned()
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
        };

        for llm_provider in llm_providers_config {
            let llm_provider: Rc<LlmProvider> = Rc::new(llm_provider);
            if llm_provider.default.unwrap_or_default() {
                match llm_providers.default {
                    Some(_) => return Err(LlmProvidersNewError::MoreThanOneDefault),
                    None => llm_providers.default = Some(Rc::clone(&llm_provider)),
                }
            }

            // Insert and check that there is no other provider with the same name.
            let name = llm_provider.name.clone();
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
        Ok(llm_providers)
    }
}
