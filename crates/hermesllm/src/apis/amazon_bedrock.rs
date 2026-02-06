use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_with::skip_serializing_none;

use std::collections::HashMap;
use thiserror::Error;

use super::ApiDefinition;
use crate::providers::request::{ProviderRequest, ProviderRequestError};
use crate::providers::streaming_response::ProviderStreamResponse;

// ============================================================================
// AMAZON BEDROCK CONVERSE API ENUMERATION
// ============================================================================

/// Enum for all supported Amazon Bedrock Converse APIs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AmazonBedrockApi {
    Converse,
    ConverseStream,
}

impl ApiDefinition for AmazonBedrockApi {
    fn endpoint(&self) -> &'static str {
        match self {
            AmazonBedrockApi::Converse => "/model/{modelId}/converse",
            AmazonBedrockApi::ConverseStream => "/model/{modelId}/converse-stream",
        }
    }

    fn from_endpoint(endpoint: &str) -> Option<Self> {
        if endpoint.ends_with("/converse") {
            Some(AmazonBedrockApi::Converse)
        } else if endpoint.ends_with("/converse-stream") {
            Some(AmazonBedrockApi::ConverseStream)
        } else {
            None
        }
    }

    fn supports_streaming(&self) -> bool {
        match self {
            AmazonBedrockApi::Converse => false,
            AmazonBedrockApi::ConverseStream => true,
        }
    }

    fn supports_tools(&self) -> bool {
        // Converse API has native tool support
        true
    }

    fn supports_vision(&self) -> bool {
        // Converse API has native vision support
        true
    }

    fn all_variants() -> Vec<Self> {
        vec![AmazonBedrockApi::Converse, AmazonBedrockApi::ConverseStream]
    }
}

// ============================================================================
// CONVERSE REQUEST STRUCTURES
// ============================================================================

/// Amazon Bedrock Converse request
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ConverseRequest {
    /// The model ID or ARN to invoke
    pub model_id: String,
    /// The messages to send to the model
    pub messages: Option<Vec<Message>>,
    /// System prompts that provide instructions or context
    pub system: Option<Vec<SystemContentBlock>>,
    /// Inference configuration
    #[serde(rename = "inferenceConfig")]
    pub inference_config: Option<InferenceConfiguration>,
    /// Tool configuration for function calling
    #[serde(rename = "toolConfig")]
    pub tool_config: Option<ToolConfiguration>,
    /// Guardrail configuration
    #[serde(rename = "guardrailConfig")]
    pub guardrail_config: Option<GuardrailConfiguration>,
    /// Additional model-specific request fields
    #[serde(rename = "additionalModelRequestFields")]
    pub additional_model_request_fields: Option<Value>,
    /// Additional model response field paths to return
    #[serde(rename = "additionalModelResponseFieldPaths")]
    pub additional_model_response_field_paths: Option<Vec<String>>,
    /// Performance configuration
    #[serde(rename = "performanceConfig")]
    pub performance_config: Option<InferenceConfiguration>,
    /// Prompt variables for Prompt management
    #[serde(rename = "promptVariables")]
    pub prompt_variables: Option<HashMap<String, PromptVariableValues>>,
    /// Request metadata for filtering logs
    #[serde(rename = "requestMetadata")]
    pub request_metadata: Option<HashMap<String, String>>,
    /// Additional custom metadata (for internal use)
    pub metadata: Option<HashMap<String, Value>>,
    /// Whether this request should use streaming endpoint (internal field, not serialized)
    #[serde(skip)]
    pub stream: bool,
}

/// Amazon Bedrock ConverseStream request (same structure as Converse)
pub type ConverseStreamRequest = ConverseRequest;

impl ProviderRequest for ConverseRequest {
    fn model(&self) -> &str {
        &self.model_id
    }

    fn set_model(&mut self, model: String) {
        self.model_id = model;
    }

    fn is_streaming(&self) -> bool {
        self.stream
    }

