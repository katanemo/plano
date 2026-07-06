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
#[serde(rename_all = "snake_case")]
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
    /// Optional override for the pricing catalog endpoint. When omitted, a
    /// sensible default is used per provider.
    pub url: Option<String>,
    pub refresh_interval: Option<u64>,
    /// Map catalog keys to Plano model names used in `routing_preferences`.
    /// DigitalOcean keys look like `lowercase(creator)/model_id`; models.dev
    /// keys look like `creator/model_id`.
    /// Example: `openai/openai-gpt-oss-120b: openai/gpt-4o`
    pub model_aliases: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostProvider {
    Digitalocean,
    #[serde(rename = "models.dev")]
    ModelsDev,
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
    /// Automatic provider prompt caching. Disabled by default; opt in globally with
    /// `prompt_caching: { enabled: true }`. Applies across the entire Plano instance
    /// and never changes which model routing selects.
    pub prompt_caching: Option<PromptCaching>,
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

/// Automatic prompt caching, configured once for the whole Plano instance.
///
/// Prompt caching keeps a multi-turn conversation's stable prefix warm in the
/// upstream provider's cache. It never influences which model routing selects — it
/// only (a) auto-injects provider cache-control markers where supported and
/// (b) derives an implicit session key from the stable prompt prefix so follow-up
/// turns reuse the same warm cache. An explicit `X-Model-Affinity` header always wins.
///
/// Disabled by default; opt in with `enabled: true`. The remaining knobs are optional
/// tuning that only take effect while caching is enabled.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PromptCaching {
    /// Master switch. Defaults to `false` (opt-in).
    #[serde(default)]
    pub enabled: bool,
    /// Derive an implicit session key from the stable prompt prefix so caches survive
    /// across turns without client changes. Defaults to `true` when caching is enabled.
    pub session_affinity: Option<bool>,
    /// Auto-inject provider cache-control markers (e.g. Anthropic `cache_control`).
    /// Defaults to `true` when caching is enabled.
    pub inject_cache_control: Option<bool>,
    /// Minimum estimated prefix tokens before a cache breakpoint is injected.
    pub min_prefix_tokens: Option<u32>,
    /// Session pin TTL; falls back to `routing.session_ttl_seconds` when unset.
    pub session_ttl_seconds: Option<u64>,
}

/// Fully-resolved, instance-wide prompt-caching settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectivePromptCaching {
    pub enabled: bool,
    pub session_affinity: bool,
    pub inject_cache_control: bool,
    pub min_prefix_tokens: u32,
    /// Pin TTL override; `None` uses `routing.session_ttl_seconds`.
    pub session_ttl_seconds: Option<u64>,
}

pub const DEFAULT_MIN_PREFIX_TOKENS: u32 = 1024;

impl Default for EffectivePromptCaching {
    fn default() -> Self {
        EffectivePromptCaching {
            enabled: false,
            session_affinity: false,
            inject_cache_control: false,
            min_prefix_tokens: DEFAULT_MIN_PREFIX_TOKENS,
            session_ttl_seconds: None,
        }
    }
}

impl PromptCaching {
    /// Resolve the instance-wide effective settings. When caching is disabled every
    /// sub-feature is off, regardless of the individual knobs.
    pub fn resolve(&self) -> EffectivePromptCaching {
        if !self.enabled {
            return EffectivePromptCaching::default();
        }
        EffectivePromptCaching {
            enabled: true,
            session_affinity: self.session_affinity.unwrap_or(true),
            inject_cache_control: self.inject_cache_control.unwrap_or(true),
            min_prefix_tokens: self.min_prefix_tokens.unwrap_or(DEFAULT_MIN_PREFIX_TOKENS),
            session_ttl_seconds: self.session_ttl_seconds,
        }
    }
}

