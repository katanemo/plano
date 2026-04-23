use std::collections::HashMap;
use std::sync::Arc;

use common::configuration::{Agent, FilterPipeline, Listener, ModelAlias, SpanAttributes};
use common::llm_providers::LlmProviders;
use tokio::sync::RwLock;

use crate::router::orchestrator::OrchestratorService;
use crate::state::StateStorage;

/// Shared application state bundled into a single Arc-wrapped struct.
///
/// Instead of cloning 8+ individual `Arc`s per connection, a single
/// `Arc<AppState>` is cloned once and passed to the request handler.
pub struct AppState {
    pub orchestrator_service: Arc<OrchestratorService>,
    pub model_aliases: Option<HashMap<String, ModelAlias>>,
    pub llm_providers: Arc<RwLock<LlmProviders>>,
    pub agents_list: Option<Vec<Agent>>,
    pub listeners: Vec<Listener>,
    pub state_storage: Option<Arc<dyn StateStorage>>,
    pub llm_provider_url: String,
    pub span_attributes: Option<SpanAttributes>,
    /// Shared HTTP client for upstream LLM requests (connection pooling / keep-alive).
    pub http_client: reqwest::Client,
    pub filter_pipeline: Arc<FilterPipeline>,
    /// When false, agentic signal analysis is skipped on LLM responses to save CPU.
    /// Controlled by `overrides.disable_signals` in plano config.
    pub signals_enabled: bool,
}