    fn extract_messages_text(&self) -> String {
        let mut text_parts = Vec::new();

        // Extract text from messages
        if let Some(messages) = &self.messages {
            for message in messages {
                for content_block in &message.content {
                    match content_block {
                        ContentBlock::Text { text } => {
                            text_parts.push(text.clone());
                        }
                        ContentBlock::GuardContent { guard_content } => {
                            if let Some(guard_text) = &guard_content.text {
                                text_parts.push(guard_text.text.clone());
                            }
                        }
                        _ => {} // Skip non-text content blocks
                    }
                }
            }
        }

        // Extract text from system prompts
        if let Some(system) = &self.system {
            for system_block in system {
                match system_block {
                    SystemContentBlock::Text { text } => {
                        text_parts.push(text.clone());
                    }
                    SystemContentBlock::GuardContent {
                        text: Some(guard_text),
                    } => {
                        text_parts.push(guard_text.text.clone());
                    }
                    SystemContentBlock::GuardContent { text: None } => {
                        // No text content in this guard content block
                    }
                }
            }
        }

        text_parts.join(" ")
    }

    fn get_recent_user_message(&self) -> Option<String> {
        self.messages
            .as_ref()?
            .iter()
            .rev() // Start from the most recent message
            .find(|msg| msg.role == ConversationRole::User)
            .and_then(|msg| {
                // Extract the first text content block from the user message
                msg.content.iter().find_map(|content| match content {
                    ContentBlock::Text { text } => Some(text.clone()),
                    _ => None,
                })
            })
    }

    fn get_tool_names(&self) -> Option<Vec<String>> {
        self.tool_config.as_ref()?.tools.as_ref().map(|tools| {
            tools
                .iter()
                .map(|tool| match tool {
                    Tool::ToolSpec { tool_spec } => tool_spec.name.clone(),
                })
                .collect()
        })
    }

    fn to_bytes(&self) -> Result<Vec<u8>, ProviderRequestError> {
        serde_json::to_vec(self).map_err(|e| ProviderRequestError {
            message: format!("Failed to serialize Bedrock request: {}", e),
            source: Some(Box::new(e)),
        })
    }

    fn metadata(&self) -> &Option<HashMap<String, Value>> {
        &self.metadata
    }

    fn remove_metadata_key(&mut self, key: &str) -> bool {
        if let Some(ref mut metadata) = self.metadata {
            metadata.remove(key).is_some()
        } else {
            false
        }
    }

    fn get_temperature(&self) -> Option<f32> {
        self.inference_config.as_ref()?.temperature
    }

