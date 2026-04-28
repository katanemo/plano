use hermesllm::apis::openai::{ModelDetail, ModelObject, Models};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use std::fmt::Display;

use crate::api::open_ai::{
    ChatCompletionTool, FunctionDefinition, FunctionParameter, FunctionParameters, ParameterType,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SessionCacheType {
    #[default]
    Memory,
    Redis,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCacheConfig {
    #[serde(rename = "type", default)]
    pub cache_type: SessionCacheType,
    /// Redis URL, e.g. `redis://localhost:6379`. Required when `type` is `redis`.
    pub url: Option<String>,
    /// Optional HTTP header name whose value is used as a tenant prefix in the cache key.
    /// When set, keys are scoped as `plano:affinity:{tenant_id}:{session_id}`.
    pub tenant_header: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Routing {
    pub llm_provider: Option<String>,
    pub model: Option<String>,
    pub session_ttl_seconds: Option<u64>,
    pub session_max_entries: Option<usize>,
    pub session_cache: Option<SessionCacheConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelAlias {
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub transport: Option<String>,
    pub tool: Option<String>,
    pub url: String,
    #[serde(rename = "type")]
    pub agent_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentFilterChain {
    pub id: String,
    pub default: Option<bool>,
    pub description: Option<String>,
    pub input_filters: Option<Vec<String>>,
}

/// A filter chain with its agent references resolved to concrete Agent objects.
/// Bundles the ordered filter IDs with the agent lookup map so they stay in sync.
#[derive(Debug, Clone, Default)]
pub struct ResolvedFilterChain {
    pub filter_ids: Vec<String>,
    pub agents: HashMap<String, Agent>,
}

impl ResolvedFilterChain {
    pub fn is_empty(&self) -> bool {
        self.filter_ids.is_empty()
    }

    pub fn to_agent_filter_chain(&self, id: &str) -> AgentFilterChain {
        AgentFilterChain {
            id: id.to_string(),
            default: None,
            description: None,
            input_filters: Some(self.filter_ids.clone()),
        }
    }
}

/// Holds resolved input and output filter chains for a model listener.
#[derive(Debug, Clone, Default)]
pub struct FilterPipeline {
    pub input: Option<ResolvedFilterChain>,
    pub output: Option<ResolvedFilterChain>,
}

impl FilterPipeline {
    pub fn has_input_filters(&self) -> bool {
        self.input.as_ref().is_some_and(|c| !c.is_empty())
    }

    pub fn has_output_filters(&self) -> bool {
        self.output.as_ref().is_some_and(|c| !c.is_empty())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ListenerType {
    Model,
    Agent,
    Prompt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Listener {
    #[serde(rename = "type")]
    pub listener_type: ListenerType,
    pub name: String,
    pub router: Option<String>,
    pub agents: Option<Vec<AgentFilterChain>>,
    pub input_filters: Option<Vec<String>>,
    pub output_filters: Option<Vec<String>>,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateStorageConfig {
    #[serde(rename = "type")]
    pub storage_type: StateStorageType,
    pub connection_string: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StateStorageType {
    Memory,
    Postgres,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SelectionPreference {
    Cheapest,
    Fastest,
    /// Return models in the same order they were defined — no reordering.
    #[default]
    #[serde(alias = "")]
    None,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SelectionPolicy {
    #[serde(default, deserialize_with = "deserialize_selection_preference")]
    pub prefer: SelectionPreference,
}

fn deserialize_selection_preference<'de, D>(
    deserializer: D,
) -> Result<SelectionPreference, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<SelectionPreference>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopLevelRoutingPreference {
    pub name: String,
    pub description: String,
    pub models: Vec<String>,
    #[serde(default)]
    pub selection_policy: SelectionPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MetricsSource {
    Cost(CostMetricsConfig),
    Latency(LatencyMetricsConfig),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostMetricsConfig {
    pub provider: CostProvider,
    pub refresh_interval: Option<u64>,
    /// Map DO catalog keys (`lowercase(creator)/model_id`) to Plano model names.
    /// Example: `openai/openai-gpt-oss-120b: openai/gpt-4o`
    pub model_aliases: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostProvider {
    Digitalocean,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyMetricsConfig {
    pub provider: LatencyProvider,
    pub url: String,
    pub query: String,
    pub refresh_interval: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LatencyProvider {
    Prometheus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Configuration {
    pub version: String,
    pub endpoints: Option<HashMap<String, Endpoint>>,
    pub model_providers: Vec<LlmProvider>,
    pub model_aliases: Option<HashMap<String, ModelAlias>>,
    pub overrides: Option<Overrides>,
    pub routing: Option<Routing>,
    pub system_prompt: Option<String>,
    pub prompt_guards: Option<PromptGuards>,
    pub prompt_targets: Option<Vec<PromptTarget>>,
    pub error_target: Option<ErrorTargetDetail>,
    pub ratelimits: Option<Vec<Ratelimit>>,
    pub tracing: Option<Tracing>,
    pub mode: Option<GatewayMode>,
    pub agents: Option<Vec<Agent>>,
    pub filters: Option<Vec<Agent>>,
    pub listeners: Vec<Listener>,
    pub state_storage: Option<StateStorageConfig>,
    pub routing_preferences: Option<Vec<TopLevelRoutingPreference>>,
    pub model_metrics_sources: Option<Vec<MetricsSource>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Overrides {
    pub prompt_target_intent_matching_threshold: Option<f64>,
    pub optimize_context_window: Option<bool>,
    pub use_agent_orchestrator: Option<bool>,
    pub llm_routing_model: Option<String>,
    pub agent_orchestration_model: Option<String>,
    pub orchestrator_model_context_length: Option<usize>,
    pub disable_signals: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Tracing {
    pub sampling_rate: Option<f64>,
    pub trace_arch_internal: Option<bool>,
    pub random_sampling: Option<u32>,
    pub opentracing_grpc_endpoint: Option<String>,
    pub span_attributes: Option<SpanAttributes>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SpanAttributes {
    pub header_prefixes: Option<Vec<String>>,
    #[serde(rename = "static")]
    pub static_attributes: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
pub enum GatewayMode {
    #[serde(rename = "llm")]
    Llm,
    #[default]
    #[serde(rename = "prompt")]
    Prompt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorTargetDetail {
    pub endpoint: Option<EndpointDetails>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PromptGuards {
    pub input_guards: HashMap<GuardType, GuardOptions>,
}

impl PromptGuards {
    pub fn jailbreak_on_exception_message(&self) -> Option<&str> {
        self.input_guards
            .get(&GuardType::Jailbreak)?
            .on_exception
            .as_ref()?
            .message
            .as_ref()?
            .as_str()
            .into()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum GuardType {
    #[serde(rename = "jailbreak")]
    Jailbreak,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardOptions {
    pub on_exception: Option<OnExceptionDetails>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnExceptionDetails {
    pub forward_to_error_target: Option<bool>,
    pub error_handler: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRatelimit {
    pub selector: LlmRatelimitSelector,
    pub limit: Limit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRatelimitSelector {
    pub http_header: Option<RatelimitHeader>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Header {
    pub key: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ratelimit {
    pub model: String,
    pub selector: Header,
    pub limit: Limit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Limit {
    pub tokens: u32,
    pub unit: TimeUnit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TimeUnit {
    #[serde(rename = "second")]
    Second,
    #[serde(rename = "minute")]
    Minute,
    #[serde(rename = "hour")]
    Hour,
    #[serde(rename = "day")]
    Day,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct RatelimitHeader {
    pub name: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
//TODO: use enum for model, but if there is a new model, we need to update the code
pub struct EmbeddingProviver {
    pub name: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum LlmProviderType {
    #[serde(rename = "anthropic")]
    Anthropic,
    #[serde(rename = "deepseek")]
    Deepseek,
    #[serde(rename = "groq")]
    Groq,
    #[serde(rename = "mistral")]
    Mistral,
    #[serde(rename = "openai")]
    OpenAI,
    #[serde(rename = "xiaomi")]
    Xiaomi,
    #[serde(rename = "gemini")]
    Gemini,
    #[serde(rename = "xai")]
    XAI,
    #[serde(rename = "together_ai")]
    TogetherAI,
    #[serde(rename = "azure_openai")]
    AzureOpenAI,
    #[serde(rename = "ollama")]
    Ollama,
    #[serde(rename = "moonshotai")]
    Moonshotai,
    #[serde(rename = "zhipu")]
    Zhipu,
    #[serde(rename = "qwen")]
    Qwen,
    #[serde(rename = "amazon_bedrock")]
    AmazonBedrock,
    #[serde(rename = "plano")]
    Plano,
    #[serde(rename = "chatgpt")]
    ChatGPT,
    #[serde(rename = "digitalocean")]
    DigitalOcean,
    #[serde(rename = "vercel")]
    Vercel,
    #[serde(rename = "openrouter")]
    OpenRouter,
}

impl Display for LlmProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmProviderType::Anthropic => write!(f, "anthropic"),
            LlmProviderType::Deepseek => write!(f, "deepseek"),
            LlmProviderType::Groq => write!(f, "groq"),
            LlmProviderType::Gemini => write!(f, "gemini"),
            LlmProviderType::Mistral => write!(f, "mistral"),
            LlmProviderType::OpenAI => write!(f, "openai"),
            LlmProviderType::Xiaomi => write!(f, "xiaomi"),
            LlmProviderType::XAI => write!(f, "xai"),
            LlmProviderType::TogetherAI => write!(f, "together_ai"),
            LlmProviderType::AzureOpenAI => write!(f, "azure_openai"),
            LlmProviderType::Ollama => write!(f, "ollama"),
            LlmProviderType::Moonshotai => write!(f, "moonshotai"),
            LlmProviderType::Zhipu => write!(f, "zhipu"),
            LlmProviderType::Qwen => write!(f, "qwen"),
            LlmProviderType::AmazonBedrock => write!(f, "amazon_bedrock"),
            LlmProviderType::Plano => write!(f, "plano"),
            LlmProviderType::ChatGPT => write!(f, "chatgpt"),
            LlmProviderType::DigitalOcean => write!(f, "digitalocean"),
            LlmProviderType::Vercel => write!(f, "vercel"),
            LlmProviderType::OpenRouter => write!(f, "openrouter"),
        }
    }
}

impl LlmProviderType {
    /// Get the ProviderId for this LlmProviderType
    /// Used with the new function-based hermesllm API
    pub fn to_provider_id(&self) -> hermesllm::ProviderId {
        hermesllm::ProviderId::try_from(self.to_string().as_str())
            .expect("LlmProviderType should always map to a valid ProviderId")
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct AgentUsagePreference {
    pub model: String,
    pub orchestration_preferences: Vec<OrchestrationPreference>,
}

/// OrchestrationPreference with custom serialization to always include default parameters.
/// The parameters field is always serialized as:
/// {"type": "object", "properties": {}, "required": []}
#[derive(Debug, Clone, Deserialize)]
pub struct OrchestrationPreference {
    pub name: String,
    pub description: String,
}

impl serde::Serialize for OrchestrationPreference {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("OrchestrationPreference", 3)?;
        state.serialize_field("name", &self.name)?;
        state.serialize_field("description", &self.description)?;
        state.serialize_field(
            "parameters",
            &serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        )?;
        state.end()
    }
}

// ── Retry Policy Configuration Types ──────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryStrategy {
    SameModel,
    SameProvider,
    DifferentProvider,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockScope {
    Model,
    Provider,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyTo {
    Global,
    Request,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackoffApplyTo {
    SameModel,
    SameProvider,
    Global,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LatencyMeasure {
    Ttfb,
    Total,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StatusCodeEntry {
    Single(u16),
    Range(String),
}

impl StatusCodeEntry {
    /// Expand a StatusCodeEntry into a list of individual status codes.
    /// For Single, returns a vec with one element.
    /// For Range (e.g. "502-504"), returns [502, 503, 504].
    pub fn expand(&self) -> Result<Vec<u16>, String> {
        match self {
            StatusCodeEntry::Single(code) => Ok(vec![*code]),
            StatusCodeEntry::Range(range_str) => {
                let parts: Vec<&str> = range_str.split('-').collect();
                if parts.len() != 2 {
                    return Err(format!(
                        "Invalid status code range format: '{}'. Expected 'start-end'.",
                        range_str
                    ));
                }
                let start: u16 = parts[0]
                    .trim()
                    .parse()
                    .map_err(|_| format!("Invalid start in status code range: '{}'", parts[0]))?;
                let end: u16 = parts[1]
                    .trim()
                    .parse()
                    .map_err(|_| format!("Invalid end in status code range: '{}'", parts[1]))?;
                if start > end {
                    return Err(format!(
                        "Status code range start ({}) must be <= end ({})",
                        start, end
                    ));
                }
                Ok((start..=end).collect())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusCodeConfig {
    pub codes: Vec<StatusCodeEntry>,
    pub strategy: RetryStrategy,
    pub max_attempts: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimeoutRetryConfig {
    pub strategy: RetryStrategy,
    pub max_attempts: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackoffConfig {
    pub apply_to: BackoffApplyTo,
    #[serde(default = "default_base_ms")]
    pub base_ms: u64,
    #[serde(default = "default_max_ms")]
    pub max_ms: u64,
    #[serde(default = "default_jitter")]
    pub jitter: bool,
}

fn default_base_ms() -> u64 {
    100
}
fn default_max_ms() -> u64 {
    5000
}
fn default_jitter() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetryAfterHandlingConfig {
    #[serde(default = "default_retry_after_scope")]
    pub scope: BlockScope,
    #[serde(default = "default_retry_after_apply_to")]
    pub apply_to: ApplyTo,
    #[serde(default = "default_max_retry_after_seconds")]
    pub max_retry_after_seconds: u64,
}

fn default_retry_after_scope() -> BlockScope {
    BlockScope::Model
}
fn default_retry_after_apply_to() -> ApplyTo {
    ApplyTo::Global
}
fn default_max_retry_after_seconds() -> u64 {
    300
}

impl Default for RetryAfterHandlingConfig {
    fn default() -> Self {
        Self {
            scope: BlockScope::Model,
            apply_to: ApplyTo::Global,
            max_retry_after_seconds: 300,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HighLatencyConfig {
    pub threshold_ms: u64,
    #[serde(default = "default_latency_measure")]
    pub measure: LatencyMeasure,
    #[serde(default = "default_min_triggers")]
    pub min_triggers: u32,
    pub trigger_window_seconds: Option<u64>,
    pub strategy: RetryStrategy,
    pub max_attempts: u32,
    #[serde(default = "default_block_duration")]
    pub block_duration_seconds: u64,
    #[serde(default = "default_block_scope")]
    pub scope: BlockScope,
    #[serde(default = "default_high_latency_apply_to")]
    pub apply_to: ApplyTo,
}

fn default_latency_measure() -> LatencyMeasure {
    LatencyMeasure::Ttfb
}
fn default_min_triggers() -> u32 {
    1
}
fn default_block_duration() -> u64 {
    300
}
fn default_block_scope() -> BlockScope {
    BlockScope::Model
}
fn default_high_latency_apply_to() -> ApplyTo {
    ApplyTo::Global
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetryPolicy {
    #[serde(default)]
    pub fallback_models: Vec<String>,
    #[serde(default = "default_retry_strategy")]
    pub default_strategy: RetryStrategy,
    #[serde(default = "default_max_attempts")]
    pub default_max_attempts: u32,
    #[serde(default)]
    pub on_status_codes: Vec<StatusCodeConfig>,
    pub on_timeout: Option<TimeoutRetryConfig>,
    pub on_high_latency: Option<HighLatencyConfig>,
    pub backoff: Option<BackoffConfig>,
    pub retry_after_handling: Option<RetryAfterHandlingConfig>,
    pub max_retry_duration_ms: Option<u64>,
}

fn default_retry_strategy() -> RetryStrategy {
    RetryStrategy::DifferentProvider
}
fn default_max_attempts() -> u32 {
    2
}

impl RetryPolicy {
    /// Get the effective Retry-After handling config.
    /// Always returns a config when retry_policy exists (Retry-After is always-on).
    pub fn effective_retry_after_config(&self) -> RetryAfterHandlingConfig {
        self.retry_after_handling.clone().unwrap_or_default()
    }
}

/// Extract provider prefix from a model identifier.
/// e.g., "openai/gpt-4o" -> "openai"
pub fn extract_provider(model_id: &str) -> &str {
    model_id.split('/').next().unwrap_or(model_id)
}

// ── End Retry Policy Configuration Types ─────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
//TODO: use enum for model, but if there is a new model, we need to update the code
pub struct LlmProvider {
    pub name: String,
    pub provider_interface: LlmProviderType,
    pub access_key: Option<String>,
    pub model: Option<String>,
    pub default: Option<bool>,
    pub stream: Option<bool>,
    pub endpoint: Option<String>,
    pub port: Option<u16>,
    pub rate_limits: Option<LlmRatelimit>,
    pub usage: Option<String>,
    pub cluster_name: Option<String>,
    pub base_url_path_prefix: Option<String>,
    pub internal: Option<bool>,
    pub passthrough_auth: Option<bool>,
    pub headers: Option<HashMap<String, String>>,
    /// Retry policy configuration. When None, retry logic is disabled.
    pub retry_policy: Option<RetryPolicy>,
}

pub trait IntoModels {
    fn into_models(self) -> Models;
}

impl IntoModels for Vec<LlmProvider> {
    fn into_models(self) -> Models {
        let data = self
            .iter()
            .filter(|provider| provider.internal != Some(true))
            .map(|provider| ModelDetail {
                id: provider.name.clone(),
                object: Some("model".to_string()),
                created: 0,
                owned_by: "system".to_string(),
            })
            .collect();

        Models {
            object: ModelObject::List,
            data,
        }
    }
}

impl Default for LlmProvider {
    fn default() -> Self {
        Self {
            name: "openai".to_string(),
            provider_interface: LlmProviderType::OpenAI,
            access_key: None,
            model: None,
            default: Some(true),
            stream: Some(false),
            endpoint: None,
            port: None,
            rate_limits: None,
            usage: None,
            cluster_name: None,
            base_url_path_prefix: None,
            internal: None,
            passthrough_auth: None,
            headers: None,
            retry_policy: None,
        }
    }
}

impl Display for LlmProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

impl LlmProvider {
    /// Get the ProviderId for this LlmProvider
    /// Used with the new function-based hermesllm API
    pub fn to_provider_id(&self) -> hermesllm::ProviderId {
        self.provider_interface.to_provider_id()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Endpoint {
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    #[serde(rename = "type")]
    pub parameter_type: Option<String>,
    pub description: String,
    pub required: Option<bool>,
    #[serde(rename = "enum")]
    pub enum_values: Option<Vec<String>>,
    pub default: Option<String>,
    pub in_path: Option<bool>,
    pub format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
pub enum HttpMethod {
    #[default]
    #[serde(rename = "GET")]
    Get,
    #[serde(rename = "POST")]
    Post,
}

impl Display for HttpMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpMethod::Get => write!(f, "GET"),
            HttpMethod::Post => write!(f, "POST"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointDetails {
    pub name: String,
    pub path: Option<String>,
    #[serde(rename = "http_method")]
    pub method: Option<HttpMethod>,
    pub http_headers: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTarget {
    pub name: String,
    pub default: Option<bool>,
    pub description: String,
    pub endpoint: Option<EndpointDetails>,
    pub parameters: Option<Vec<Parameter>>,
    pub system_prompt: Option<String>,
    pub auto_llm_dispatch_on_response: Option<bool>,
}

// convert PromptTarget to ChatCompletionTool
impl From<&PromptTarget> for ChatCompletionTool {
    fn from(val: &PromptTarget) -> Self {
        let properties: HashMap<String, FunctionParameter> = match val.parameters {
            Some(ref entities) => {
                let mut properties: HashMap<String, FunctionParameter> = HashMap::new();
                for entity in entities.iter() {
                    let param = FunctionParameter {
                        parameter_type: ParameterType::from(
                            entity.parameter_type.clone().unwrap_or("str".to_string()),
                        ),
                        description: entity.description.clone(),
                        required: entity.required,
                        enum_values: entity.enum_values.clone(),
                        default: entity.default.clone(),
                        format: entity.format.clone(),
                    };
                    properties.insert(entity.name.clone(), param);
                }
                properties
            }
            None => HashMap::new(),
        };

        ChatCompletionTool {
            tool_type: crate::api::open_ai::ToolType::Function,
            function: FunctionDefinition {
                name: val.name.clone(),
                description: val.description.clone(),
                parameters: FunctionParameters { properties },
            },
        }
    }
}

#[cfg(test)]
mod test {
    use pretty_assertions::assert_eq;
    use std::fs;

    use super::{IntoModels, LlmProvider, LlmProviderType};
    use crate::api::open_ai::ToolType;

    use proptest::prelude::*;

    // ── Proptest Strategies for Retry Config Types ─────────────────────────

    fn arb_retry_strategy() -> impl Strategy<Value = super::RetryStrategy> {
        prop_oneof![
            Just(super::RetryStrategy::SameModel),
            Just(super::RetryStrategy::SameProvider),
            Just(super::RetryStrategy::DifferentProvider),
        ]
    }

    fn arb_block_scope() -> impl Strategy<Value = super::BlockScope> {
        prop_oneof![
            Just(super::BlockScope::Model),
            Just(super::BlockScope::Provider),
        ]
    }

    fn arb_apply_to() -> impl Strategy<Value = super::ApplyTo> {
        prop_oneof![Just(super::ApplyTo::Global), Just(super::ApplyTo::Request),]
    }

    fn arb_backoff_apply_to() -> impl Strategy<Value = super::BackoffApplyTo> {
        prop_oneof![
            Just(super::BackoffApplyTo::SameModel),
            Just(super::BackoffApplyTo::SameProvider),
            Just(super::BackoffApplyTo::Global),
        ]
    }

    fn arb_latency_measure() -> impl Strategy<Value = super::LatencyMeasure> {
        prop_oneof![
            Just(super::LatencyMeasure::Ttfb),
            Just(super::LatencyMeasure::Total),
        ]
    }

    fn arb_status_code_entry() -> impl Strategy<Value = super::StatusCodeEntry> {
        prop_oneof![
            (100u16..=599u16).prop_map(super::StatusCodeEntry::Single),
            (100u16..=599u16)
                .prop_flat_map(|start| (Just(start), start..=599u16))
                .prop_map(|(start, end)| super::StatusCodeEntry::Range(format!(
                    "{}-{}",
                    start, end
                ))),
        ]
    }

    fn arb_status_code_config() -> impl Strategy<Value = super::StatusCodeConfig> {
        (
            prop::collection::vec(arb_status_code_entry(), 1..=3),
            arb_retry_strategy(),
            1u32..=10u32,
        )
            .prop_map(|(codes, strategy, max_attempts)| super::StatusCodeConfig {
                codes,
                strategy,
                max_attempts,
            })
    }

    fn arb_timeout_retry_config() -> impl Strategy<Value = super::TimeoutRetryConfig> {
        (arb_retry_strategy(), 1u32..=10u32).prop_map(|(strategy, max_attempts)| {
            super::TimeoutRetryConfig {
                strategy,
                max_attempts,
            }
        })
    }

    fn arb_backoff_config() -> impl Strategy<Value = super::BackoffConfig> {
        (arb_backoff_apply_to(), 1u64..=1000u64, prop::bool::ANY)
            .prop_flat_map(|(apply_to, base_ms, jitter)| {
                let max_ms_min = base_ms + 1;
                (
                    Just(apply_to),
                    Just(base_ms),
                    max_ms_min..=(base_ms + 50000),
                    Just(jitter),
                )
            })
            .prop_map(|(apply_to, base_ms, max_ms, jitter)| super::BackoffConfig {
                apply_to,
                base_ms,
                max_ms,
                jitter,
            })
    }

    fn arb_retry_after_handling_config() -> impl Strategy<Value = super::RetryAfterHandlingConfig> {
        (arb_block_scope(), arb_apply_to(), 1u64..=3600u64).prop_map(
            |(scope, apply_to, max_retry_after_seconds)| super::RetryAfterHandlingConfig {
                scope,
                apply_to,
                max_retry_after_seconds,
            },
        )
    }

    fn arb_high_latency_config() -> impl Strategy<Value = super::HighLatencyConfig> {
        (
            1u64..=60000u64,
            arb_latency_measure(),
            1u32..=10u32,
            arb_retry_strategy(),
            1u32..=10u32,
            1u64..=3600u64,
            arb_block_scope(),
            arb_apply_to(),
        )
            .prop_map(
                |(
                    threshold_ms,
                    measure,
                    min_triggers,
                    strategy,
                    max_attempts,
                    block_duration_seconds,
                    scope,
                    apply_to,
                )| {
                    let trigger_window_seconds = if min_triggers > 1 { Some(60u64) } else { None };
                    super::HighLatencyConfig {
                        threshold_ms,
                        measure,
                        min_triggers,
                        trigger_window_seconds,
                        strategy,
                        max_attempts,
                        block_duration_seconds,
                        scope,
                        apply_to,
                    }
                },
            )
    }

    fn arb_retry_policy() -> impl Strategy<Value = super::RetryPolicy> {
        (
            prop::collection::vec("[a-z]{2,6}/[a-z0-9-]{3,10}", 0..=3),
            arb_retry_strategy(),
            1u32..=10u32,
            prop::collection::vec(arb_status_code_config(), 0..=3),
            prop::option::of(arb_timeout_retry_config()),
            prop::option::of(arb_high_latency_config()),
            prop::option::of(arb_backoff_config()),
            prop::option::of(arb_retry_after_handling_config()),
            prop::option::of(1u64..=120000u64),
        )
            .prop_map(
                |(
                    fallback_models,
                    default_strategy,
                    default_max_attempts,
                    on_status_codes,
                    on_timeout,
                    on_high_latency,
                    backoff,
                    retry_after_handling,
                    max_retry_duration_ms,
                )| {
                    super::RetryPolicy {
                        fallback_models,
                        default_strategy,
                        default_max_attempts,
                        on_status_codes,
                        on_timeout,
                        on_high_latency,
                        backoff,
                        retry_after_handling,
                        max_retry_duration_ms,
                    }
                },
            )
    }

    // ── Property Tests ─────────────────────────────────────────────────────

    // Feature: retry-on-ratelimit, Property 1: Configuration Round-Trip Parsing
    // **Validates: Requirements 1.2**
    proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(100))]

        /// Property 1: Configuration Round-Trip Parsing
        /// Generate arbitrary valid RetryPolicy structs, serialize to YAML,
        /// re-parse, and assert equivalence.
        #[test]
        fn prop_retry_policy_round_trip(policy in arb_retry_policy()) {
            let yaml = serde_yaml::to_string(&policy)
                .expect("serialization should succeed");
            let parsed: super::RetryPolicy = serde_yaml::from_str(&yaml)
                .expect("deserialization should succeed");

            // Direct structural equality — all types derive PartialEq
            prop_assert_eq!(&policy, &parsed);
        }

    }

    // Feature: retry-on-ratelimit, Property 2: Configuration Defaults Applied Correctly
    // **Validates: Requirements 1.2**
    proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(100))]

        /// Property 2: Configuration Defaults Applied Correctly
        /// Generate RetryPolicy YAML with optional fields omitted, parse,
        /// and assert correct defaults are applied.
        #[test]
        fn prop_retry_policy_defaults(
            include_on_status_codes in prop::bool::ANY,
            include_backoff in prop::bool::ANY,
            include_retry_after in prop::bool::ANY,
            include_on_timeout in prop::bool::ANY,
            include_on_high_latency in prop::bool::ANY,
        ) {
            // Build a minimal YAML — RetryPolicy has serde defaults for all fields,
            // so even an empty mapping is valid.
            let mut parts: Vec<String> = Vec::new();

            // When we include sections, only provide required sub-fields so
            // we can verify the optional sub-fields get their defaults.
            if include_on_status_codes {
                parts.push("on_status_codes:\n  - codes: [429]\n    strategy: same_model\n    max_attempts: 2".to_string());
            }
            if include_backoff {
                parts.push("backoff:\n  apply_to: global".to_string());
            }
            if include_retry_after {
                parts.push("retry_after_handling:\n  scope: provider".to_string());
            }
            if include_on_timeout {
                parts.push("on_timeout:\n  strategy: same_model\n  max_attempts: 1".to_string());
            }
            if include_on_high_latency {
                parts.push("on_high_latency:\n  threshold_ms: 5000\n  strategy: different_provider\n  max_attempts: 2".to_string());
            }

            let yaml = if parts.is_empty() {
                "{}".to_string()
            } else {
                parts.join("\n")
            };

            let parsed: super::RetryPolicy = serde_yaml::from_str(&yaml)
                .expect("deserialization should succeed");

            // Assert top-level defaults
            prop_assert_eq!(parsed.default_strategy, super::RetryStrategy::DifferentProvider);
            prop_assert_eq!(parsed.default_max_attempts, 2);
            prop_assert!(parsed.fallback_models.is_empty());
            prop_assert_eq!(parsed.max_retry_duration_ms, None);

            // Assert on_status_codes defaults to empty vec
            if !include_on_status_codes {
                prop_assert!(parsed.on_status_codes.is_empty());
            }

            // Assert backoff defaults when present
            if include_backoff {
                let backoff = parsed.backoff.as_ref().unwrap();
                prop_assert_eq!(backoff.base_ms, 100);
                prop_assert_eq!(backoff.max_ms, 5000);
                prop_assert_eq!(backoff.jitter, true);
            } else {
                prop_assert!(parsed.backoff.is_none());
            }

            // Assert retry_after_handling defaults when present
            if include_retry_after {
                let rah = parsed.retry_after_handling.as_ref().unwrap();
                prop_assert_eq!(rah.scope, super::BlockScope::Provider); // explicitly set
                prop_assert_eq!(rah.apply_to, super::ApplyTo::Global); // default
                prop_assert_eq!(rah.max_retry_after_seconds, 300); // default
            } else {
                prop_assert!(parsed.retry_after_handling.is_none());
            }

            // Assert effective_retry_after_config always returns valid defaults
            let effective = parsed.effective_retry_after_config();
            if include_retry_after {
                prop_assert_eq!(effective.scope, super::BlockScope::Provider);
            } else {
                prop_assert_eq!(effective.scope, super::BlockScope::Model);
            }
            prop_assert_eq!(effective.apply_to, super::ApplyTo::Global);
            prop_assert_eq!(effective.max_retry_after_seconds, 300);

            // Assert high latency defaults when present
            if include_on_high_latency {
                let hl = parsed.on_high_latency.as_ref().unwrap();
                prop_assert_eq!(hl.measure, super::LatencyMeasure::Ttfb); // default
                prop_assert_eq!(hl.min_triggers, 1); // default
                prop_assert_eq!(hl.block_duration_seconds, 300); // default
                prop_assert_eq!(hl.scope, super::BlockScope::Model); // default
                prop_assert_eq!(hl.apply_to, super::ApplyTo::Global); // default
            }
        }
    }

    #[test]
    fn test_deserialize_configuration() {
        let ref_config = fs::read_to_string(
            "../../docs/source/resources/includes/plano_config_full_reference_rendered.yaml",
        )
        .expect("reference config file not found");

        let config: super::Configuration = serde_yaml::from_str(&ref_config).unwrap();
        assert_eq!(config.version, "v0.4.0");

        if let Some(prompt_targets) = &config.prompt_targets {
            assert!(
                !prompt_targets.is_empty(),
                "prompt_targets should not be empty if present"
            );
        }

        if let Some(tracing) = config.tracing.as_ref() {
            if let Some(sampling_rate) = tracing.sampling_rate {
                assert_eq!(sampling_rate, 0.1);
            }
        }

        let mode = config.mode.as_ref().unwrap_or(&super::GatewayMode::Prompt);
        assert_eq!(*mode, super::GatewayMode::Prompt);
    }

    #[test]
    fn test_tool_conversion() {
        let ref_config = fs::read_to_string(
            "../../docs/source/resources/includes/plano_config_full_reference_rendered.yaml",
        )
        .expect("reference config file not found");
        let config: super::Configuration = serde_yaml::from_str(&ref_config).unwrap();
        if let Some(prompt_targets) = &config.prompt_targets {
            if let Some(prompt_target) = prompt_targets
                .iter()
                .find(|p| p.name == "reboot_network_device")
            {
                let chat_completion_tool: super::ChatCompletionTool = prompt_target.into();
                assert_eq!(chat_completion_tool.tool_type, ToolType::Function);
                assert_eq!(chat_completion_tool.function.name, "reboot_network_device");
                assert_eq!(
                    chat_completion_tool.function.description,
                    "Reboot a specific network device"
                );
                assert_eq!(chat_completion_tool.function.parameters.properties.len(), 2);
                assert!(chat_completion_tool
                    .function
                    .parameters
                    .properties
                    .contains_key("device_id"));
                let device_id_param = chat_completion_tool
                    .function
                    .parameters
                    .properties
                    .get("device_id")
                    .unwrap();
                assert_eq!(
                    device_id_param.parameter_type,
                    crate::api::open_ai::ParameterType::String
                );
                assert_eq!(
                    device_id_param.description,
                    "Identifier of the network device to reboot.".to_string()
                );
                assert_eq!(device_id_param.required, Some(true));
                let confirmation_param = chat_completion_tool
                    .function
                    .parameters
                    .properties
                    .get("confirmation")
                    .unwrap();
                assert_eq!(
                    confirmation_param.parameter_type,
                    crate::api::open_ai::ParameterType::Bool
                );
            }
        }
    }

    // Feature: retry-on-ratelimit, Property 4: Status Code Range Expansion
    // **Validates: Requirements 1.8**
    proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(100))]

        /// Property 4: Status Code Range Expansion — degenerate range (start == end)
        /// A range "N-N" should expand to a single-element vec containing N.
        #[test]
        fn prop_status_code_range_expansion(
            code in 100u16..=599u16,
        ) {
            let range_str = format!("{}-{}", code, code);
            let entry = super::StatusCodeEntry::Range(range_str);
            let expanded = entry.expand().expect("expand should succeed for valid range");
            prop_assert_eq!(expanded.len(), 1);
            prop_assert_eq!(expanded[0], code);
        }

        /// Property 4: Status Code Range Expansion — Single variant
        /// Generate arbitrary code (100..=599), expand, assert vec of length 1 containing that code.
        #[test]
        fn prop_status_code_single_expansion(code in 100u16..=599u16) {
            let entry = super::StatusCodeEntry::Single(code);
            let expanded = entry.expand().expect("expand should succeed for Single");
            prop_assert_eq!(expanded.len(), 1);
            prop_assert_eq!(expanded[0], code);
        }
    }

    proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(100))]

        /// Property 4: Status Code Range Expansion — arbitrary start..=end range
        /// Generate arbitrary valid range strings "start-end" (100 ≤ start ≤ end ≤ 599),
        /// expand, and assert correct count and bounds.
        #[test]
        fn prop_status_code_range_expansion_full(
            (start, end) in (100u16..=599u16).prop_flat_map(|s| (Just(s), s..=599u16))
        ) {
            let range_str = format!("{}-{}", start, end);
            let entry = super::StatusCodeEntry::Range(range_str);
            let expanded = entry.expand().expect("expand should succeed for valid range");

            let expected_len = (end - start + 1) as usize;
            prop_assert_eq!(expanded.len(), expected_len, "length should be end - start + 1");
            prop_assert_eq!(*expanded.first().unwrap(), start, "first element should be start");
            prop_assert_eq!(*expanded.last().unwrap(), end, "last element should be end");

            for &code in &expanded {
                prop_assert!(code >= start && code <= end, "all codes should be in [start, end]");
            }
        }
    }

    #[test]
    fn test_into_models_filters_internal_providers() {
        let providers = vec![
            LlmProvider {
                name: "openai-gpt4".to_string(),
                provider_interface: LlmProviderType::OpenAI,
                model: Some("gpt-4".to_string()),
                internal: None,
                ..Default::default()
            },
            LlmProvider {
                name: "plano-orchestrator".to_string(),
                provider_interface: LlmProviderType::Plano,
                model: Some("Plano-Orchestrator".to_string()),
                internal: Some(true),
                ..Default::default()
            },
        ];

        let models = providers.into_models();

        assert_eq!(models.data.len(), 1);

        let model_ids: Vec<String> = models.data.iter().map(|m| m.id.clone()).collect();
        assert!(model_ids.contains(&"openai-gpt4".to_string()));
        assert!(!model_ids.contains(&"plano-orchestrator".to_string()));
    }
    #[test]
    fn test_llm_provider_type_vercel_and_openrouter_roundtrip() {
        // Regression: brightstaff used to reject `provider_interface: vercel`
        // (and `openrouter`) because these variants were missing from
        // `LlmProviderType`, causing `planoai up` with the synthesized default
        // config to crash on startup.
        for (yaml_value, expected) in [
            ("vercel", LlmProviderType::Vercel),
            ("openrouter", LlmProviderType::OpenRouter),
        ] {
            let parsed: LlmProviderType =
                serde_yaml::from_str(yaml_value).expect("variant should deserialize");
            assert_eq!(parsed, expected);
            assert_eq!(parsed.to_string(), yaml_value);
            // to_provider_id() bridges into hermesllm; both providers must be
            // recognized there as well or this panics.
            let _ = parsed.to_provider_id();
        }
    }

    #[test]
    fn test_overrides_disable_signals_default_none() {
        let overrides = super::Overrides::default();
        assert_eq!(overrides.disable_signals, None);
    }

    #[test]
    fn test_overrides_disable_signals_deserialize() {
        let yaml = r#"
disable_signals: true
"#;
        let overrides: super::Overrides = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(overrides.disable_signals, Some(true));

        let yaml_false = r#"
disable_signals: false
"#;
        let overrides: super::Overrides = serde_yaml::from_str(yaml_false).unwrap();
        assert_eq!(overrides.disable_signals, Some(false));

        let yaml_missing = "{}";
        let overrides: super::Overrides = serde_yaml::from_str(yaml_missing).unwrap();
        assert_eq!(overrides.disable_signals, None);
    }

    // ── P0 Edge Case Tests: YAML Config Pattern Parsing ────────────────────

    /// Helper to parse a RetryPolicy from a YAML string.
    fn parse_retry_policy(yaml: &str) -> super::RetryPolicy {
        serde_yaml::from_str(yaml).expect("YAML should parse into RetryPolicy")
    }

    #[test]
    fn test_pattern1_multi_provider_failover_for_rate_limits() {
        let yaml = r#"
    fallback_models: [anthropic/claude-3-5-sonnet]
    on_status_codes:
      - codes: [429]
        strategy: "different_provider"
        max_attempts: 2
    "#;
        let policy = parse_retry_policy(yaml);
        assert_eq!(policy.fallback_models, vec!["anthropic/claude-3-5-sonnet"]);
        assert_eq!(policy.on_status_codes.len(), 1);
        assert_eq!(
            policy.on_status_codes[0].strategy,
            super::RetryStrategy::DifferentProvider
        );
        assert_eq!(policy.on_status_codes[0].max_attempts, 2);
    }

    #[test]
    fn test_pattern2_same_provider_failover_with_model_downgrade() {
        let yaml = r#"
    fallback_models: [openai/gpt-4o-mini, anthropic/claude-3-5-sonnet]
    on_status_codes:
      - codes: [429]
        strategy: "same_provider"
        max_attempts: 2
    "#;
        let policy = parse_retry_policy(yaml);
        assert_eq!(policy.fallback_models.len(), 2);
        assert_eq!(
            policy.on_status_codes[0].strategy,
            super::RetryStrategy::SameProvider
        );
    }

    #[test]
    fn test_pattern3_single_model_with_backoff_on_multiple_error_types() {
        let yaml = r#"
    fallback_models: []
    on_status_codes:
      - codes: [429]
        strategy: "same_model"
        max_attempts: 3
      - codes: [503]
        strategy: "same_model"
        max_attempts: 3
    backoff:
      apply_to: "same_model"
      base_ms: 500
    "#;
        let policy = parse_retry_policy(yaml);
        assert!(policy.fallback_models.is_empty());
        assert_eq!(policy.on_status_codes.len(), 2);
        let backoff = policy.backoff.unwrap();
        assert_eq!(backoff.apply_to, super::BackoffApplyTo::SameModel);
        assert_eq!(backoff.base_ms, 500);
        // max_ms defaults to 5000
        assert_eq!(backoff.max_ms, 5000);
    }

    #[test]
    fn test_pattern4_per_status_code_strategy_customization() {
        let yaml = r#"
    fallback_models: [openai/gpt-4o-mini, anthropic/claude-3-5-sonnet]
    default_strategy: "different_provider"
    default_max_attempts: 2
    on_status_codes:
      - codes: [429]
        strategy: "same_provider"
        max_attempts: 2
      - codes: [502]
        strategy: "different_provider"
        max_attempts: 3
      - codes: [503]
        strategy: "same_model"
        max_attempts: 2
      - codes: [504]
        strategy: "different_provider"
        max_attempts: 2
    on_timeout:
      strategy: "different_provider"
      max_attempts: 2
    "#;
        let policy = parse_retry_policy(yaml);
        assert_eq!(
            policy.default_strategy,
            super::RetryStrategy::DifferentProvider
        );
        assert_eq!(policy.default_max_attempts, 2);
        assert_eq!(policy.on_status_codes.len(), 4);
        assert_eq!(
            policy.on_status_codes[2].strategy,
            super::RetryStrategy::SameModel
        );
        let timeout = policy.on_timeout.unwrap();
        assert_eq!(timeout.strategy, super::RetryStrategy::DifferentProvider);
        assert_eq!(timeout.max_attempts, 2);
    }

    #[test]
    fn test_pattern5_timeout_specific_configuration() {
        let yaml = r#"
    fallback_models: [anthropic/claude-3-5-sonnet]
    default_strategy: "different_provider"
    default_max_attempts: 2
    on_status_codes:
      - codes: [429]
        strategy: "same_provider"
        max_attempts: 2
    on_timeout:
      strategy: "different_provider"
      max_attempts: 3
    "#;
        let policy = parse_retry_policy(yaml);
        let timeout = policy.on_timeout.unwrap();
        assert_eq!(timeout.max_attempts, 3);
    }

    #[test]
    fn test_pattern6_no_retry_parses_as_empty() {
        // Pattern 6: No retry_policy section. We test that an empty YAML
        // object parses with all defaults.
        let yaml = "{}";
        let policy = parse_retry_policy(yaml);
        assert!(policy.fallback_models.is_empty());
        assert_eq!(
            policy.default_strategy,
            super::RetryStrategy::DifferentProvider
        );
        assert_eq!(policy.default_max_attempts, 2);
        assert!(policy.on_status_codes.is_empty());
        assert!(policy.on_timeout.is_none());
        assert!(policy.backoff.is_none());
        assert!(policy.max_retry_duration_ms.is_none());
    }

    #[test]
    fn test_pattern7_backoff_only_for_same_model() {
        let yaml = r#"
    fallback_models: [anthropic/claude-3-5-sonnet]
    on_status_codes:
      - codes: [429]
        strategy: "same_model"
        max_attempts: 2
    backoff:
      apply_to: "same_model"
      base_ms: 100
      max_ms: 5000
      jitter: true
    "#;
        let policy = parse_retry_policy(yaml);
        let backoff = policy.backoff.unwrap();
        assert_eq!(backoff.apply_to, super::BackoffApplyTo::SameModel);
        assert!(backoff.jitter);
    }

    #[test]
    fn test_pattern8_backoff_for_same_provider() {
        let yaml = r#"
    fallback_models: [openai/gpt-4o-mini, anthropic/claude-3-5-sonnet]
    on_status_codes:
      - codes: [429]
        strategy: "same_provider"
        max_attempts: 2
    backoff:
      apply_to: "same_provider"
      base_ms: 200
      max_ms: 10000
      jitter: true
    "#;
        let policy = parse_retry_policy(yaml);
        let backoff = policy.backoff.unwrap();
        assert_eq!(backoff.apply_to, super::BackoffApplyTo::SameProvider);
        assert_eq!(backoff.base_ms, 200);
        assert_eq!(backoff.max_ms, 10000);
    }

    #[test]
    fn test_pattern9_global_backoff() {
        let yaml = r#"
    fallback_models: [anthropic/claude-3-5-sonnet]
    on_status_codes:
      - codes: [429]
        strategy: "different_provider"
        max_attempts: 2
    backoff:
      apply_to: "global"
      base_ms: 50
      max_ms: 2000
      jitter: true
    "#;
        let policy = parse_retry_policy(yaml);
        let backoff = policy.backoff.unwrap();
        assert_eq!(backoff.apply_to, super::BackoffApplyTo::Global);
        assert_eq!(backoff.base_ms, 50);
        assert_eq!(backoff.max_ms, 2000);
    }

    #[test]
    fn test_pattern10_deterministic_backoff_without_jitter() {
        let yaml = r#"
    fallback_models: []
    on_status_codes:
      - codes: [429]
        strategy: "same_model"
        max_attempts: 3
    backoff:
      apply_to: "same_model"
      base_ms: 1000
      max_ms: 30000
      jitter: false
    "#;
        let policy = parse_retry_policy(yaml);
        let backoff = policy.backoff.unwrap();
        assert!(!backoff.jitter);
        assert_eq!(backoff.base_ms, 1000);
        assert_eq!(backoff.max_ms, 30000);
    }

    #[test]
    fn test_pattern11_no_backoff_fast_failover() {
        let yaml = r#"
    fallback_models: [anthropic/claude-3-5-sonnet]
    on_status_codes:
      - codes: [429]
        strategy: "different_provider"
        max_attempts: 2
    "#;
        let policy = parse_retry_policy(yaml);
        assert!(policy.backoff.is_none());
    }

    #[test]
    fn test_pattern17_mixed_integer_and_range_codes() {
        let yaml = r#"
    fallback_models: [anthropic/claude-3-5-sonnet]
    default_strategy: "different_provider"
    default_max_attempts: 2
    on_status_codes:
      - codes: [429, "430-450", 526]
        strategy: "same_provider"
        max_attempts: 2
      - codes: ["502-504"]
        strategy: "different_provider"
        max_attempts: 3
    "#;
        let policy = parse_retry_policy(yaml);
        assert_eq!(policy.on_status_codes.len(), 2);

        // Verify first entry: 429 + range 430-450 + 526
        let first = &policy.on_status_codes[0];
        assert_eq!(first.codes.len(), 3);
        let expanded: Vec<u16> = first
            .codes
            .iter()
            .flat_map(|c| c.expand().unwrap())
            .collect();
        // 429 + (430..=450 = 21 codes) + 526 = 23 codes
        assert_eq!(expanded.len(), 23);
        assert!(expanded.contains(&429));
        assert!(expanded.contains(&430));
        assert!(expanded.contains(&450));
        assert!(expanded.contains(&526));
        assert!(!expanded.contains(&451));

        // Verify second entry: range 502-504
        let second = &policy.on_status_codes[1];
        let expanded2: Vec<u16> = second
            .codes
            .iter()
            .flat_map(|c| c.expand().unwrap())
            .collect();
        assert_eq!(expanded2, vec![502, 503, 504]);
    }

    #[test]
    fn test_pattern12_model_level_retry_after_blocking() {
        let yaml = r#"
    fallback_models: [openai/gpt-4o-mini, anthropic/claude-3-5-sonnet]
    on_status_codes:
      - codes: [429]
        strategy: "different_provider"
        max_attempts: 2
      - codes: [503]
        strategy: "different_provider"
        max_attempts: 2
    retry_after_handling:
      scope: "model"
      apply_to: "global"
    "#;
        let policy = parse_retry_policy(yaml);
        assert_eq!(policy.fallback_models.len(), 2);
        assert_eq!(policy.on_status_codes.len(), 2);
        let rah = policy.retry_after_handling.unwrap();
        assert_eq!(rah.scope, super::BlockScope::Model);
        assert_eq!(rah.apply_to, super::ApplyTo::Global);
        // max_retry_after_seconds defaults to 300
        assert_eq!(rah.max_retry_after_seconds, 300);
    }

    #[test]
    fn test_pattern13_provider_level_retry_after_blocking() {
        let yaml = r#"
    fallback_models: [anthropic/claude-3-5-sonnet]
    on_status_codes:
      - codes: [429]
        strategy: "different_provider"
        max_attempts: 2
      - codes: [503]
        strategy: "different_provider"
        max_attempts: 2
      - codes: [502]
        strategy: "different_provider"
        max_attempts: 2
    retry_after_handling:
      scope: "provider"
      apply_to: "global"
    "#;
        let policy = parse_retry_policy(yaml);
        assert_eq!(policy.on_status_codes.len(), 3);
        let rah = policy.retry_after_handling.unwrap();
        assert_eq!(rah.scope, super::BlockScope::Provider);
        assert_eq!(rah.apply_to, super::ApplyTo::Global);
        assert_eq!(rah.max_retry_after_seconds, 300);
    }

    #[test]
    fn test_pattern14_request_level_retry_after() {
        let yaml = r#"
    fallback_models: [anthropic/claude-3-5-sonnet]
    on_status_codes:
      - codes: [429]
        strategy: "different_provider"
        max_attempts: 2
      - codes: [503]
        strategy: "different_provider"
        max_attempts: 2
    retry_after_handling:
      scope: "model"
      apply_to: "request"
    "#;
        let policy = parse_retry_policy(yaml);
        let rah = policy.retry_after_handling.unwrap();
        assert_eq!(rah.scope, super::BlockScope::Model);
        assert_eq!(rah.apply_to, super::ApplyTo::Request);
        assert_eq!(rah.max_retry_after_seconds, 300);
    }

    #[test]
    fn test_pattern15_no_custom_retry_after_config_defaults_plus_backoff() {
        let yaml = r#"
    fallback_models: []
    on_status_codes:
      - codes: [429]
        strategy: "same_model"
        max_attempts: 3
      - codes: [503]
        strategy: "same_model"
        max_attempts: 3
    backoff:
      apply_to: "same_model"
      base_ms: 1000
      max_ms: 30000
      jitter: true
    "#;
        let policy = parse_retry_policy(yaml);
        // No retry_after_handling section → None
        assert!(policy.retry_after_handling.is_none());
        // But effective config should return defaults
        let effective = policy.effective_retry_after_config();
        assert_eq!(effective.scope, super::BlockScope::Model);
        assert_eq!(effective.apply_to, super::ApplyTo::Global);
        assert_eq!(effective.max_retry_after_seconds, 300);
        // Backoff is present
        let backoff = policy.backoff.unwrap();
        assert_eq!(backoff.apply_to, super::BackoffApplyTo::SameModel);
        assert_eq!(backoff.base_ms, 1000);
        assert_eq!(backoff.max_ms, 30000);
        assert!(backoff.jitter);
    }

    #[test]
    fn test_pattern16_fallback_models_list_for_targeted_failover() {
        let yaml = r#"
    fallback_models: [openai/gpt-4o-mini, anthropic/claude-3-5-sonnet, anthropic/claude-3-opus]
    default_strategy: "different_provider"
    default_max_attempts: 2
    on_status_codes:
      - codes: [429]
        strategy: "same_provider"
        max_attempts: 2
    "#;
        let policy = parse_retry_policy(yaml);
        assert_eq!(
            policy.fallback_models,
            vec![
                "openai/gpt-4o-mini",
                "anthropic/claude-3-5-sonnet",
                "anthropic/claude-3-opus",
            ]
        );
        assert_eq!(
            policy.default_strategy,
            super::RetryStrategy::DifferentProvider
        );
        assert_eq!(policy.default_max_attempts, 2);
        assert_eq!(policy.on_status_codes.len(), 1);
        assert_eq!(
            policy.on_status_codes[0].strategy,
            super::RetryStrategy::SameProvider
        );
    }

    #[test]
    fn test_backoff_without_apply_to_fails_deserialization() {
        // backoff.apply_to is a required field (no serde default), so YAML
        // without it should fail to deserialize.
        let yaml = r#"
    on_status_codes:
      - codes: [429]
        strategy: "same_model"
        max_attempts: 2
    backoff:
      base_ms: 100
      max_ms: 5000
    "#;
        let result: Result<super::RetryPolicy, _> = serde_yaml::from_str(yaml);
        assert!(
            result.is_err(),
            "backoff without apply_to should fail deserialization"
        );
    }
}
