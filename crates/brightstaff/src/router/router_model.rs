use hermesllm::apis::openai::{ChatCompletionsRequest, Message};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RoutingModelError {
    #[error("Failed to parse JSON: {0}")]
    JsonError(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, RoutingModelError>;

/// Internal route descriptor passed to the router model to build its prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingPreference {
    pub name: String,
    pub description: String,
}

/// Groups a model with its routing preferences (used internally by RouterModelV1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelUsagePreference {
    pub model: String,
    pub routing_preferences: Vec<RoutingPreference>,
}

pub trait RouterModel: Send + Sync {
    fn generate_request(
        &self,
        messages: &[Message],
        usage_preferences: &Option<Vec<ModelUsagePreference>>,
    ) -> ChatCompletionsRequest;
    fn parse_response(
        &self,
        content: &str,
        usage_preferences: &Option<Vec<ModelUsagePreference>>,
    ) -> Result<Option<(String, String)>>;
    fn get_model_name(&self) -> String;
}