    fn get_messages(&self) -> Vec<crate::apis::openai::Message> {
        use crate::apis::openai::{Message, MessageContent, Role};

        let mut openai_messages = Vec::new();

        // Add system messages if present
        if let Some(system) = &self.system {
            for sys_block in system {
                if let SystemContentBlock::Text { text } = sys_block {
                    openai_messages.push(Message {
                        role: Role::System,
                        content: Some(MessageContent::Text(text.clone())),
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
                    });
                }
            }
        }

        // Convert conversation messages
        if let Some(messages) = &self.messages {
            for msg in messages {
                let role = match msg.role {
                    ConversationRole::User => Role::User,
                    ConversationRole::Assistant => Role::Assistant,
                };

                // Extract text from content blocks
                let content = msg
                    .content
                    .iter()
                    .filter_map(|block| {
                        if let ContentBlock::Text { text } = block {
                            Some(text.clone())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");

                openai_messages.push(Message {
                    role,
                    content: Some(MessageContent::Text(content)),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                });
            }
        }

        openai_messages
    }

    fn set_messages(&mut self, messages: &[crate::apis::openai::Message]) {
        // Convert OpenAI messages to Bedrock format
        use crate::apis::amazon_bedrock::{ContentBlock, ConversationRole, SystemContentBlock};

        let mut system_blocks = Vec::new();
        let mut bedrock_messages = Vec::new();

        for msg in messages {
            match msg.role {
                crate::apis::openai::Role::System => {
                    if let Some(crate::apis::openai::MessageContent::Text(text)) = &msg.content {
                        system_blocks.push(SystemContentBlock::Text { text: text.clone() });
                    }
                }
                crate::apis::openai::Role::User | crate::apis::openai::Role::Assistant => {
                    let role = match msg.role {
                        crate::apis::openai::Role::User => ConversationRole::User,
                        crate::apis::openai::Role::Assistant => ConversationRole::Assistant,
                        _ => continue,
                    };

                    let content = if let Some(crate::apis::openai::MessageContent::Text(text)) =
                        &msg.content
                    {
                        vec![ContentBlock::Text { text: text.clone() }]
                    } else {
                        vec![]
                    };

                    bedrock_messages.push(crate::apis::amazon_bedrock::Message { role, content });
                }
                _ => {}
            }
        }

        if !system_blocks.is_empty() {
            self.system = Some(system_blocks);
        }
        self.messages = Some(bedrock_messages);
    }
}

// ============================================================================
// CONVERSE RESPONSE STRUCTURES
// ============================================================================

/// Amazon Bedrock Converse response
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConverseResponse {
    /// The result from the call to Converse
    pub output: ConverseOutput,
    /// The reason why the model stopped generating output
    #[serde(rename = "stopReason")]
    pub stop_reason: StopReason,
    /// Token usage information
    pub usage: BedrockTokenUsage,
    /// Metrics for the call
    pub metrics: Option<ConverseMetrics>,
    /// Additional model response fields
    #[serde(rename = "additionalModelResponseFields")]
    pub additional_model_response_fields: Option<Value>,
    /// Performance configuration used
    #[serde(rename = "performanceConfig")]
    pub performance_config: Option<PerformanceConfiguration>,
    /// Trace information for guardrails
    pub trace: Option<ConverseTrace>,
}

/// Amazon Bedrock ConverseStream response events
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum ConverseStreamEvent {
    MessageStart(MessageStartEvent),
    ContentBlockStart(ContentBlockStartEvent),
    ContentBlockDelta(ContentBlockDeltaEvent),
    ContentBlockStop(ContentBlockStopEvent),
    MessageStop(MessageStopEvent),
    Metadata(Box<ConverseStreamMetadataEvent>),
    // Error events
    InternalServerException(BedrockException),
    ModelStreamErrorException(BedrockException),
    ServiceUnavailableException(BedrockException),
    ThrottlingException(BedrockException),
    ValidationException(BedrockException),
}

// ============================================================================
// MESSAGE AND CONTENT STRUCTURES
// ============================================================================

/// Message in a conversation
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Message {
    /// Role of the message sender (user, assistant)
    pub role: ConversationRole,
    /// Content blocks in the message
    pub content: Vec<ContentBlock>,
}

/// Conversation role enumeration
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConversationRole {
    User,
    Assistant,
}

/// Content block in a message
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Image {
        image: ImageBlock,
    },
    Document {
        document: DocumentBlock,
    },
    ToolUse {
        #[serde(rename = "toolUse")]
        tool_use: ToolUseBlock,
    },
    ToolResult {
        #[serde(rename = "toolResult")]
        tool_result: ToolResultBlock,
    },
    GuardContent {
        #[serde(rename = "guardContent")]
        guard_content: GuardContentBlock,
    },
}

/// Image block structure
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ImageBlock {
    pub source: ImageSource,
}

/// Document block structure
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DocumentBlock {
    pub source: DocumentSource,
    pub name: Option<String>,
}

/// Tool use block structure
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ToolUseBlock {
    #[serde(rename = "toolUseId")]
    pub tool_use_id: String,
    pub name: String,
    pub input: Value,
}

/// Tool result block structure
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ToolResultBlock {
    #[serde(rename = "toolUseId")]
    pub tool_use_id: String,
    pub content: Vec<ToolResultContentBlock>,
    pub status: Option<ToolResultStatus>,
}

/// Guard content block structure
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GuardContentBlock {
    pub text: Option<GuardContentText>,
}

/// System content block for system prompts
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum SystemContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "guardContent")]
    GuardContent { text: Option<GuardContentText> },
}

/// Image source for vision capabilities
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum ImageSource {
    #[serde(rename = "base64")]
    Base64 {
        #[serde(rename = "mediaType")]
        media_type: String,
        data: String,
    },
}

/// Document source for document processing
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum DocumentSource {
    #[serde(rename = "base64")]
    Base64 {
        #[serde(rename = "mediaType")]
        media_type: String,
        data: String,
    },
}

