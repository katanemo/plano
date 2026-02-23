use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_with::skip_serializing_none;
use std::collections::HashMap;
use std::fmt::Display;
use thiserror::Error;

use super::ApiDefinition;
use crate::providers::request::{ProviderRequest, ProviderRequestError};
use crate::providers::response::{ProviderResponse, TokenUsage};
use crate::providers::streaming_response::ProviderStreamResponse;
use crate::transforms::lib::ExtractText;
use crate::{CHAT_COMPLETIONS_PATH, OPENAI_RESPONSES_API_PATH};

// ============================================================================
// OPENAI API ENUMERATION
// ============================================================================

/// Enum for all supported OpenAI APIs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpenAIApi {
    ChatCompletions,
    Responses,
    // Future APIs can be added here:
    // Embeddings,
    // FineTuning,
    // etc.
}

impl ApiDefinition for OpenAIApi {
    fn endpoint(&self) -> &'static str {
        match self {
            OpenAIApi::ChatCompletions => CHAT_COMPLETIONS_PATH,
            OpenAIApi::Responses => OPENAI_RESPONSES_API_PATH,
        }
    }

    fn from_endpoint(endpoint: &str) -> Option<Self> {
        match endpoint {
            CHAT_COMPLETIONS_PATH => Some(OpenAIApi::ChatCompletions),
            OPENAI_RESPONSES_API_PATH => Some(OpenAIApi::Responses),
            _ => None,
        }
    }

    fn supports_streaming(&self) -> bool {
        match self {
            OpenAIApi::ChatCompletions => true,
            OpenAIApi::Responses => true,
        }
    }

    fn supports_tools(&self) -> bool {
        match self {
            OpenAIApi::ChatCompletions => true,
            OpenAIApi::Responses => true,
        }
    }

    fn supports_vision(&self) -> bool {
        match self {
            OpenAIApi::ChatCompletions => true,
            OpenAIApi::Responses => true,
        }
    }

    fn all_variants() -> Vec<Self> {
        vec![OpenAIApi::ChatCompletions, OpenAIApi::Responses]
    }
}

/// Chat completions API request
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ChatCompletionsRequest {
    pub messages: Vec<Message>,
    #[serde(default)]
    pub model: String,
    // pub audio: Option<Audio> // GOOD FIRST ISSUE: future support for audio input
    pub frequency_penalty: Option<f32>,
    // Function calling configuration has been deprecated, but we keep it for compatibility
    pub function_call: Option<FunctionChoice>,
    pub functions: Option<Vec<Tool>>,
    pub logit_bias: Option<HashMap<String, i32>>,
    pub logprobs: Option<bool>,
    pub max_completion_tokens: Option<u32>,
    // Maximum tokens in the response has been deprecated, but we keep it for compatibility
    pub max_tokens: Option<u32>,
    pub modalities: Option<Vec<String>>,
    pub metadata: Option<HashMap<String, Value>>,
    pub n: Option<u32>,
    pub presence_penalty: Option<f32>,
    pub parallel_tool_calls: Option<bool>,
    pub prediction: Option<StaticContent>,
    // pub reasoning_effect: Option<bool>, // GOOD FIRST ISSUE: Future support for reasoning effects
    pub response_format: Option<Value>,
    pub reasoning_effort: Option<String>, // e.g., "none", "low", "medium", "high"
    // pub safety_identifier: Option<String>, // GOOD FIRST ISSUE: Future support for safety identifiers
    pub seed: Option<i32>,
    pub service_tier: Option<String>,
    pub stop: Option<Vec<String>>,
    pub store: Option<bool>,
    pub stream: Option<bool>,
    pub stream_options: Option<StreamOptions>,
    pub temperature: Option<f32>,
    pub tool_choice: Option<ToolChoice>,
    pub tools: Option<Vec<Tool>>,
    pub top_p: Option<f32>,
    pub top_logprobs: Option<u32>,
    pub user: Option<String>,
    // pub web_search: Option<bool>, // GOOD FIRST ISSUE: Future support for web search

    // VLLM-specific parameters (used by Arch-Function)
    pub top_k: Option<u32>,
    pub stop_token_ids: Option<Vec<u32>>,
    pub continue_final_message: Option<bool>,
    pub add_generation_prompt: Option<bool>,
}

impl ChatCompletionsRequest {
    /// Suppress max_tokens if the model is o3, o3-*, openrouter/o3, or openrouter/o3-*
    pub fn suppress_max_tokens_if_o3(&mut self) {
        let model = self.model.as_str();
        let is_o3 = model == "o3"
            || model.starts_with("o3-")
            || model == "openrouter/o3"
            || model.starts_with("openrouter/o3-");
        if is_o3 {
            self.max_tokens = None;
        }
    }

    pub fn fix_temperature_if_gpt5(&mut self) {
        let model = self.model.as_str();
        if model.starts_with("gpt-5") {
            self.temperature = Some(1.0);
        }
    }
}

