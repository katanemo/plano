use hermesllm::apis::openai::{ModelDetail, ModelObject, Models};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Display;

use crate::api::open_ai::{
    ChatCompletionTool, FunctionDefinition, FunctionParameter, FunctionParameters, ParameterType,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Routing {
    pub model_provider: Option<String>,
    pub model: Option<String>,
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
    pub filter_chain: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Listener {
    pub name: String,
    pub router: Option<String>,
    pub agents: Option<Vec<AgentFilterChain>>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Configuration {
    pub version: String,
    pub endpoints: Option<HashMap<String, Endpoint>>,
    pub model_providers: Vec<LlmProvider>,
    pub model_aliases: Option<HashMap<String, ModelAlias>>,
    pub overrides: Option<Overrides>,
    pub system_prompt: Option<String>,
    pub prompt_guards: Option<PromptGuards>,
    pub prompt_targets: Option<Vec<PromptTarget>>,
    pub error_target: Option<ErrorTargetDetail>,
    pub ratelimits: Option<Vec<Ratelimit>>,
    pub tracing: Option<Tracing>,
    pub mode: Option<GatewayMode>,
    pub routing: Option<Routing>,
    pub agents: Option<Vec<Agent>>,
    pub filters: Option<Vec<Agent>>,
    pub listeners: Vec<Listener>,
    pub state_storage: Option<StateStorageConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Overrides {
    pub prompt_target_intent_matching_threshold: Option<f64>,
    pub optimize_context_window: Option<bool>,
    pub use_agent_orchestrator: Option<bool>,
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
    #[serde(rename = "arch")]
    Arch,
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
}

impl Display for LlmProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmProviderType::Arch => write!(f, "arch"),
            LlmProviderType::Anthropic => write!(f, "anthropic"),
            LlmProviderType::Deepseek => write!(f, "deepseek"),
            LlmProviderType::Groq => write!(f, "groq"),
            LlmProviderType::Gemini => write!(f, "gemini"),
            LlmProviderType::Mistral => write!(f, "mistral"),
            LlmProviderType::OpenAI => write!(f, "openai"),
            LlmProviderType::XAI => write!(f, "xai"),
            LlmProviderType::TogetherAI => write!(f, "together_ai"),
            LlmProviderType::AzureOpenAI => write!(f, "azure_openai"),
            LlmProviderType::Ollama => write!(f, "ollama"),
            LlmProviderType::Moonshotai => write!(f, "moonshotai"),
            LlmProviderType::Zhipu => write!(f, "zhipu"),
            LlmProviderType::Qwen => write!(f, "qwen"),
            LlmProviderType::AmazonBedrock => write!(f, "amazon_bedrock"),
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
pub struct ModelUsagePreference {
    pub model: String,
    pub routing_preferences: Vec<RoutingPreference>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingPreference {
    pub name: String,
    pub description: String,
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
                let start: u16 = parts[0].trim().parse().map_err(|_| {
                    format!("Invalid start in status code range: '{}'", parts[0])
                })?;
                let end: u16 = parts[1].trim().parse().map_err(|_| {
                    format!("Invalid end in status code range: '{}'", parts[1])
                })?;
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
    pub routing_preferences: Option<Vec<RoutingPreference>>,
    pub cluster_name: Option<String>,
    pub base_url_path_prefix: Option<String>,
    pub internal: Option<bool>,
    pub passthrough_auth: Option<bool>,
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
            routing_preferences: None,
            cluster_name: None,
            base_url_path_prefix: None,
            internal: None,
            passthrough_auth: None,
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

