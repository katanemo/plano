use std::collections::HashMap;
use std::sync::Arc;

use common::configuration::{Agent, Listener, ModelAlias};
use common::llm_providers::LlmProviders;
use tokio::sync::RwLock;

use crate::router::llm::RouterService;
use crate::router::orchestrator::OrchestratorService;
use crate::state::StateStorage;

/// Shared application state bundled into a single Arc-wrapped struct.
///
/// Instead of cloning 8+ individual `Arc`s per connection, a single
/// `Arc<AppState>` is cloned once and passed to the request handler.
pub struct AppState {
    pub router_service: Arc<RouterService>,
    pub orchestrator_service: Arc<OrchestratorService>,
    pub model_aliases: Arc<Option<HashMap<String, ModelAlias>>>,
    pub llm_providers: Arc<RwLock<LlmProviders>>,
    pub agents_list: Arc<RwLock<Option<Vec<Agent>>>>,
    pub listeners: Arc<RwLock<Vec<Listener>>>,
    pub state_storage: Option<Arc<dyn StateStorage>>,
    pub llm_provider_url: String,
    /// Shared HTTP client for upstream LLM requests (connection pooling / keep-alive).
    pub http_client: reqwest::Client,
}