// ============================================================================
// CHAT COMPLETIONS API TYPES
// ============================================================================

/// Message role in a chat conversation
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Message {
    pub role: Role,
    /// The contents of the message. Required unless tool_calls is specified (for assistant role)
    pub content: Option<MessageContent>,
    pub name: Option<String>,
    /// Tool calls made by the assistant (only present for assistant role)
    pub tool_calls: Option<Vec<ToolCall>>,
    /// ID of the tool call that this message is responding to (only present for tool role)
    pub tool_call_id: Option<String>,
}

#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ResponseMessage {
    pub role: Role,
    /// The contents of the message (can be null for some cases)
    pub content: Option<String>,
    /// The refusal message generated by the model
    pub refusal: Option<String>,
    /// Annotations for the message, when applicable, as when using the web search tool
    pub annotations: Option<Vec<Value>>,
    /// If the audio output modality is requested, this object contains data about the audio response
    pub audio: Option<Value>,
    /// Deprecated and replaced by tool_calls. The name and arguments of a function that should be called
    pub function_call: Option<FunctionCall>,
    /// The tool calls generated by the model, such as function calls
    pub tool_calls: Option<Vec<ToolCall>>,
}

impl Default for ResponseMessage {
    fn default() -> Self {
        ResponseMessage {
            role: Role::Assistant,
            content: None,
            refusal: None,
            annotations: None,
            audio: None,
            function_call: None,
            tool_calls: None,
        }
    }
}

impl ResponseMessage {
    /// Convert ResponseMessage to Message for internal processing
    /// This is useful for transformations that need to work with the request Message type
    pub fn to_message(&self) -> Message {
        Message {
            role: self.role.clone(),
            content: self
                .content
                .as_ref()
                .map(|s| MessageContent::Text(s.clone())),
            name: None, // Response messages don't have names in the same way request messages do
            tool_calls: self.tool_calls.clone(),
            tool_call_id: None, // Response messages don't have tool_call_id
        }
    }
}

/// In the OpenAI API, this is represented as either:
/// - A string for simple text content
/// - An array of content parts for multimodal content (text + images)
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

// Content Extraction
impl ExtractText for MessageContent {
    fn extract_text(&self) -> String {
        match self {
            MessageContent::Text(text) => text.clone(),
            MessageContent::Parts(parts) => parts.extract_text(),
        }
    }
}

impl ExtractText for Option<MessageContent> {
    fn extract_text(&self) -> String {
        self.as_ref().map(|c| c.extract_text()).unwrap_or_default()
    }
}

impl ExtractText for Vec<ContentPart> {
    fn extract_text(&self) -> String {
        self.iter()
            .filter_map(|part| match part {
                ContentPart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Display for MessageContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.extract_text())
    }
}

/// Individual content part within a message (text or image)
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum ContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: ImageUrl },
}

/// Image URL configuration for vision capabilities
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ImageUrl {
    pub url: String,
    pub detail: Option<String>,
}

/// A single message in a chat conversation
/// A tool call made by the assistant
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionCall,
}

/// Function call within a tool call
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

/// Tool definition for function calling
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Tool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: Function,
}

/// Function definition within a tool
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Function {
    pub name: String,
    pub description: Option<String>,
    pub parameters: Value,
    pub strict: Option<bool>,
}

/// Tool choice string values
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ToolChoiceType {
    /// Let the model automatically decide whether to call tools
    Auto,
    /// Force the model to call at least one tool
    Required,
    /// Prevent the model from calling any tools
    None,
}

/// Tool choice configuration
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(untagged)]
pub enum ToolChoice {
    /// String-based tool choice (auto, required, none)
    Type(ToolChoiceType),
    /// Specific function to call
    Function {
        #[serde(rename = "type")]
        choice_type: String,
        function: FunctionChoice,
    },
}

/// Specific function choice
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct FunctionChoice {
    pub name: String,
}

/// Static content for prediction/prefill functionality
///
/// Static predicted output content, such as the content of a text file
/// that is being regenerated.
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StaticContent {
    /// The type of the predicted content you want to provide.
    /// This type is currently always "content".
    #[serde(rename = "type")]
    pub content_type: String,
    /// The content that should be matched when generating a model response.
    /// If generated tokens would match this content, the entire model response
    /// can be returned much more quickly.
    ///
    /// Can be either:
    /// - A string for simple text content
    /// - An array of content parts for structured content
    pub content: StaticContentType,
}

/// Content type for static/predicted content
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum StaticContentType {
    /// Simple text content - the content used for a Predicted Output.
    /// This is often the text of a file you are regenerating with minor changes.
    Text(String),
    /// An array of content parts with a defined type.
    /// Can contain text inputs and other supported content types.
    Parts(Vec<ContentPart>),
}