/// Tool result content block
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum ToolResultContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { source: ImageSource },
    #[serde(rename = "json")]
    Json { json: Value },
}

/// Tool result status
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ToolResultStatus {
    Success,
    Error,
}

/// Guard content text with qualifiers
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GuardContentText {
    pub text: String,
    pub qualifiers: Option<Vec<GuardContentQualifier>>,
}

/// Guard content qualifier
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GuardContentQualifier {
    Grounding,
    Relevance,
    Harmfulness,
    Helpfulness,
}

// ============================================================================
// INFERENCE AND TOOL CONFIGURATION
// ============================================================================

/// Inference configuration for the model
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct InferenceConfiguration {
    /// Maximum tokens to generate
    #[serde(rename = "maxTokens")]
    pub max_tokens: Option<u32>,
    /// Temperature for randomness (0.0 to 1.0)
    pub temperature: Option<f32>,
    /// Top-p sampling parameter (0.0 to 1.0)
    #[serde(rename = "topP")]
    pub top_p: Option<f32>,
    /// Stop sequences to halt generation
    #[serde(rename = "stopSequences")]
    pub stop_sequences: Option<Vec<String>>,
}

/// Tool configuration for function calling
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ToolConfiguration {
    /// Available tools for the model
    pub tools: Option<Vec<Tool>>,
    /// Tool choice configuration
    #[serde(rename = "toolChoice")]
    pub tool_choice: Option<ToolChoice>,
}

/// Tool definition
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum Tool {
    ToolSpec {
        #[serde(rename = "toolSpec")]
        tool_spec: ToolSpecDefinition,
    },
}

/// Tool specification definition
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ToolSpecDefinition {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: ToolInputSchema,
}

/// Tool input schema
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ToolInputSchema {
    pub json: Value,
}

/// Tool choice configuration
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum ToolChoice {
    Auto {
        #[serde(rename = "auto")]
        auto: AutoChoice,
    },
    Any {
        #[serde(rename = "any")]
        any: AnyChoice,
    },
    Tool {
        #[serde(rename = "tool")]
        tool: ToolChoiceSpec,
    },
}

/// Auto tool choice (empty object)
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AutoChoice {}

/// Any tool choice (empty object)
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AnyChoice {}

/// Specific tool choice
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ToolChoiceSpec {
    pub name: String,
}

// ============================================================================
// GUARDRAIL CONFIGURATION
// ============================================================================

/// Guardrail configuration for content filtering
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GuardrailConfiguration {
    /// Guardrail identifier
    #[serde(rename = "guardrailIdentifier")]
    pub guardrail_identifier: String,
    /// Guardrail version
    #[serde(rename = "guardrailVersion")]
    pub guardrail_version: String,
    /// Trace setting
    pub trace: Option<GuardrailTrace>,
}

/// Guardrail configuration for streaming (has additional field)
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GuardrailStreamConfiguration {
    /// Guardrail identifier
    #[serde(rename = "guardrailIdentifier")]
    pub guardrail_identifier: String,
    /// Guardrail version
    #[serde(rename = "guardrailVersion")]
    pub guardrail_version: String,
    /// Stream processing mode
    #[serde(rename = "streamProcessingMode")]
    pub stream_processing_mode: Option<String>,
    /// Trace setting
    pub trace: Option<GuardrailTrace>,
}

/// Guardrail trace setting
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum GuardrailTrace {
    Enabled,
    Disabled,
}

// ============================================================================
// PERFORMANCE CONFIGURATION
// ============================================================================

/// Performance configuration for latency optimization
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PerformanceConfiguration {
    /// Latency optimization setting
    pub latency: Option<PerformanceLatency>,
}

/// Performance latency setting
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PerformanceLatency {
    Standard,
    Optimized,
}

// ============================================================================
// RESPONSE OUTPUT STRUCTURES
// ============================================================================

/// Converse output (union type)
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum ConverseOutput {
    Message { message: Message },
}

/// Stop reason enumeration
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    StopSequence,
    GuardrailIntervened,
    ContentFiltered,
}