impl EffectivePromptCaching {
    /// Resolve from an optional config block; `None` means caching is off.
    pub fn from_config(config: Option<&PromptCaching>) -> Self {
        config.map(PromptCaching::resolve).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Tracing {
    pub sampling_rate: Option<f64>,
    pub trace_arch_internal: Option<bool>,
    pub random_sampling: Option<u32>,
    pub opentracing_grpc_endpoint: Option<String>,
    pub span_attributes: Option<SpanAttributes>,
    /// Provider-agnostic telemetry export destinations. Each entry is tagged by
    /// its `type` (e.g. `posthog`) so new backends can be added without breaking
    /// existing configs. LLM spans are translated into each backend's native
    /// event format and streamed in addition to any `opentracing_grpc_endpoint`.
    pub exporters: Option<Vec<Exporter>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SpanAttributes {
    pub header_prefixes: Option<Vec<String>>,
    #[serde(rename = "static")]
    pub static_attributes: Option<HashMap<String, String>>,
}

/// A telemetry export destination configured under `tracing.exporters`.
///
/// The list is provider-agnostic; each variant is internally tagged by its
/// `type` field (e.g. `type: posthog`). Additional backends (datadog, raw
/// otlp, ...) can be added as new variants without breaking existing configs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Exporter {
    /// PostHog AI observability. LLM spans are converted into PostHog
    /// `$ai_generation` events and POSTed to the configured `url`.
    Posthog(PosthogExporter),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PosthogExporter {
    /// PostHog host, e.g. `https://us.i.posthog.com`. The `/batch/` capture
    /// path is appended automatically.
    pub url: String,
    /// PostHog project API key (token). Supports `$ENV_VAR` expansion at render
    /// time, e.g. `$POSTHOG_API_KEY`.
    pub api_key: String,
    /// Optional request header whose value is used as the PostHog `distinct_id`.
    /// When unset (or the header is missing on a request) events are captured
    /// anonymously.
    pub distinct_id_header: Option<String>,
    /// When true, include the truncated user message preview as `$ai_input`.
    /// Defaults to `false` to avoid sending prompt content off-box.
    pub capture_messages: Option<bool>,
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
    #[serde(rename = "astraflow")]
    Astraflow,
    #[serde(rename = "astraflow_cn")]
    AstraflowCN,
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
            LlmProviderType::Astraflow => write!(f, "astraflow"),
            LlmProviderType::AstraflowCN => write!(f, "astraflow_cn"),
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

    use super::{
        EffectivePromptCaching, IntoModels, LlmProvider, LlmProviderType, PromptCaching,
        DEFAULT_MIN_PREFIX_TOKENS,
    };
    use crate::api::open_ai::ToolType;

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

    #[test]
    fn test_deserialize_models_dev_cost_source() {
        let yaml = r#"
- type: cost
  provider: models.dev
  url: https://models.dev/api.json
  refresh_interval: 3600
  model_aliases:
    openai/gpt-oss-120b: openai/gpt-4o
"#;
        let sources: Vec<super::MetricsSource> = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(sources.len(), 1);
        match &sources[0] {
            super::MetricsSource::Cost(cfg) => {
                assert!(matches!(cfg.provider, super::CostProvider::ModelsDev));
                assert_eq!(cfg.url.as_deref(), Some("https://models.dev/api.json"));
                assert_eq!(cfg.refresh_interval, Some(3600));
                assert_eq!(
                    cfg.model_aliases
                        .as_ref()
                        .and_then(|m| m.get("openai/gpt-oss-120b"))
                        .map(String::as_str),
                    Some("openai/gpt-4o")
                );
            }
            other => panic!("expected cost source, got {other:?}"),
        }
    }

    #[test]
    fn test_deserialize_digitalocean_cost_source_without_url() {
        let yaml = r#"
- type: cost
  provider: digitalocean
"#;
        let sources: Vec<super::MetricsSource> = serde_yaml::from_str(yaml).unwrap();
        match &sources[0] {
            super::MetricsSource::Cost(cfg) => {
                assert!(matches!(cfg.provider, super::CostProvider::Digitalocean));
                assert_eq!(cfg.url, None);
            }
            other => panic!("expected cost source, got {other:?}"),
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

    #[test]
    fn test_prompt_caching_disabled_by_default() {
        // Absent config → everything off.
        let effective = EffectivePromptCaching::from_config(None);
        assert!(!effective.enabled);
        assert!(!effective.session_affinity);
        assert!(!effective.inject_cache_control);

        // Present but not enabled → still off.
        let cfg: PromptCaching = serde_yaml::from_str("enabled: false").unwrap();
        let effective = cfg.resolve();
        assert!(!effective.enabled);
        assert!(!effective.session_affinity);
        assert!(!effective.inject_cache_control);
    }

    #[test]
    fn test_prompt_caching_enabled_defaults() {
        // A bare `enabled: true` turns everything on with sensible defaults.
        let cfg: PromptCaching = serde_yaml::from_str("enabled: true").unwrap();
        let effective = cfg.resolve();
        assert!(effective.enabled);
        assert!(effective.session_affinity);
        assert!(effective.inject_cache_control);
        assert_eq!(effective.min_prefix_tokens, DEFAULT_MIN_PREFIX_TOKENS);
        assert_eq!(effective.session_ttl_seconds, None);
    }

    #[test]
    fn test_prompt_caching_optional_knobs() {
        let yaml = r#"
enabled: true
session_affinity: false
inject_cache_control: false
min_prefix_tokens: 2048
session_ttl_seconds: 3600
"#;
        let cfg: PromptCaching = serde_yaml::from_str(yaml).unwrap();
        let effective = cfg.resolve();
        assert!(effective.enabled);
        assert!(!effective.session_affinity);
        assert!(!effective.inject_cache_control);
        assert_eq!(effective.min_prefix_tokens, 2048);
        assert_eq!(effective.session_ttl_seconds, Some(3600));
    }

    #[test]
    fn test_prompt_caching_knobs_ignored_when_disabled() {
        // Knobs only take effect while caching is enabled.
        let yaml = r#"
enabled: false
session_affinity: true
inject_cache_control: true
"#;
        let cfg: PromptCaching = serde_yaml::from_str(yaml).unwrap();
        let effective = cfg.resolve();
        assert!(!effective.enabled);
        assert!(!effective.session_affinity);
        assert!(!effective.inject_cache_control);
    }

    #[test]
    fn test_tracing_posthog_exporter_deserialize() {
        let yaml = r#"
random_sampling: 100
exporters:
  - type: posthog
    url: https://us.i.posthog.com
    api_key: phc_secret
    distinct_id_header: x-user-id
    capture_messages: true
"#;
        let tracing: super::Tracing = serde_yaml::from_str(yaml).unwrap();
        let exporters = tracing.exporters.expect("exporters should be parsed");
        assert_eq!(exporters.len(), 1);
        match &exporters[0] {
            super::Exporter::Posthog(posthog) => {
                assert_eq!(posthog.url, "https://us.i.posthog.com");
                assert_eq!(posthog.api_key, "phc_secret");
                assert_eq!(posthog.distinct_id_header.as_deref(), Some("x-user-id"));
                assert_eq!(posthog.capture_messages, Some(true));
            }
        }
    }

    #[test]
    fn test_tracing_posthog_exporter_minimal() {
        let yaml = r#"
exporters:
  - type: posthog
    url: https://eu.i.posthog.com
    api_key: phc_eu
"#;
        let tracing: super::Tracing = serde_yaml::from_str(yaml).unwrap();
        let exporters = tracing.exporters.unwrap();
        match &exporters[0] {
            super::Exporter::Posthog(posthog) => {
                assert_eq!(posthog.url, "https://eu.i.posthog.com");
                assert_eq!(posthog.distinct_id_header, None);
                assert_eq!(posthog.capture_messages, None);
            }
        }
    }
}