/// Chat completions API response
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ChatCompletionsResponse {
    pub id: String,
    pub object: Option<String>,
    pub created: u64,
    pub model: String,
    pub choices: Vec<Choice>,
    pub usage: Usage,
    pub system_fingerprint: Option<String>,
    pub service_tier: Option<String>,
    // This isn't a standard OpenAI field, but we include it for extensibility
    pub metadata: Option<HashMap<String, Value>>,
}

/// Finish reason for completion
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    FunctionCall, // Legacy
}

/// Token usage information
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    pub completion_tokens_details: Option<CompletionTokensDetails>,
}

/// Detailed breakdown of prompt tokens
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PromptTokensDetails {
    pub cached_tokens: Option<u32>,
    pub audio_tokens: Option<u32>,
}

/// Detailed breakdown of completion tokens
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CompletionTokensDetails {
    pub reasoning_tokens: Option<u32>,
    pub audio_tokens: Option<u32>,
    pub accepted_prediction_tokens: Option<u32>,
    pub rejected_prediction_tokens: Option<u32>,
}

/// A single choice in the response
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Choice {
    pub index: u32,
    pub message: ResponseMessage,
    pub finish_reason: Option<FinishReason>,
    pub logprobs: Option<Value>,
}

// ============================================================================
// STREAMING API TYPES
// ============================================================================

/// Streaming response from chat completions API
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ChatCompletionsStreamResponse {
    pub id: String,
    pub object: Option<String>,
    pub created: u64,
    pub model: String,
    pub choices: Vec<StreamChoice>,
    pub usage: Option<Usage>, // Only in final chunk
    pub system_fingerprint: Option<String>,
    /// Specifies the processing type used for serving the request
    pub service_tier: Option<String>,
}

/// A choice in a streaming response
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StreamChoice {
    pub index: u32,
    pub delta: MessageDelta,
    pub finish_reason: Option<FinishReason>,
    pub logprobs: Option<Value>,
}

/// Message delta for streaming updates
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MessageDelta {
    pub role: Option<Role>,
    pub content: Option<String>,
    /// The refusal message generated by the model
    pub refusal: Option<String>,
    /// Deprecated and replaced by tool_calls. The name and arguments of a function that should be called
    pub function_call: Option<FunctionCall>,
    pub tool_calls: Option<Vec<ToolCallDelta>>,
}

/// Tool call delta for streaming tool call updates
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ToolCallDelta {
    pub index: u32,
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub call_type: Option<String>,
    pub function: Option<FunctionCallDelta>,
}