/// Token usage information for Bedrock Converse API
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct BedrockTokenUsage {
    /// Input tokens processed
    #[serde(rename = "inputTokens")]
    pub input_tokens: u32,
    /// Output tokens generated
    #[serde(rename = "outputTokens")]
    pub output_tokens: u32,
    /// Total tokens used
    #[serde(rename = "totalTokens")]
    pub total_tokens: u32,
    /// Server tool usage (for function calling)
    #[serde(rename = "serverToolUsage")]
    pub server_tool_usage: Option<Value>,
    /// Cache read input tokens
    #[serde(rename = "cacheReadInputTokens")]
    pub cache_read_input_tokens: Option<u32>,
    /// Cache write input tokens
    #[serde(rename = "cacheWriteInputTokens")]
    pub cache_write_input_tokens: Option<u32>,
}

/// Converse metrics
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConverseMetrics {
    /// Latency in milliseconds
    #[serde(rename = "latencyMs")]
    pub latency_ms: u64,
}

/// Converse trace information
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConverseTrace {
    /// Guardrail trace information
    pub guardrail: Option<GuardrailTraceAssessment>,
    /// Prompt router trace information
    #[serde(rename = "promptRouter")]
    pub prompt_router: Option<PromptRouterTrace>,
}

/// Guardrail trace assessment (simplified)
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GuardrailTraceAssessment {
    /// Action reason
    #[serde(rename = "actionReason")]
    pub action_reason: Option<String>,
    /// Model output
    #[serde(rename = "modelOutput")]
    pub model_output: Option<Vec<String>>,
    /// Input assessment
    #[serde(rename = "inputAssessment")]
    pub input_assessment: Option<HashMap<String, Value>>,
    /// Output assessments
    #[serde(rename = "outputAssessments")]
    pub output_assessments: Option<HashMap<String, Vec<Value>>>,
}

/// Prompt router trace
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PromptRouterTrace {
    /// Invoked model ID
    #[serde(rename = "invokedModelId")]
    pub invoked_model_id: String,
}

// ============================================================================
// STREAMING EVENT STRUCTURES
// ============================================================================

/// Message start event
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MessageStartEvent {
    /// Role of the message
    pub role: ConversationRole,
}

/// Content block start event
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ContentBlockStartEvent {
    /// Content block index
    #[serde(rename = "contentBlockIndex")]
    pub content_block_index: i32,
    /// Start information
    pub start: ContentBlockStart,
}

/// Content block start information
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum ContentBlockStart {
    ToolUse {
        #[serde(rename = "toolUse")]
        tool_use: ToolUseStart,
    },
}

/// Tool use start information
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ToolUseStart {
    #[serde(rename = "toolUseId")]
    pub tool_use_id: String,
    pub name: String,
}

/// Content block delta event
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ContentBlockDeltaEvent {
    /// Content block index
    #[serde(rename = "contentBlockIndex")]
    pub content_block_index: i32,
    /// Delta information
    pub delta: ContentBlockDelta,
}

