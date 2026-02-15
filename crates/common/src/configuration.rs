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

    #[test]
    fn test_deserialize_configuration() {
        let ref_config = fs::read_to_string(
            "../../docs/source/resources/includes/plano_config_full_reference_rendered.yaml",
        )
        .expect("reference config file not found");

        let config: super::Configuration = serde_yaml::from_str(&ref_config).unwrap();
        assert_eq!(config.version, "v0.3.0");

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
                name: "arch-router".to_string(),
                provider_interface: LlmProviderType::Arch,
                model: Some("Arch-Router".to_string()),
                internal: Some(true),
                ..Default::default()
            },
            LlmProvider {
                name: "plano-orchestrator".to_string(),
                provider_interface: LlmProviderType::Arch,
                model: Some("Plano-Orchestrator".to_string()),
                internal: Some(true),
                ..Default::default()
            },
        ];

        let models = providers.into_models();

        // Should only have 1 model: openai-gpt4
        assert_eq!(models.data.len(), 1);

        // Verify internal models are excluded from /v1/models
        let model_ids: Vec<String> = models.data.iter().map(|m| m.id.clone()).collect();
        assert!(model_ids.contains(&"openai-gpt4".to_string()));
        assert!(!model_ids.contains(&"arch-router".to_string()));
        assert!(!model_ids.contains(&"plano-orchestrator".to_string()));
    }
}