/// Function call delta for streaming function call updates
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct FunctionCallDelta {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

/// Stream options for controlling streaming behavior
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StreamOptions {
    pub include_usage: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDetail {
    pub id: String,
    pub object: Option<String>,
    pub created: usize,
    pub owned_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ModelObject {
    #[serde(rename = "list")]
    List,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Models {
    pub object: ModelObject,
    pub data: Vec<ModelDetail>,
}

// Error type for streaming operations
#[derive(Debug, thiserror::Error)]
pub enum OpenAIStreamError {
    #[error("JSON parsing error: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("UTF-8 parsing error: {0}")]
    Utf8Error(#[from] std::str::Utf8Error),
    #[error("Invalid streaming data: {0}")]
    InvalidStreamingData(String),
}

#[derive(Debug, Error)]
pub enum OpenAIError {
    #[error("json error: {0}")]
    JsonParseError(#[from] serde_json::Error),
    #[error("utf8 parsing error: {0}")]
    Utf8Error(#[from] std::str::Utf8Error),
    #[error("invalid streaming data err {source}, data: {data}")]
    InvalidStreamingData {
        source: serde_json::Error,
        data: String,
    },
    #[error("unsupported provider: {provider}")]
    UnsupportedProvider { provider: String },
}

// ============================================================================
/// Trait Implementations
/// ===========================================================================
/// Parameterized conversion for ChatCompletionsRequest
impl TryFrom<&[u8]> for ChatCompletionsRequest {
    type Error = OpenAIStreamError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        let mut req: ChatCompletionsRequest =
            serde_json::from_slice(bytes).map_err(OpenAIStreamError::from)?;
        // Use the centralized suppression logic
        req.suppress_max_tokens_if_o3();
        req.fix_temperature_if_gpt5();
        Ok(req)
    }
}

/// Parameterized conversion for ChatCompletionsResponse
impl TryFrom<&[u8]> for ChatCompletionsResponse {
    type Error = OpenAIStreamError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        serde_json::from_slice(bytes).map_err(OpenAIStreamError::from)
    }
}

/// Implementation of TokenUsage for OpenAI Usage type
impl TokenUsage for Usage {
    fn completion_tokens(&self) -> usize {
        self.completion_tokens as usize
    }

    fn prompt_tokens(&self) -> usize {
        self.prompt_tokens as usize
    }

    fn total_tokens(&self) -> usize {
        self.total_tokens as usize
    }
}

/// Implementation of ProviderRequest for ChatCompletionsRequest
impl ProviderRequest for ChatCompletionsRequest {
    fn model(&self) -> &str {
        &self.model
    }

    fn set_model(&mut self, model: String) {
        self.model = model;
    }

    fn is_streaming(&self) -> bool {
        self.stream.unwrap_or_default()
    }

    fn extract_messages_text(&self) -> String {
        self.messages.iter().fold(String::new(), |acc, m| {
            let content_text = m
                .content
                .as_ref()
                .map(|content| match content {
                    MessageContent::Text(text) => text.clone(),
                    MessageContent::Parts(parts) => parts
                        .iter()
                        .map(|part| match part {
                            ContentPart::Text { text } => text.clone(),
                            ContentPart::ImageUrl { .. } => "[Image]".to_string(),
                        })
                        .collect::<Vec<_>>()
                        .join(" "),
                })
                .unwrap_or_default();
            acc + " " + &content_text
        })
    }

    fn get_recent_user_message(&self) -> Option<String> {
        self.messages.last().and_then(|msg| {
            msg.content.as_ref().and_then(|content| match content {
                MessageContent::Text(text) => Some(text.clone()),
                MessageContent::Parts(_) => None, // No user message in parts
            })
        })
    }

    fn get_tool_names(&self) -> Option<Vec<String>> {
        // First check the 'tools' field (current API)
        if let Some(tools) = &self.tools {
            let names: Vec<String> = tools
                .iter()
                .map(|tool| tool.function.name.clone())
                .collect();
            if !names.is_empty() {
                return Some(names);
            }
        }

        // Fallback to 'functions' field (deprecated but still supported)
        if let Some(functions) = &self.functions {
            let names: Vec<String> = functions
                .iter()
                .map(|func| func.function.name.clone())
                .collect();
            if !names.is_empty() {
                return Some(names);
            }
        }

        None
    }

    fn to_bytes(&self) -> Result<Vec<u8>, ProviderRequestError> {
        serde_json::to_vec(&self).map_err(|e| ProviderRequestError {
            message: format!("Failed to serialize OpenAI request: {}", e),
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
        self.temperature
    }

    fn get_messages(&self) -> Vec<crate::apis::openai::Message> {
        self.messages.clone()
    }

    fn set_messages(&mut self, messages: &[crate::apis::openai::Message]) {
        self.messages = messages.to_vec();
    }
}

/// Implementation of ProviderResponse for ChatCompletionsResponse
impl ProviderResponse for ChatCompletionsResponse {
    fn usage(&self) -> Option<&dyn TokenUsage> {
        Some(&self.usage)
    }

    fn extract_usage_counts(&self) -> Option<(usize, usize, usize)> {
        Some((
            self.usage.prompt_tokens(),
            self.usage.completion_tokens(),
            self.usage.total_tokens(),
        ))
    }
}

// Direct implementation of ProviderStreamResponse trait on ChatCompletionsStreamResponse
impl ProviderStreamResponse for ChatCompletionsStreamResponse {
    fn content_delta(&self) -> Option<&str> {
        self.choices
            .first()
            .and_then(|choice| choice.delta.content.as_deref())
    }

    fn is_final(&self) -> bool {
        self.choices
            .first()
            .map(|choice| choice.finish_reason.is_some())
            .unwrap_or(false)
    }

    fn role(&self) -> Option<&str> {
        self.choices.first().and_then(|choice| {
            choice.delta.role.as_ref().map(|r| match r {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
            })
        })
    }

    fn event_type(&self) -> Option<&str> {
        None // OpenAI doesn't use event types in SSE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_required_fields() {
        // Create a JSON object with only required fields
        let original_json = json!({
            "model": "gpt-4",
            "messages": [
                {
                    "content": "Hello, world!",
                    "role": "user"
                }
            ]
        });

        // Deserialize JSON into ChatCompletionsRequest
        let deserialized_request: ChatCompletionsRequest =
            serde_json::from_value(original_json.clone()).unwrap();

        // Validate required fields are properly set
        assert_eq!(deserialized_request.model, "gpt-4");
        assert_eq!(deserialized_request.messages.len(), 1);

        let message = &deserialized_request.messages[0];
        assert_eq!(message.role, Role::User);
        assert!(message.content.is_some());
        if let Some(MessageContent::Text(content)) = &message.content {
            assert_eq!(content, "Hello, world!");
        } else {
            panic!("Expected text content");
        }

        // Serialize the ChatCompletionsRequest back to JSON
        let serialized_json = serde_json::to_value(&deserialized_request).unwrap();
        assert_eq!(original_json, serialized_json);
    }

    #[test]
    fn test_optional_fields_serialization() {
        // Create a JSON object with optional fields set
        let original_json = json!({
            "model": "gpt-4",
            "messages": [
                {
                    "content": "Test message",
                    "role": "user",
                    "name": "test_user"
                }
            ],
            "temperature": 0.7,
            "max_tokens": 150,
            "stream": true,
            "stream_options": {
                "include_usage": true
            },
            "metadata": {
                "user_id": "123"
            }
        });

        // Deserialize JSON into ChatCompletionsRequest
        let deserialized_request: ChatCompletionsRequest =
            serde_json::from_value(original_json.clone()).unwrap();

        // Validate required fields
        assert_eq!(deserialized_request.model, "gpt-4");
        assert_eq!(deserialized_request.messages.len(), 1);

        let message = &deserialized_request.messages[0];
        assert_eq!(message.role, Role::User);
        assert!(message.content.is_some());
        if let Some(MessageContent::Text(content)) = &message.content {
            assert_eq!(content, "Test message");
        } else {
            panic!("Expected text content");
        }
        assert_eq!(message.name, Some("test_user".to_string()));

        // Validate optional fields are properly set
        assert!((deserialized_request.temperature.unwrap() - 0.7).abs() < 1e-6);
        assert_eq!(deserialized_request.max_tokens, Some(150));
        assert_eq!(deserialized_request.stream, Some(true));
        assert!(deserialized_request.stream_options.is_some());
        assert!(deserialized_request.metadata.is_some());

        // Validate fields not in JSON are None
        assert!(deserialized_request.top_p.is_none());
        assert!(deserialized_request.frequency_penalty.is_none());
        assert!(deserialized_request.presence_penalty.is_none());
        assert!(deserialized_request.stop.is_none());
        assert!(deserialized_request.tools.is_none());

        // Serialize back to JSON and compare (handle floating point precision)
        let serialized_json = serde_json::to_value(&deserialized_request).unwrap();

        // Compare all fields except temperature which needs floating point comparison
        assert_eq!(serialized_json["model"], original_json["model"]);
        assert_eq!(serialized_json["messages"], original_json["messages"]);
        assert_eq!(serialized_json["max_tokens"], original_json["max_tokens"]);
        assert_eq!(serialized_json["stream"], original_json["stream"]);
        assert_eq!(
            serialized_json["stream_options"],
            original_json["stream_options"]
        );
        assert_eq!(serialized_json["metadata"], original_json["metadata"]);

        // Handle temperature with floating point tolerance
        let original_temp = original_json["temperature"].as_f64().unwrap();
        let serialized_temp = serialized_json["temperature"].as_f64().unwrap();
        assert!((original_temp - serialized_temp).abs() < 1e-6);
    }

    #[test]
    fn test_nested_types_serialization() {
        // Create a comprehensive JSON object with nested types - a ChatCompletionsRequest with complex message content and tools
        let original_json = json!({
            "model": "gpt-4-vision-preview",
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": "What can you see in this image and what's the weather like in the location shown?"
                        },
                        {
                            "type": "image_url",
                            "image_url": {
                                "url": "https://example.com/cityscape.jpg",
                                "detail": "high"
                            }
                        }
                    ]
                },
                {
                    "role": "assistant",
                    "content": "I can see a beautiful cityscape. Let me check the weather for you.",
                    "tool_calls": [
                        {
                            "id": "call_weather123",
                            "type": "function",
                            "function": {
                                "name": "get_weather",
                                "arguments": "{\"location\": \"New York, NY\"}"
                            }
                        }
                    ]
                },
                {
                    "role": "tool",
                    "content": "Current weather in New York: 72°F, sunny",
                    "tool_call_id": "call_weather123"
                }
            ],
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "description": "Get current weather information for a location",
                        "parameters": {
                            "type": "object",
                            "properties": {
                                "location": {
                                    "type": "string",
                                    "description": "The city and state, e.g. San Francisco, CA"
                                }
                            },
                            "required": ["location"]
                        },
                        "strict": true
                    }
                }
            ],
            "tool_choice": "auto",
            "temperature": 0.7,
            "max_tokens": 1000,
            "prediction": {
                "type": "content",
                "content": "Based on the image analysis and weather data, I can provide you with comprehensive information."
            }
        });

        // Deserialize JSON into ChatCompletionsRequest
        let deserialized_request: ChatCompletionsRequest =
            serde_json::from_value(original_json.clone()).unwrap();

        // Validate top-level fields
        assert_eq!(deserialized_request.model, "gpt-4-vision-preview");
        assert_eq!(deserialized_request.messages.len(), 3);
        assert!((deserialized_request.temperature.unwrap() - 0.7).abs() < 1e-6);
        assert_eq!(deserialized_request.max_tokens, Some(1000));

        // Validate first message (user with multimodal content)
        let user_message = &deserialized_request.messages[0];
        assert_eq!(user_message.role, Role::User);
        assert!(user_message.content.is_some());
        if let Some(MessageContent::Parts(ref content_parts)) = user_message.content {
            assert_eq!(content_parts.len(), 2);

            // Validate text content part
            if let ContentPart::Text { text } = &content_parts[0] {
                assert_eq!(text, "What can you see in this image and what's the weather like in the location shown?");
            } else {
                panic!("Expected text content part");
            }

            // Validate image URL content part
            if let ContentPart::ImageUrl { ref image_url } = content_parts[1] {
                assert_eq!(image_url.url, "https://example.com/cityscape.jpg");
                assert_eq!(image_url.detail, Some("high".to_string()));
            } else {
                panic!("Expected image URL content part");
            }
        } else {
            panic!("Expected multimodal content parts for user message");
        }

        // Validate second message (assistant with tool calls)
        let assistant_message = &deserialized_request.messages[1];
        assert_eq!(assistant_message.role, Role::Assistant);
        assert!(assistant_message.content.is_some());
        if let Some(MessageContent::Text(text)) = &assistant_message.content {
            assert_eq!(
                text,
                "I can see a beautiful cityscape. Let me check the weather for you."
            );
        } else {
            panic!("Expected text content for assistant message");
        }

        // Validate tool calls in assistant message
        assert!(assistant_message.tool_calls.is_some());
        let tool_calls = assistant_message.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 1);

        let tool_call = &tool_calls[0];
        assert_eq!(tool_call.id, "call_weather123");
        assert_eq!(tool_call.call_type, "function");
        assert_eq!(tool_call.function.name, "get_weather");
        assert_eq!(
            tool_call.function.arguments,
            "{\"location\": \"New York, NY\"}"
        );

        // Validate third message (tool response)
        let tool_message = &deserialized_request.messages[2];
        assert_eq!(tool_message.role, Role::Tool);
        assert!(tool_message.content.is_some());
        if let Some(MessageContent::Text(text)) = &tool_message.content {
            assert_eq!(text, "Current weather in New York: 72°F, sunny");
        } else {
            panic!("Expected text content for tool message");
        }
        assert_eq!(
            tool_message.tool_call_id,
            Some("call_weather123".to_string())
        );

        // Validate tools array
        assert!(deserialized_request.tools.is_some());
        let tools = deserialized_request.tools.as_ref().unwrap();
        assert_eq!(tools.len(), 1);

        let tool = &tools[0];
        assert_eq!(tool.tool_type, "function");
        assert_eq!(tool.function.name, "get_weather");
        assert_eq!(
            tool.function.description,
            Some("Get current weather information for a location".to_string())
        );
        assert_eq!(tool.function.strict, Some(true));

        // Validate tool parameters schema
        let parameters = &tool.function.parameters;
        assert_eq!(parameters["type"], "object");
        assert!(parameters["properties"]["location"].is_object());
        assert_eq!(parameters["required"], json!(["location"]));

        // Validate tool choice
        if let Some(ToolChoice::Type(choice)) = &deserialized_request.tool_choice {
            assert_eq!(choice, &ToolChoiceType::Auto);
        } else {
            panic!("Expected auto tool choice");
        }

        // Validate prediction
        assert!(deserialized_request.prediction.is_some());
        let prediction = deserialized_request.prediction.as_ref().unwrap();
        assert_eq!(prediction.content_type, "content");
        if let StaticContentType::Text(text) = &prediction.content {
            assert_eq!(text, "Based on the image analysis and weather data, I can provide you with comprehensive information.");
        } else {
            panic!("Expected text prediction content");
        }

        // Serialize back to JSON and compare (handle floating point precision)
        let serialized_json = serde_json::to_value(&deserialized_request).unwrap();

        // Compare all fields except floating point ones
        assert_eq!(serialized_json["model"], original_json["model"]);
        assert_eq!(serialized_json["messages"], original_json["messages"]);
        assert_eq!(serialized_json["max_tokens"], original_json["max_tokens"]);
        assert_eq!(serialized_json["tools"], original_json["tools"]);
        assert_eq!(serialized_json["tool_choice"], original_json["tool_choice"]);
        assert_eq!(serialized_json["prediction"], original_json["prediction"]);

        // Handle floating point field with tolerance
        let original_temp = original_json["temperature"].as_f64().unwrap();
        let serialized_temp = serialized_json["temperature"].as_f64().unwrap();
        assert!((original_temp - serialized_temp).abs() < 1e-6);
    }

    #[test]
    fn test_assistant_message_with_tool_calls_no_content() {
        // Test that assistant messages can have tool_calls without content
        let json_with_tool_calls_no_content = json!({
            "model": "gpt-4",
            "messages": [
                {
                    "role": "user",
                    "content": "What's the weather in San Francisco?"
                },
                {
                    "role": "assistant",
                    "tool_calls": [
                        {
                            "id": "call_123",
                            "type": "function",
                            "function": {
                                "name": "get_weather",
                                "arguments": "{\"location\": \"San Francisco, CA\"}"
                            }
                        }
                    ]
                }
            ]
        });

        // Should deserialize successfully
        let request: ChatCompletionsRequest =
            serde_json::from_value(json_with_tool_calls_no_content.clone()).unwrap();

        assert_eq!(request.messages.len(), 2);

        // Check user message
        let user_msg = &request.messages[0];
        assert_eq!(user_msg.role, Role::User);
        assert!(user_msg.content.is_some());

        // Check assistant message - should have tool_calls but no content
        let assistant_msg = &request.messages[1];
        assert_eq!(assistant_msg.role, Role::Assistant);
        assert!(assistant_msg.content.is_none());
        assert!(assistant_msg.tool_calls.is_some());

        let tool_calls = assistant_msg.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "call_123");
        assert_eq!(tool_calls[0].function.name, "get_weather");

        // Should serialize back without content field
        let serialized = serde_json::to_value(&request).unwrap();
        // Verify the assistant message doesn't have a content field in serialized JSON
        let serialized_assistant_msg = &serialized["messages"][1];
        assert!(serialized_assistant_msg.get("content").is_none());
        assert!(serialized_assistant_msg.get("tool_calls").is_some());
    }

    #[test]
    fn test_api_provider_trait() {
        // Test the ApiDefinition trait implementation
        let api = OpenAIApi::ChatCompletions;

        // Test trait methods
        assert_eq!(api.endpoint(), CHAT_COMPLETIONS_PATH);
        assert!(api.supports_streaming());
        assert!(api.supports_tools());
        assert!(api.supports_vision());

        // Test from_endpoint
        let found_api = OpenAIApi::from_endpoint(CHAT_COMPLETIONS_PATH);
        assert_eq!(found_api, Some(OpenAIApi::ChatCompletions));

        let not_found = OpenAIApi::from_endpoint("/v1/unknown");
        assert_eq!(not_found, None);

        // Test all_variants
        let all_variants = OpenAIApi::all_variants();
        assert_eq!(all_variants.len(), 2);
        assert!(all_variants.contains(&OpenAIApi::ChatCompletions));
        assert!(all_variants.contains(&OpenAIApi::Responses));
    }

    #[test]
    fn test_role_specific_behavior() {
        // Test 1: User message - basic content, no tool-related fields
        let user_json = json!({
            "content": "Hello!",
            "role": "user",
            "name": "user123"
        });

        let deserialized_user: Message = serde_json::from_value(user_json.clone()).unwrap();
        assert_eq!(deserialized_user.role, Role::User);
        if let Some(MessageContent::Text(content)) = &deserialized_user.content {
            assert_eq!(content, "Hello!");
        } else {
            panic!("Expected text content");
        }
        assert_eq!(deserialized_user.name, Some("user123".to_string()));
        assert!(deserialized_user.tool_calls.is_none());
        assert!(deserialized_user.tool_call_id.is_none());

        let serialized_user_json = serde_json::to_value(&deserialized_user).unwrap();
        assert_eq!(user_json, serialized_user_json);

        // Test 2: Assistant message with tool calls
        let assistant_json = json!({
            "content": "I'll help with that.",
            "role": "assistant",
            "tool_calls": [
                {
                    "id": "call_456",
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "arguments": r#"{"location":"SF"}"#
                    }
                }
            ]
        });

        let deserialized_assistant: Message =
            serde_json::from_value(assistant_json.clone()).unwrap();
        assert_eq!(deserialized_assistant.role, Role::Assistant);
        if let Some(MessageContent::Text(content)) = &deserialized_assistant.content {
            assert_eq!(content, "I'll help with that.");
        } else {
            panic!("Expected text content");
        }
        assert!(deserialized_assistant.tool_calls.is_some());
        assert!(deserialized_assistant.tool_call_id.is_none());
        assert!(deserialized_assistant.name.is_none());

        let tool_calls = deserialized_assistant.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "call_456");
        assert_eq!(tool_calls[0].function.name, "get_weather");

        let serialized_assistant_json = serde_json::to_value(&deserialized_assistant).unwrap();
        assert_eq!(assistant_json, serialized_assistant_json);

        // Test 3: Tool message responding to a call
        let tool_json = json!({
            "content": "Weather is sunny",
            "role": "tool",
            "tool_call_id": "call_456"
        });

        let deserialized_tool: Message = serde_json::from_value(tool_json.clone()).unwrap();
        assert_eq!(deserialized_tool.role, Role::Tool);
        if let Some(MessageContent::Text(content)) = &deserialized_tool.content {
            assert_eq!(content, "Weather is sunny");
        } else {
            panic!("Expected text content");
        }
        assert_eq!(deserialized_tool.tool_call_id, Some("call_456".to_string()));
        assert!(deserialized_tool.tool_calls.is_none());
        assert!(deserialized_tool.name.is_none());

        let serialized_tool_json = serde_json::to_value(&deserialized_tool).unwrap();
        assert_eq!(tool_json, serialized_tool_json);

        // Test 4: ResponseMessage vs Message differences
        let response_json = json!({
            "role": "assistant",
            "content": "Response content",
            "annotations": [
                {"type": "citation"}
            ]
        });

        let deserialized_response: ResponseMessage =
            serde_json::from_value(response_json.clone()).unwrap();
        assert_eq!(deserialized_response.role, Role::Assistant);
        assert_eq!(
            deserialized_response.content,
            Some("Response content".to_string())
        );
        assert!(deserialized_response.annotations.is_some());
        assert!(deserialized_response.refusal.is_none());
        assert!(deserialized_response.function_call.is_none());
        assert!(deserialized_response.tool_calls.is_none());

        let serialized_response_json = serde_json::to_value(&deserialized_response).unwrap();
        assert_eq!(response_json, serialized_response_json);

        // Test conversion from ResponseMessage to Message
        let converted = deserialized_response.to_message();
        assert_eq!(converted.role, Role::Assistant);
        if let Some(MessageContent::Text(text)) = converted.content {
            assert_eq!(text, "Response content");
        } else {
            panic!("Expected text content");
        }
        assert!(converted.name.is_none());
        assert!(converted.tool_call_id.is_none());
    }

    #[test]
    fn test_tool_choice_type_serialization() {
        // Test that the enum serializes to the correct string values
        let auto_choice = ToolChoice::Type(ToolChoiceType::Auto);
        let required_choice = ToolChoice::Type(ToolChoiceType::Required);
        let none_choice = ToolChoice::Type(ToolChoiceType::None);

        let auto_json = serde_json::to_value(&auto_choice).unwrap();
        let required_json = serde_json::to_value(&required_choice).unwrap();
        let none_json = serde_json::to_value(&none_choice).unwrap();

        assert_eq!(auto_json, "auto");
        assert_eq!(required_json, "required");
        assert_eq!(none_json, "none");

        // Test deserialization from string values
        let auto_deserialized: ToolChoice = serde_json::from_value(json!("auto")).unwrap();
        let required_deserialized: ToolChoice = serde_json::from_value(json!("required")).unwrap();
        let none_deserialized: ToolChoice = serde_json::from_value(json!("none")).unwrap();

        assert_eq!(auto_deserialized, ToolChoice::Type(ToolChoiceType::Auto));
        assert_eq!(
            required_deserialized,
            ToolChoice::Type(ToolChoiceType::Required)
        );
        assert_eq!(none_deserialized, ToolChoice::Type(ToolChoiceType::None));

        // Test that invalid string values fail deserialization (type safety!)
        let invalid_result: Result<ToolChoice, _> = serde_json::from_value(json!("invalid"));
        assert!(invalid_result.is_err());
    }

    #[test]
    fn test_chat_completions_response_with_service_tier() {
        // Test that ChatCompletionsResponse can deserialize OpenAI responses with service_tier field
        let json_response = r#"{
            "id": "chatcmpl-CAJc2Df6QCc7Mv3RP0Cf2xlbDV1x2",
            "object": "chat.completion",
            "created": 1756574706,
            "model": "gpt-4o-2024-08-06",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Test response content",
                    "annotations": []
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 65,
                "completion_tokens": 184,
                "total_tokens": 249,
                "prompt_tokens_details": {
                    "cached_tokens": 0,
                    "audio_tokens": 0
                },
                "completion_tokens_details": {
                    "reasoning_tokens": 0,
                    "audio_tokens": 0,
                    "accepted_prediction_tokens": 0,
                    "rejected_prediction_tokens": 0
                }
            },
            "service_tier": "default",
            "system_fingerprint": "fp_f33640a400"
        }"#;

        let response: ChatCompletionsResponse = serde_json::from_str(json_response).unwrap();

        assert_eq!(response.id, "chatcmpl-CAJc2Df6QCc7Mv3RP0Cf2xlbDV1x2");
        assert_eq!(response.object.as_deref(), Some("chat.completion"));
        assert_eq!(response.created, 1756574706);
        assert_eq!(response.model, "gpt-4o-2024-08-06");
        assert_eq!(response.service_tier, Some("default".to_string()));
        assert_eq!(
            response.system_fingerprint,
            Some("fp_f33640a400".to_string())
        );
        assert_eq!(response.choices.len(), 1);
        assert_eq!(response.usage.prompt_tokens, 65);
        assert_eq!(response.usage.completion_tokens, 184);
        assert_eq!(response.usage.total_tokens, 249);
    }

    #[test]
    fn test_chat_completions_response_without_service_tier() {
        // Test that ChatCompletionsResponse can deserialize responses without service_tier (backward compatibility)
        let json_response = r#"{
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Test response"
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 20,
                "total_tokens": 30
            }
        }"#;

        let response: ChatCompletionsResponse = serde_json::from_str(json_response).unwrap();

        assert_eq!(response.id, "chatcmpl-123");
        assert_eq!(response.service_tier, None); // Should be None when not present
        assert_eq!(response.system_fingerprint, None);
    }
}
