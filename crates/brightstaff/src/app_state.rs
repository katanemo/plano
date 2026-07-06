use std::collections::HashMap;
use std::sync::Arc;

use common::configuration::{
    Agent, FilterPipeline, Listener, ModelAlias, PromptCachingOverride, SpanAttributes,
};
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
    /// Request header whose value populates the observability `distinct_id`
    /// (e.g. PostHog). Sourced from `tracing.exporters[].distinct_id_header`.
    /// `None` means LLM events are captured anonymously.
    pub distinct_id_header: Option<String>,
    /// Shared HTTP client for upstream LLM requests (connection pooling / keep-alive).
    pub http_client: reqwest::Client,
    pub filter_pipeline: Arc<FilterPipeline>,
    /// When false, agentic signal analysis is skipped on LLM responses to save CPU.
    /// Controlled by `overrides.disable_signals` in plano config.
    pub signals_enabled: bool,
    /// Scoped prompt-caching overrides from `overrides.prompt_caching`.
    pub prompt_caching_overrides: Option<Vec<PromptCachingOverride>>,
    /// Default for implicit session affinity (`routing.implicit_session_affinity`,
    /// default true). Per-scope `overrides.prompt_caching` entries can still flip it.
    pub implicit_affinity_default: bool,
}