/// Content block delta information
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum ContentBlockDelta {
    Text {
        text: String,
    },
    ToolUse {
        #[serde(rename = "toolUse")]
        tool_use: ToolUseDelta,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ToolUseDelta {
    pub input: String,
}

/// Content block stop event
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ContentBlockStopEvent {
    /// Content block index
    #[serde(rename = "contentBlockIndex")]
    pub content_block_index: i32,
}

/// Message stop event
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MessageStopEvent {
    /// Stop reason
    #[serde(rename = "stopReason")]
    pub stop_reason: StopReason,
    /// Additional model response fields
    #[serde(rename = "additionalModelResponseFields")]
    pub additional_model_response_fields: Option<Value>,
}

/// Stream metadata event
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConverseStreamMetadataEvent {
    /// Token usage
    pub usage: BedrockTokenUsage,
    /// Stream metrics
    pub metrics: Option<ConverseStreamMetrics>,
    /// Trace information
    pub trace: Option<ConverseStreamTrace>,
    /// Performance configuration
    #[serde(rename = "performanceConfig")]
    pub performance_config: Option<PerformanceConfiguration>,
}

/// Stream metrics
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConverseStreamMetrics {
    /// Latency in milliseconds
    #[serde(rename = "latencyMs")]
    pub latency_ms: u64,
}

/// Stream trace information
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConverseStreamTrace {
    /// Guardrail trace
    pub guardrail: Option<GuardrailTraceAssessment>,
    /// Prompt router trace
    #[serde(rename = "promptRouter")]
    pub prompt_router: Option<PromptRouterTrace>,
}

// ============================================================================
// PROMPT MANAGEMENT
// ============================================================================

/// Prompt variable values for Prompt management
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum PromptVariableValues {
    Text { text: String },
}

// ============================================================================
// ERROR TYPES
// ============================================================================

/// Bedrock exception structure
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BedrockException {
    /// Exception message
    pub message: Option<String>,
    /// Original status code (for model errors)
    #[serde(rename = "originalStatusCode")]
    pub original_status_code: Option<u16>,
    /// Resource name (for model errors)
    #[serde(rename = "resourceName")]
    pub resource_name: Option<String>,
    /// Original message (for stream errors)
    #[serde(rename = "originalMessage")]
    pub original_message: Option<String>,
}

/// Bedrock-specific error types
#[derive(Error, Debug)]
pub enum BedrockError {
    #[error("Access denied: {message}")]
    AccessDenied { message: String },

    #[error("Internal server error: {message}")]
    InternalServer { message: String },

    #[error("Model error: {message}")]
    ModelError {
        message: String,
        original_status_code: Option<u16>,
        resource_name: Option<String>,
    },

    #[error("Model not ready: {message}")]
    ModelNotReady { message: String },

    #[error("Model timeout: {message}")]
    ModelTimeout { message: String },

    #[error("Resource not found: {message}")]
    ResourceNotFound { message: String },

    #[error("Service unavailable: {message}")]
    ServiceUnavailable { message: String },

    #[error("Throttling: {message}")]
    Throttling { message: String },

    #[error("Validation error: {message}")]
    Validation { message: String },

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

// ============================================================================
// TRAIT IMPLEMENTATIONS
// ============================================================================

// Note: Trait implementations will be added later when we implement transformations
// For now, we're focusing on modeling the request/response shapes

impl crate::providers::response::TokenUsage for BedrockTokenUsage {
    fn completion_tokens(&self) -> usize {
        self.output_tokens as usize
    }

    fn prompt_tokens(&self) -> usize {
        self.input_tokens as usize
    }

    fn total_tokens(&self) -> usize {
        self.total_tokens as usize
    }
}

// ============================================================================
// EVENT STREAM PARSING
// ============================================================================

/// Convert from aws-smithy-eventstream DecodedFrame to ConverseStreamEvent
impl TryFrom<&aws_smithy_eventstream::frame::DecodedFrame> for ConverseStreamEvent {
    type Error = BedrockError;

    fn try_from(frame: &aws_smithy_eventstream::frame::DecodedFrame) -> Result<Self, Self::Error> {
        // Only process Complete frames, skip Incomplete
        let message = match frame {
            aws_smithy_eventstream::frame::DecodedFrame::Complete(msg) => msg,
            aws_smithy_eventstream::frame::DecodedFrame::Incomplete => {
                return Err(BedrockError::Validation {
                    message: "Expected Complete frame, got Incomplete".to_string(),
                })
            }
        };

        // Extract the :event-type and :message-type headers
        let event_type = message
            .headers()
            .iter()
            .find(|h| h.name().as_str() == ":event-type")
            .and_then(|h| h.value().as_string().ok())
            .ok_or_else(|| BedrockError::Validation {
                message: "Missing :event-type header".to_string(),
            })?
            .as_str();

        let message_type = message
            .headers()
            .iter()
            .find(|h| h.name().as_str() == ":message-type")
            .and_then(|h| h.value().as_string().ok())
            .ok_or_else(|| BedrockError::Validation {
                message: "Missing :message-type header".to_string(),
            })?
            .as_str();

        let payload = message.payload();

        // Parse the event based on message type and event type
        match message_type {
            "event" => match event_type {
                "messageStart" => {
                    let event: MessageStartEvent =
                        serde_json::from_slice(payload).map_err(BedrockError::Serialization)?;
                    Ok(ConverseStreamEvent::MessageStart(event))
                }
                "contentBlockStart" => {
                    let event: ContentBlockStartEvent =
                        serde_json::from_slice(payload).map_err(BedrockError::Serialization)?;
                    Ok(ConverseStreamEvent::ContentBlockStart(event))
                }
                "contentBlockDelta" => {
                    let event: ContentBlockDeltaEvent =
                        serde_json::from_slice(payload).map_err(BedrockError::Serialization)?;
                    Ok(ConverseStreamEvent::ContentBlockDelta(event))
                }
                "contentBlockStop" => {
                    let event: ContentBlockStopEvent =
                        serde_json::from_slice(payload).map_err(BedrockError::Serialization)?;
                    Ok(ConverseStreamEvent::ContentBlockStop(event))
                }
                "messageStop" => {
                    let event: MessageStopEvent =
                        serde_json::from_slice(payload).map_err(BedrockError::Serialization)?;
                    Ok(ConverseStreamEvent::MessageStop(event))
                }
                "metadata" => {
                    let event: ConverseStreamMetadataEvent =
                        serde_json::from_slice(payload).map_err(BedrockError::Serialization)?;
                    Ok(ConverseStreamEvent::Metadata(Box::new(event)))
                }
                unknown => Err(BedrockError::Validation {
                    message: format!("Unknown event type: {}", unknown),
                }),
            },
            "exception" => match event_type {
                "internalServerException" => {
                    let exception: BedrockException =
                        serde_json::from_slice(payload).map_err(BedrockError::Serialization)?;
                    Ok(ConverseStreamEvent::InternalServerException(exception))
                }
                "modelStreamErrorException" => {
                    let exception: BedrockException =
                        serde_json::from_slice(payload).map_err(BedrockError::Serialization)?;
                    Ok(ConverseStreamEvent::ModelStreamErrorException(exception))
                }
                "serviceUnavailableException" => {
                    let exception: BedrockException =
                        serde_json::from_slice(payload).map_err(BedrockError::Serialization)?;
                    Ok(ConverseStreamEvent::ServiceUnavailableException(exception))
                }
                "throttlingException" => {
                    let exception: BedrockException =
                        serde_json::from_slice(payload).map_err(BedrockError::Serialization)?;
                    Ok(ConverseStreamEvent::ThrottlingException(exception))
                }
                "validationException" => {
                    let exception: BedrockException =
                        serde_json::from_slice(payload).map_err(BedrockError::Serialization)?;
                    Ok(ConverseStreamEvent::ValidationException(exception))
                }
                unknown => Err(BedrockError::Validation {
                    message: format!("Unknown exception type: {}", unknown),
                }),
            },
            unknown => Err(BedrockError::Validation {
                message: format!("Unknown message type: {}", unknown),
            }),
        }
    }
}

impl From<ConverseStreamEvent> for String {
    fn from(val: ConverseStreamEvent) -> String {
        let transformed_json = serde_json::to_string(&val).unwrap_or_default();
        let event_type = match &val {
            ConverseStreamEvent::MessageStart { .. } => "message_start",
            ConverseStreamEvent::ContentBlockStart { .. } => "content_block_start",
            ConverseStreamEvent::ContentBlockDelta { .. } => "content_block_delta",
            ConverseStreamEvent::ContentBlockStop { .. } => "content_block_stop",
            ConverseStreamEvent::MessageStop { .. } => "message_stop",
            ConverseStreamEvent::Metadata { .. } => "metadata",
            ConverseStreamEvent::InternalServerException { .. } => "internal_server_exception",
            ConverseStreamEvent::ModelStreamErrorException { .. } => "model_stream_error_exception",
            ConverseStreamEvent::ServiceUnavailableException { .. } => {
                "service_unavailable_exception"
            }
            ConverseStreamEvent::ThrottlingException { .. } => "throttling_exception",
            ConverseStreamEvent::ValidationException { .. } => "validation_exception",
        };

        let event = format!("event: {}\n", event_type);
        let data = format!("data: {}\n\n", transformed_json);
        event + &data
    }
}

// Implement ProviderStreamResponse for ConverseStreamEvent
impl ProviderStreamResponse for ConverseStreamEvent {
    fn content_delta(&self) -> Option<&str> {
        match self {
            ConverseStreamEvent::ContentBlockDelta(event) => match &event.delta {
                ContentBlockDelta::Text { text } => Some(text),
                ContentBlockDelta::ToolUse { .. } => None,
            },
            _ => None,
        }
    }

    fn is_final(&self) -> bool {
        matches!(self, ConverseStreamEvent::MessageStop(_))
    }

    fn role(&self) -> Option<&str> {
        match self {
            ConverseStreamEvent::MessageStart(event) => Some(event.role.as_str()),
            _ => None,
        }
    }

    fn event_type(&self) -> Option<&str> {
        Some(match self {
            ConverseStreamEvent::MessageStart(_) => "messageStart",
            ConverseStreamEvent::ContentBlockStart(_) => "contentBlockStart",
            ConverseStreamEvent::ContentBlockDelta(_) => "contentBlockDelta",
            ConverseStreamEvent::ContentBlockStop(_) => "contentBlockStop",
            ConverseStreamEvent::MessageStop(_) => "messageStop",
            ConverseStreamEvent::Metadata(_) => "metadata",
            ConverseStreamEvent::InternalServerException(_) => "internalServerException",
            ConverseStreamEvent::ModelStreamErrorException(_) => "modelStreamErrorException",
            ConverseStreamEvent::ServiceUnavailableException(_) => "serviceUnavailableException",
            ConverseStreamEvent::ThrottlingException(_) => "throttlingException",
            ConverseStreamEvent::ValidationException(_) => "validationException",
        })
    }
}

// Add as_str helper for ConversationRole
impl ConversationRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConversationRole::User => "user",
            ConversationRole::Assistant => "assistant",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_tool_serialization_format() {
        let tool = Tool::ToolSpec {
            tool_spec: ToolSpecDefinition {
                name: "get_weather".to_string(),
                description: Some("Get the current weather for a specified city".to_string()),
                input_schema: ToolInputSchema {
                    json: json!({
                        "type": "object",
                        "properties": {
                            "city": {
                                "type": "string",
                                "description": "The city to get weather for"
                            }
                        },
                        "required": ["city"]
                    }),
                },
            },
        };

        let serialized = serde_json::to_value(&tool).unwrap();
        println!(
            "Tool serialization: {}",
            serde_json::to_string_pretty(&serialized).unwrap()
        );

        // Verify the structure matches Bedrock API expectations
        assert!(serialized.get("toolSpec").is_some());
        assert!(serialized.get("type").is_none()); // Should not have a type field

        let tool_spec = serialized.get("toolSpec").unwrap();
        assert_eq!(tool_spec.get("name").unwrap(), "get_weather");
        assert_eq!(
            tool_spec.get("description").unwrap(),
            "Get the current weather for a specified city"
        );
        assert!(tool_spec.get("inputSchema").is_some());
    }

    #[test]
    fn test_tool_choice_serialization_format() {
        // Test Auto choice
        let auto_choice = ToolChoice::Auto {
            auto: AutoChoice {},
        };
        let serialized = serde_json::to_value(&auto_choice).unwrap();
        println!(
            "Auto ToolChoice serialization: {}",
            serde_json::to_string_pretty(&serialized).unwrap()
        );

        assert!(serialized.get("auto").is_some());
        assert!(serialized.get("type").is_none()); // Should not have a type field

        // Test Tool choice
        let tool_choice = ToolChoice::Tool {
            tool: ToolChoiceSpec {
                name: "get_weather".to_string(),
            },
        };
        let serialized = serde_json::to_value(&tool_choice).unwrap();
        println!(
            "Tool ToolChoice serialization: {}",
            serde_json::to_string_pretty(&serialized).unwrap()
        );

        assert!(serialized.get("tool").is_some());
        assert!(serialized.get("type").is_none()); // Should not have a type field

        let tool_spec = serialized.get("tool").unwrap();
        assert_eq!(tool_spec.get("name").unwrap(), "get_weather");
    }
}
