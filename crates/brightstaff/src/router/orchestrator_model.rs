use common::configuration::AgentUsagePreference;
use hermesllm::apis::openai::{ChatCompletionsRequest, Message};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum OrchestratorModelError {
    #[error("Failed to parse JSON: {0}")]
    JsonError(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, OrchestratorModelError>;

/// OrchestratorModel trait for handling orchestration requests.
/// Returns multiple routes as the model output format is:
/// {"route": ["route_name_1", "route_name_2", ...]}
pub trait OrchestratorModel: Send + Sync {
    fn generate_request(
        &self,
        messages: &[Message],
        usage_preferences: &Option<Vec<AgentUsagePreference>>,
    ) -> ChatCompletionsRequest;
    /// Returns a vector of (route_name, model_name) tuples for all matched routes.
    fn parse_response(
        &self,
        content: &str,
        usage_preferences: &Option<Vec<AgentUsagePreference>>,
    ) -> Result<Option<Vec<(String, String)>>>;
    fn get_model_name(&self) -> String;
}
