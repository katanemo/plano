use common::configuration::AgentUsagePreference;
use hermesllm::apis::openai::{ChatCompletionsRequest, Message};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum OrchestratorModelError {
    #[error("Failed to parse JSON: {0}")]
    JsonError(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, OrchestratorModelError>;

/// The result of running Plano-Orchestrator over a conversation: zero or more
/// selected routes (each mapped to its upstream model name) plus zero or more
/// selected Agent Skills. Skills are filtered down by the consumer to the
/// catalog defined under `routing_preferences[].skills` for the chosen route.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrchestratorSelection {
    pub routes: Vec<(String, String)>,
    pub skills: Vec<String>,
}

impl OrchestratorSelection {
    pub fn is_empty(&self) -> bool {
        self.routes.is_empty() && self.skills.is_empty()
    }
}

/// OrchestratorModel trait for handling orchestration requests.
/// Returns multiple routes and skills as the model output format is:
/// {"route": ["route_name_1", ...], "skills": ["skill_name_1", ...]}
pub trait OrchestratorModel: Send + Sync {
    fn generate_request(
        &self,
        messages: &[Message],
        usage_preferences: &Option<Vec<AgentUsagePreference>>,
    ) -> ChatCompletionsRequest;
    /// Parses the orchestrator's raw model output into selected routes (each
    /// mapped to a model) and selected skill names.
    fn parse_response(
        &self,
        content: &str,
        usage_preferences: &Option<Vec<AgentUsagePreference>>,
    ) -> Result<Option<OrchestratorSelection>>;
    fn get_model_name(&self) -> String;
}
