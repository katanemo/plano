use crate::providers::response::TokenUsage;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_with::skip_serializing_none;
use std::collections::HashMap;

use super::ApiDefinition;
use crate::providers::request::{ProviderRequest, ProviderRequestError};
use crate::providers::response::ProviderResponse;
use crate::providers::streaming_response::ProviderStreamResponse;
use crate::transforms::lib::ExtractText;
use crate::MESSAGES_PATH;

// Enum for all supported Anthropic APIs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnthropicApi {
    Messages,
    // Future APIs can be added here:
    // Embeddings,
    // etc.
}

impl ApiDefinition for AnthropicApi {
    fn endpoint(&self) -> &'static str {
        match self {
            AnthropicApi::Messages => MESSAGES_PATH,
        }
    }

    fn from_endpoint(endpoint: &str) -> Option<Self> {
        match endpoint {
            MESSAGES_PATH => Some(AnthropicApi::Messages),
            _ => None,
        }
    }

    fn supports_streaming(&self) -> bool {
        match self {
            AnthropicApi::Messages => true,
        }
    }

    fn supports_tools(&self) -> bool {
        match self {
            AnthropicApi::Messages => true,
        }
    }

    fn supports_vision(&self) -> bool {
        match self {
            AnthropicApi::Messages => true,
        }
    }

    fn all_variants() -> Vec<Self> {
        vec![AnthropicApi::Messages]
    }
}

// Service tier enum for request priority
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServiceTier {
    Auto,
    StandardOnly,
}

// Thinking configuration
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ThinkingConfig {
    #[serde(rename = "type")]
    pub thinking_type: String,
    pub budget_tokens: Option<u32>,
}

// MCP Server types
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpServerType {
    Url,
}

#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct McpToolConfiguration {
    pub allowed_tools: Option<Vec<String>>,
    pub enabled: Option<bool>,
}

#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct McpServer {
    pub name: String,
    #[serde(rename = "type")]
    pub server_type: McpServerType,
    pub url: String,
    pub authorization_token: Option<String>,
    pub tool_configuration: Option<McpToolConfiguration>,
}

#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MessagesRequest {
    #[serde(default)]
    pub model: String,
    pub messages: Vec<MessagesMessage>,
    pub max_tokens: u32,
    pub container: Option<String>,
    pub mcp_servers: Option<Vec<McpServer>>,
    pub system: Option<MessagesSystemPrompt>,
    pub metadata: Option<HashMap<String, Value>>,
    pub service_tier: Option<ServiceTier>,
    pub thinking: Option<ThinkingConfig>,

    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    pub stream: Option<bool>,
    pub stop_sequences: Option<Vec<String>>,
    pub tools: Option<Vec<MessagesTool>>,
    pub tool_choice: Option<MessagesToolChoice>,
}

// Messages API specific types
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessagesRole {
    User,
    Assistant,
}

/// Cache control types for content blocks
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "type")]
pub enum MessagesCacheControl {
    Ephemeral,
}

/// Tool result content can be either a string or array of content blocks
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum ToolResultContent {
    Text(String),
    Blocks(Vec<MessagesContentBlock>),
}

impl ExtractText for ToolResultContent {
    fn extract_text(&self) -> String {
        match self {
            ToolResultContent::Text(text) => text.clone(),
            ToolResultContent::Blocks(blocks) => blocks.extract_text(),
        }
    }
}

#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "type")]
pub enum MessagesContentBlock {
    Text {
        text: String,
        cache_control: Option<MessagesCacheControl>,
    },
    Thinking {
        thinking: String,
        signature: Option<String>,
        cache_control: Option<MessagesCacheControl>,
    },
    Image {
        source: MessagesImageSource,
    },
    Document {
        source: MessagesDocumentSource,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
        cache_control: Option<MessagesCacheControl>,
    },
    ToolResult {
        tool_use_id: String,
        is_error: Option<bool>,
        content: ToolResultContent,
        cache_control: Option<MessagesCacheControl>,
    },
    ServerToolUse {
        id: String,
        name: String,
        input: Value,
    },
    WebSearchToolResult {
        tool_use_id: String,
        is_error: Option<bool>,
        content: Vec<MessagesContentBlock>,
    },
    CodeExecutionToolResult {
        tool_use_id: String,
        is_error: Option<bool>,
        content: Vec<MessagesContentBlock>,
    },
    McpToolUse {
        id: String,
        name: String,
        input: Value,
    },
    McpToolResult {
        tool_use_id: String,
        is_error: Option<bool>,
        content: Vec<MessagesContentBlock>,
    },
    ContainerUpload {
        id: String,
        name: String,
        media_type: String,
        data: String,
    },
}

impl ExtractText for Vec<MessagesContentBlock> {
    fn extract_text(&self) -> String {
        self.iter()
            .filter_map(|block| match block {
                MessagesContentBlock::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "type")]
pub enum MessagesImageSource {
    Base64 { media_type: String, data: String },
    Url { url: String },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "type")]
pub enum MessagesDocumentSource {
    Base64 { media_type: String, data: String },
    Url { url: String },
    File { file_id: String },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum MessagesMessageContent {
    Single(String),
    Blocks(Vec<MessagesContentBlock>),
}

impl ExtractText for MessagesMessageContent {
    fn extract_text(&self) -> String {
        match self {
            MessagesMessageContent::Single(text) => text.clone(),
            MessagesMessageContent::Blocks(parts) => parts.extract_text(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum MessagesSystemPrompt {
    Single(String),
    Blocks(Vec<MessagesContentBlock>),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MessagesMessage {
    pub role: MessagesRole,
    pub content: MessagesMessageContent,
}

#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MessagesTool {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Value,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MessagesToolChoiceType {
    Auto,
    Any,
    Tool,
    None,
}

#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MessagesToolChoice {
    #[serde(rename = "type")]
    pub kind: MessagesToolChoiceType,
    pub name: Option<String>,
    pub disable_parallel_tool_use: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessagesStopReason {
    EndTurn,
    MaxTokens,
    StopSequence,
    ToolUse,
    PauseTurn,
    Refusal,
}

#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MessagesUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_creation_input_tokens: Option<u32>,
    pub cache_read_input_tokens: Option<u32>,
}

// Container response object
#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MessagesContainer {
    pub id: String,
    #[serde(rename = "type")]
    pub container_type: String,
    pub name: String,
    pub status: String,
}

#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MessagesResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub obj_type: String,
    pub role: MessagesRole,
    pub content: Vec<MessagesContentBlock>,
    pub model: String,
    pub stop_reason: MessagesStopReason,
    pub stop_sequence: Option<String>,
    pub usage: MessagesUsage,
    pub container: Option<MessagesContainer>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "type")]
pub enum MessagesStreamEvent {
    MessageStart {
        message: MessagesStreamMessage,
    },
    ContentBlockStart {
        index: u32,
        content_block: MessagesContentBlock,
    },
    ContentBlockDelta {
        index: u32,
        delta: MessagesContentDelta,
    },
    ContentBlockStop {
        index: u32,
    },
    MessageDelta {
        delta: MessagesMessageDelta,
        usage: MessagesUsage,
    },
    MessageStop,
    Ping,
}

#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MessagesStreamMessage {
    pub id: String,
    #[serde(rename = "type")]
    pub obj_type: String,
    pub role: MessagesRole,
    pub content: Vec<Value>, // Initially empty
    pub model: String,
    pub stop_reason: Option<MessagesStopReason>,
    pub stop_sequence: Option<String>,
    pub usage: MessagesUsage,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum MessagesContentDelta {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
    #[serde(rename = "thinking_delta")]
    ThinkingDelta { thinking: String },
    #[serde(rename = "signature_delta")]
    SignatureDelta { signature: String },
}

#[skip_serializing_none]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MessagesMessageDelta {
    pub stop_reason: MessagesStopReason,
    pub stop_sequence: Option<String>,
}

// Helper functions for API detection and conversion
impl MessagesRequest {
    pub fn api_type() -> AnthropicApi {
        AnthropicApi::Messages
    }
}

impl TryFrom<&[u8]> for MessagesRequest {
    type Error = serde_json::Error;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        serde_json::from_slice(bytes)
    }
}

impl TokenUsage for MessagesResponse {
    fn completion_tokens(&self) -> usize {
        self.usage.output_tokens as usize
    }
    fn prompt_tokens(&self) -> usize {
        self.usage.input_tokens as usize
    }
    fn total_tokens(&self) -> usize {
        (self.usage.input_tokens + self.usage.output_tokens) as usize
    }
}

impl ProviderResponse for MessagesResponse {
    fn usage(&self) -> Option<&dyn TokenUsage> {
        Some(self)
    }
    fn extract_usage_counts(&self) -> Option<(usize, usize, usize)> {
        Some((
            self.usage.input_tokens as usize,
            self.usage.output_tokens as usize,
            (self.usage.input_tokens + self.usage.output_tokens) as usize,
        ))
    }
}

impl ProviderRequest for MessagesRequest {
    fn model(&self) -> &str {
        &self.model
    }

    fn set_model(&mut self, model: String) {
        self.model = model;
    }

    fn is_streaming(&self) -> bool {
        self.stream.unwrap_or(false)
    }

    fn extract_messages_text(&self) -> String {
        let mut text_parts = Vec::new();

        // Include system prompt if present
        if let Some(system) = &self.system {
            match system {
                MessagesSystemPrompt::Single(s) => text_parts.push(s.clone()),
                MessagesSystemPrompt::Blocks(blocks) => {
                    for block in blocks {
                        if let MessagesContentBlock::Text { text, .. } = block {
                            text_parts.push(text.clone());
                        }
                    }
                }
            }
        }

        // Extract text from all messages
        for message in &self.messages {
            match &message.content {
                MessagesMessageContent::Single(text) => text_parts.push(text.clone()),
                MessagesMessageContent::Blocks(blocks) => {
                    for block in blocks {
                        if let MessagesContentBlock::Text { text, .. } = block {
                            text_parts.push(text.clone());
                        }
                    }
                }
            }
        }

        text_parts.join(" ")
    }

    fn get_recent_user_message(&self) -> Option<String> {
        // Find the most recent user message
        for message in self.messages.iter().rev() {
            if message.role == MessagesRole::User {
                match &message.content {
                    MessagesMessageContent::Single(text) => return Some(text.clone()),
                    MessagesMessageContent::Blocks(blocks) => {
                        for block in blocks {
                            if let MessagesContentBlock::Text { text, .. } = block {
                                return Some(text.clone());
                            }
                        }
                    }
                }
            }
        }
        None
    }

    fn get_tool_names(&self) -> Option<Vec<String>> {
        self.tools
            .as_ref()
            .map(|tools| tools.iter().map(|tool| tool.name.clone()).collect())
    }

    fn to_bytes(&self) -> Result<Vec<u8>, ProviderRequestError> {
        serde_json::to_vec(self).map_err(|e| ProviderRequestError {
            message: format!("Failed to serialize MessagesRequest: {}", e),
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
        use crate::apis::openai::Message;

        let mut openai_messages = Vec::new();

        // Add system prompt as system message if present
        if let Some(system) = &self.system {
            openai_messages.push(system.clone().into());
        }

        // Convert each Anthropic message to OpenAI format
        for msg in &self.messages {
            if let Ok(converted_msgs) = TryInto::<Vec<Message>>::try_into(msg.clone()) {
                openai_messages.extend(converted_msgs);
            }
        }

        openai_messages
    }

    fn set_messages(&mut self, messages: &[crate::apis::openai::Message]) {
        // Convert OpenAI messages to Anthropic format
        // Separate system messages from regular messages
        let mut system_messages = Vec::new();
        let mut regular_messages = Vec::new();

        for msg in messages {
            if msg.role == crate::apis::openai::Role::System {
                system_messages.push(msg.clone());
            } else {
                regular_messages.push(msg.clone());
            }
        }

        // Set system prompt if there are system messages
        if !system_messages.is_empty() {
            // Combine all system messages into one
            let system_text = system_messages
                .iter()
                .filter_map(|msg| {
                    if let Some(crate::apis::openai::MessageContent::Text(text)) = &msg.content {
                        Some(text.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");

            self.system = Some(crate::apis::anthropic::MessagesSystemPrompt::Single(
                system_text,
            ));
        }

        // Convert regular messages
        self.messages = regular_messages
            .iter()
            .filter_map(|msg| msg.clone().try_into().ok())
            .collect();
    }
}

impl MessagesResponse {
    pub fn api_type() -> AnthropicApi {
        AnthropicApi::Messages
    }
}

impl MessagesStreamEvent {
    pub fn api_type() -> AnthropicApi {
        AnthropicApi::Messages
    }
}

impl MessagesRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            MessagesRole::User => "user",
            MessagesRole::Assistant => "assistant",
        }
    }
}

// Implement ProviderStreamResponse for MessagesStreamEvent
impl ProviderStreamResponse for MessagesStreamEvent {
    fn content_delta(&self) -> Option<&str> {
        match self {
            MessagesStreamEvent::ContentBlockDelta { delta, .. } => match delta {
                MessagesContentDelta::TextDelta { text } => Some(text),
                MessagesContentDelta::ThinkingDelta { thinking } => Some(thinking),
                _ => None,
            },
            _ => None,
        }
    }

    fn is_final(&self) -> bool {
        matches!(self, MessagesStreamEvent::MessageStop)
    }

    fn role(&self) -> Option<&str> {
        match self {
            MessagesStreamEvent::MessageStart { message } => Some(message.role.as_str()),
            _ => None,
        }
    }

    fn event_type(&self) -> Option<&str> {
        Some(match self {
            MessagesStreamEvent::MessageStart { .. } => "message_start",
            MessagesStreamEvent::ContentBlockStart { .. } => "content_block_start",
            MessagesStreamEvent::ContentBlockDelta { .. } => "content_block_delta",
            MessagesStreamEvent::ContentBlockStop { .. } => "content_block_stop",
            MessagesStreamEvent::MessageDelta { .. } => "message_delta",
            MessagesStreamEvent::MessageStop => "message_stop",
            MessagesStreamEvent::Ping => "ping",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_anthropic_required_fields() {
        // Create a JSON object with only required fields
        let original_json = json!({
            "model": "claude-3-sonnet-20240229",
            "messages": [
                {
                    "role": "user",
                    "content": "Hello"
                }
            ],
            "max_tokens": 100
        });

        // Deserialize JSON into MessagesRequest
        let deserialized_request: MessagesRequest =
            serde_json::from_value(original_json.clone()).unwrap();

        // Validate required fields are properly set
        assert_eq!(deserialized_request.model, "claude-3-sonnet-20240229");
        assert_eq!(deserialized_request.messages.len(), 1);
        assert_eq!(deserialized_request.max_tokens, 100);

        let message = &deserialized_request.messages[0];
        assert_eq!(message.role, MessagesRole::User);
        if let MessagesMessageContent::Single(content) = &message.content {
            assert_eq!(content, "Hello");
        } else {
            panic!("Expected single content");
        }

        // Validate optional fields are None
        assert!(deserialized_request.system.is_none());
        assert!(deserialized_request.container.is_none());
        assert!(deserialized_request.mcp_servers.is_none());
        assert!(deserialized_request.service_tier.is_none());
        assert!(deserialized_request.thinking.is_none());
        assert!(deserialized_request.temperature.is_none());
        assert!(deserialized_request.top_p.is_none());
        assert!(deserialized_request.top_k.is_none());
        assert!(deserialized_request.stream.is_none());
        assert!(deserialized_request.stop_sequences.is_none());
        assert!(deserialized_request.tools.is_none());
        assert!(deserialized_request.tool_choice.is_none());
        assert!(deserialized_request.metadata.is_none());

        // Serialize back to JSON and compare
        let serialized_json = serde_json::to_value(&deserialized_request).unwrap();
        assert_eq!(original_json, serialized_json);
    }

    #[test]
    fn test_anthropic_optional_fields() {
        // Create a JSON object with optional fields set
        let original_json = json!({
            "model": "claude-3-sonnet-20240229",
            "messages": [
                {
                    "role": "user",
                    "content": "Hello"
                }
            ],
            "max_tokens": 100,
            "temperature": 0.7,
            "top_p": 0.9,
            "system": "You are a helpful assistant",
            "service_tier": "auto",
            "thinking": {
                "type": "enabled"
            },
            "metadata": {
                "user_id": "123"
            }
        });

        // Deserialize JSON into MessagesRequest
        let deserialized_request: MessagesRequest =
            serde_json::from_value(original_json.clone()).unwrap();

        // Validate required fields
        assert_eq!(deserialized_request.model, "claude-3-sonnet-20240229");
        assert_eq!(deserialized_request.messages.len(), 1);
        assert_eq!(deserialized_request.max_tokens, 100);

        // Validate optional fields are properly set
        assert!((deserialized_request.temperature.unwrap() - 0.7).abs() < 1e-6);
        assert!((deserialized_request.top_p.unwrap() - 0.9).abs() < 1e-6);
        assert_eq!(deserialized_request.service_tier, Some(ServiceTier::Auto));

        if let Some(MessagesSystemPrompt::Single(system)) = &deserialized_request.system {
            assert_eq!(system, "You are a helpful assistant");
        } else {
            panic!("Expected single system prompt");
        }

        if let Some(thinking) = &deserialized_request.thinking {
            assert_eq!(thinking.thinking_type, "enabled");
        } else {
            panic!("Expected thinking config");
        }

        assert!(deserialized_request.metadata.is_some());

        // Validate fields not in JSON are None
        assert!(deserialized_request.container.is_none());
        assert!(deserialized_request.mcp_servers.is_none());
        assert!(deserialized_request.top_k.is_none());
        assert!(deserialized_request.stream.is_none());
        assert!(deserialized_request.stop_sequences.is_none());
        assert!(deserialized_request.tools.is_none());
        assert!(deserialized_request.tool_choice.is_none());

        // Serialize back to JSON and compare (handle floating point precision)
        let serialized_json = serde_json::to_value(&deserialized_request).unwrap();

        // Compare all fields except floating point ones
        assert_eq!(serialized_json["model"], original_json["model"]);
        assert_eq!(serialized_json["messages"], original_json["messages"]);
        assert_eq!(serialized_json["max_tokens"], original_json["max_tokens"]);
        assert_eq!(serialized_json["system"], original_json["system"]);
        assert_eq!(
            serialized_json["service_tier"],
            original_json["service_tier"]
        );
        assert_eq!(serialized_json["thinking"], original_json["thinking"]);
        assert_eq!(serialized_json["metadata"], original_json["metadata"]);

        // Handle floating point fields with tolerance
        let original_temp = original_json["temperature"].as_f64().unwrap();
        let serialized_temp = serialized_json["temperature"].as_f64().unwrap();
        assert!((original_temp - serialized_temp).abs() < 1e-6);

        let original_top_p = original_json["top_p"].as_f64().unwrap();
        let serialized_top_p = serialized_json["top_p"].as_f64().unwrap();
        assert!((original_top_p - serialized_top_p).abs() < 1e-6);
    }

    #[test]
    fn test_anthropic_nested_types() {
        // Create a comprehensive JSON object with nested types - a MessagesRequest with complex message content and tools
        let original_json = json!({
            "model": "claude-3-sonnet-20240229",
            "max_tokens": 1000,
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": "What can you see in this image and what's the weather like?"
                        },
                        {
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": "image/jpeg",
                                "data": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="
                            }
                        }
                    ]
                },
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "thinking",
                            "thinking": "Let me analyze the image and then check the weather..."
                        },
                        {
                            "type": "text",
                            "text": "I can see the image. Let me check the weather for you."
                        },
                        {
                            "type": "tool_use",
                            "id": "toolu_weather123",
                            "name": "get_weather",
                            "input": {
                                "location": "San Francisco, CA"
                            }
                        }
                    ]
                }
            ],
            "tools": [
                {
                    "name": "get_weather",
                    "description": "Get current weather information for a location",
                    "input_schema": {
                        "type": "object",
                        "properties": {
                            "location": {
                                "type": "string",
                                "description": "The city and state, e.g. San Francisco, CA"
                            }
                        },
                        "required": ["location"]
                    }
                }
            ],
            "tool_choice": {
                "type": "auto"
            },
            "system": [
                {
                    "type": "text",
                    "text": "You are a helpful assistant that can analyze images and provide weather information."
                }
            ]
        });

        // Deserialize JSON into MessagesRequest
        let deserialized_request: MessagesRequest =
            serde_json::from_value(original_json.clone()).unwrap();

        // Validate top-level fields
        assert_eq!(deserialized_request.model, "claude-3-sonnet-20240229");
        assert_eq!(deserialized_request.max_tokens, 1000);
        assert_eq!(deserialized_request.messages.len(), 2);

        // Validate first message (user with text and image content)
        let user_message = &deserialized_request.messages[0];
        assert_eq!(user_message.role, MessagesRole::User);
        if let MessagesMessageContent::Blocks(ref content_blocks) = user_message.content {
            assert_eq!(content_blocks.len(), 2);

            // Validate text content block
            if let MessagesContentBlock::Text { text, .. } = &content_blocks[0] {
                assert_eq!(
                    text,
                    "What can you see in this image and what's the weather like?"
                );
            } else {
                panic!("Expected text content block");
            }

            // Validate image content block
            if let MessagesContentBlock::Image { ref source } = content_blocks[1] {
                if let MessagesImageSource::Base64 { media_type, data } = source {
                    assert_eq!(media_type, "image/jpeg");
                    assert_eq!(data, "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==");
                } else {
                    panic!("Expected base64 image source");
                }
            } else {
                panic!("Expected image content block");
            }
        } else {
            panic!("Expected content blocks for user message");
        }

        // Validate second message (assistant with thinking, text, and tool use)
        let assistant_message = &deserialized_request.messages[1];
        assert_eq!(assistant_message.role, MessagesRole::Assistant);
        if let MessagesMessageContent::Blocks(ref content_blocks) = assistant_message.content {
            assert_eq!(content_blocks.len(), 3);

            // Validate thinking content block
            if let MessagesContentBlock::Thinking { thinking, .. } = &content_blocks[0] {
                assert_eq!(
                    thinking,
                    "Let me analyze the image and then check the weather..."
                );
            } else {
                panic!("Expected thinking content block");
            }

            // Validate text content block
            if let MessagesContentBlock::Text { text, .. } = &content_blocks[1] {
                assert_eq!(
                    text,
                    "I can see the image. Let me check the weather for you."
                );
            } else {
                panic!("Expected text content block");
            }

            // Validate tool use content block
            if let MessagesContentBlock::ToolUse {
                ref id,
                ref name,
                ref input,
                ..
            } = content_blocks[2]
            {
                assert_eq!(id, "toolu_weather123");
                assert_eq!(name, "get_weather");
                assert_eq!(input["location"], "San Francisco, CA");
            } else {
                panic!("Expected tool use content block");
            }
        } else {
            panic!("Expected content blocks for assistant message");
        }

        // Validate tools array
        assert!(deserialized_request.tools.is_some());
        let tools = deserialized_request.tools.as_ref().unwrap();
        assert_eq!(tools.len(), 1);

        let tool = &tools[0];
        assert_eq!(tool.name, "get_weather");
        assert_eq!(
            tool.description,
            Some("Get current weather information for a location".to_string())
        );
        assert_eq!(tool.input_schema["type"], "object");
        assert!(tool.input_schema["properties"]["location"].is_object());

        // Validate tool choice
        assert!(deserialized_request.tool_choice.is_some());
        let tool_choice = deserialized_request.tool_choice.as_ref().unwrap();
        assert_eq!(tool_choice.kind, MessagesToolChoiceType::Auto);
        assert!(tool_choice.name.is_none());

        // Validate system prompt with content blocks
        assert!(deserialized_request.system.is_some());
        if let Some(MessagesSystemPrompt::Blocks(ref system_blocks)) = deserialized_request.system {
            assert_eq!(system_blocks.len(), 1);
            if let MessagesContentBlock::Text { text, .. } = &system_blocks[0] {
                assert_eq!(text, "You are a helpful assistant that can analyze images and provide weather information.");
            } else {
                panic!("Expected text content block in system prompt");
            }
        } else {
            panic!("Expected system prompt with content blocks");
        }

        // Serialize back to JSON and compare
        let serialized_json = serde_json::to_value(&deserialized_request).unwrap();
        assert_eq!(original_json, serialized_json);
    }

    #[test]
    fn test_anthropic_mcp_server_configuration() {
        // Test MCP Server configuration with JSON-first approach
        let mcp_server_json = json!({
            "name": "test-server",
            "type": "url",
            "url": "https://example.com/mcp",
            "authorization_token": "secret-token",
            "tool_configuration": {
                "allowed_tools": ["tool1", "tool2"],
                "enabled": true
            }
        });

        let deserialized_mcp: McpServer = serde_json::from_value(mcp_server_json.clone()).unwrap();
        assert_eq!(deserialized_mcp.name, "test-server");
        assert_eq!(deserialized_mcp.server_type, McpServerType::Url);
        assert_eq!(deserialized_mcp.url, "https://example.com/mcp");
        assert_eq!(
            deserialized_mcp.authorization_token,
            Some("secret-token".to_string())
        );

        if let Some(tool_config) = &deserialized_mcp.tool_configuration {
            assert_eq!(
                tool_config.allowed_tools,
                Some(vec!["tool1".to_string(), "tool2".to_string()])
            );
            assert_eq!(tool_config.enabled, Some(true));
        } else {
            panic!("Expected tool configuration");
        }

        let serialized_mcp_json = serde_json::to_value(&deserialized_mcp).unwrap();
        assert_eq!(mcp_server_json, serialized_mcp_json);

        // Test MCP Server with minimal configuration (optional fields as None)
        let minimal_mcp_json = json!({
            "name": "minimal-server",
            "type": "url",
            "url": "https://minimal.com/mcp"
        });

        let deserialized_minimal: McpServer =
            serde_json::from_value(minimal_mcp_json.clone()).unwrap();
        assert_eq!(deserialized_minimal.name, "minimal-server");
        assert_eq!(deserialized_minimal.server_type, McpServerType::Url);
        assert_eq!(deserialized_minimal.url, "https://minimal.com/mcp");
        assert!(deserialized_minimal.authorization_token.is_none());
        assert!(deserialized_minimal.tool_configuration.is_none());

        let serialized_minimal_json = serde_json::to_value(&deserialized_minimal).unwrap();
        assert_eq!(minimal_mcp_json, serialized_minimal_json);
    }

    #[test]
    fn test_anthropic_response_types() {
        // Test MessagesResponse deserialization
        let response_json = json!({
            "id": "msg_01ABC123",
            "type": "message",
            "role": "assistant",
            "content": [
                {
                    "type": "text",
                    "text": "Hello! How can I help you today?"
                }
            ],
            "model": "claude-3-sonnet-20240229",
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 10,
                "output_tokens": 25,
                "cache_creation_input_tokens": 5,
                "cache_read_input_tokens": 3
            }
        });

        let deserialized_response: MessagesResponse =
            serde_json::from_value(response_json.clone()).unwrap();
        assert_eq!(deserialized_response.id, "msg_01ABC123");
        assert_eq!(deserialized_response.obj_type, "message");
        assert_eq!(deserialized_response.role, MessagesRole::Assistant);
        assert_eq!(deserialized_response.model, "claude-3-sonnet-20240229");
        assert_eq!(
            deserialized_response.stop_reason,
            MessagesStopReason::EndTurn
        );
        assert!(deserialized_response.stop_sequence.is_none());
        assert!(deserialized_response.container.is_none());

        // Check content
        assert_eq!(deserialized_response.content.len(), 1);
        if let MessagesContentBlock::Text { text, .. } = &deserialized_response.content[0] {
            assert_eq!(text, "Hello! How can I help you today?");
        } else {
            panic!("Expected text content block");
        }

        // Check usage
        assert_eq!(deserialized_response.usage.input_tokens, 10);
        assert_eq!(deserialized_response.usage.output_tokens, 25);
        assert_eq!(
            deserialized_response.usage.cache_creation_input_tokens,
            Some(5)
        );
        assert_eq!(deserialized_response.usage.cache_read_input_tokens, Some(3));

        let serialized_response_json = serde_json::to_value(&deserialized_response).unwrap();
        assert_eq!(response_json, serialized_response_json);

        // Test streaming event
        let stream_event_json = json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {
                "type": "text_delta",
                "text": " How"
            }
        });

        let deserialized_event: MessagesStreamEvent =
            serde_json::from_value(stream_event_json.clone()).unwrap();
        if let MessagesStreamEvent::ContentBlockDelta { index, ref delta } = deserialized_event {
            assert_eq!(index, 0);
            if let MessagesContentDelta::TextDelta { text } = delta {
                assert_eq!(text, " How");
            } else {
                panic!("Expected text delta");
            }
        } else {
            panic!("Expected content block delta event");
        }

        let serialized_event_json = serde_json::to_value(&deserialized_event).unwrap();
        assert_eq!(stream_event_json, serialized_event_json);
    }

    #[test]
    fn test_anthropic_tool_use_content() {
        // Test tool use and tool result content blocks
        let tool_use_json = json!({
            "type": "tool_use",
            "id": "toolu_01ABC123",
            "name": "get_weather",
            "input": {
                "location": "San Francisco, CA"
            }
        });

        let deserialized_tool_use: MessagesContentBlock =
            serde_json::from_value(tool_use_json.clone()).unwrap();
        if let MessagesContentBlock::ToolUse {
            ref id,
            ref name,
            ref input,
            ..
        } = deserialized_tool_use
        {
            assert_eq!(id, "toolu_01ABC123");
            assert_eq!(name, "get_weather");
            assert_eq!(input["location"], "San Francisco, CA");
        } else {
            panic!("Expected tool use content block");
        }

        let serialized_tool_use_json = serde_json::to_value(&deserialized_tool_use).unwrap();
        assert_eq!(tool_use_json, serialized_tool_use_json);

        // Test tool result content block
        let tool_result_json = json!({
            "type": "tool_result",
            "tool_use_id": "toolu_01ABC123",
            "content": [
                {
                    "type": "text",
                    "text": "The weather in San Francisco is sunny, 72°F"
                }
            ]
        });

        let deserialized_tool_result: MessagesContentBlock =
            serde_json::from_value(tool_result_json.clone()).unwrap();
        if let MessagesContentBlock::ToolResult {
            ref tool_use_id,
            ref is_error,
            ref content,
            ..
        } = deserialized_tool_result
        {
            assert_eq!(tool_use_id, "toolu_01ABC123");
            assert!(is_error.is_none());
            if let ToolResultContent::Blocks(blocks) = content {
                assert_eq!(blocks.len(), 1);
                if let MessagesContentBlock::Text { text, .. } = &blocks[0] {
                    assert_eq!(text, "The weather in San Francisco is sunny, 72°F");
                } else {
                    panic!("Expected text content in tool result");
                }
            } else {
                panic!("Expected blocks content in tool result");
            }
        } else {
            panic!("Expected tool result content block");
        }

        let serialized_tool_result_json = serde_json::to_value(&deserialized_tool_result).unwrap();
        assert_eq!(tool_result_json, serialized_tool_result_json);
    }

    #[test]
    fn test_anthropic_nested_types_with_cache_control() {
        // Test complete MessagesRequest with cache_control fields and various content types
        let complex_request_json = json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 4096,
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": "<system-reminder>\nThis is a reminder that your todo list is currently empty. DO NOT mention this to the user explicitly because they are already aware. If you are working on tasks that would benefit from a todo list please use the TodoWrite tool to create one. If not, please feel free to ignore. Again do not mention this message to the user.\n</system-reminder>"
                        },
                        {
                            "type": "text",
                            "text": "<system-reminder>\nAs you answer the user's questions, you can use the following context:\n# important-instruction-reminders\nDo what has been asked; nothing more, nothing less.\nNEVER create files unless they're absolutely necessary for achieving your goal.\nALWAYS prefer editing an existing file to creating a new one.\nNEVER proactively create documentation files (*.md) or README files. Only create documentation files if explicitly requested by the User.\n\n      \n      IMPORTANT: this context may or may not be relevant to your tasks. You should not respond to this context unless it is highly relevant to your task.\n</system-reminder>\n"
                        },
                        {
                            "type": "text",
                            "text": "Do we need to add more tests to transformers.rs?"
                        }
                    ]
                },
                {
                    "role": "assistant",
                    "content": [
                        {
                            "type": "tool_use",
                            "id": "call_kV50LtJQKHvvzZui5TW56DUl",
                            "name": "TodoWrite",
                            "input": {
                                "todos": [
                                    {
                                        "activeForm": "Locating and inspecting transformers.rs tests",
                                        "content": "Locate transformers.rs and inspect existing tests",
                                        "status": "pending"
                                    },
                                    {
                                        "activeForm": "Running tests and checking failures",
                                        "content": "Run the test suite and check for failures related to transformers.rs",
                                        "status": "pending"
                                    },
                                    {
                                        "activeForm": "Adding/updating tests for transformers.rs",
                                        "content": "Add or update unit/integration tests for transformers.rs if coverage is insufficient",
                                        "status": "pending"
                                    }
                                ]
                            },
                            "cache_control": {
                                "type": "ephemeral"
                            }
                        }
                    ]
                },
                {
                    "role": "user",
                    "content": [
                        {
                            "tool_use_id": "call_kV50LtJQKHvvzZui5TW56DUl",
                            "type": "tool_result",
                            "content": "Todos have been modified successfully. Ensure that you continue to use the todo list to track your progress. Please proceed with the current tasks if applicable\n\n<system-reminder>\nYour todo list has changed. DO NOT mention this explicitly to the user. Here are the latest contents of your todo list:\n\n[{\"content\":\"Locate transformers.rs and inspect existing tests\",\"status\":\"pending\",\"activeForm\":\"Locating and inspecting transformers.rs tests\"},{\"content\":\"Run the test suite and check for failures related to transformers.rs\",\"status\":\"pending\",\"activeForm\":\"Running tests and checking failures\"},{\"content\":\"Add or update unit/integration tests for transformers.rs if coverage is insufficient\",\"status\":\"pending\",\"activeForm\":\"Adding/updating tests for transformers.rs\"}]. Continue on with the tasks at hand if applicable.\n</system-reminder>"
                        },
                        {
                            "type": "text",
                            "text": "should I add more tests to transformers.rs?"
                        },
                        {
                            "type": "text",
                            "text": "try again",
                            "cache_control": {
                                "type": "ephemeral"
                            }
                        }
                    ]
                }
            ],
            "temperature": 1,
            "system": [
                {
                    "type": "text",
                    "text": "You are Claude Code, Anthropic's official CLI for Claude.",
                    "cache_control": {
                        "type": "ephemeral"
                    }
                },
                {
                    "type": "text",
                    "text": "\nYou are an interactive CLI tool that helps users with software engineering tasks. Use the instructions below and the tools available to you to assist the user.\n\nIMPORTANT: Assist with defensive security tasks only. Refuse to create, modify, or improve code that may be used maliciously. Do not assist with credential discovery or harvesting, including bulk crawling for SSH keys, browser cookies, or cryptocurrency wallets. Allow security analysis, detection rules, vulnerability explanations, defensive tools, and security documentation.\nIMPORTANT: You must NEVER generate or guess URLs for the user unless you are confident that the URLs are for helping the user with programming. You may use URLs provided by the user in their messages or local files.\n\nIf the user asks for help or wants to give feedback inform them of the following: \n- /help: Get help with using Claude Code\n- To give feedback, users should report the issue at https://github.com/anthropics/claude-code/issues\n\nWhen\u{2060} the user directly asks about Claude Code (eg. \"can Claude Code do...\", \"does Claude Code have...\"), or asks in second person (eg. \"are you able...\", \"can you do...\"), or asks how to use a specific Claude Code feature (eg. implement a hook, or write a slash command), use the WebFetch tool to gather information to answer the question from Claude Code docs. The list of available docs is available at https://docs.claude.com/en/docs/claude-code/claude_code_docs_map.md.\n\n#\u{2060} Tone and style\nYou should be concise, direct, and to the point.\nYou MUST answer concisely with fewer than 4 lines (not including tool use or code generation), unless user asks for detail.\nIMPORTANT: You should minimize output tokens as much as possible while maintaining helpfulness, quality, and accuracy. Only address the specific task at hand, avoiding tangential information unless absolutely critical for completing the request. If you can answer in 1-3 sentences or a short paragraph, please do.\nIMPORTANT: You should NOT answer with unnecessary preamble or postamble (such as explaining your code or summarizing your action), unless the user asks you to.\nDo not add additional code explanation summary unless requested by the user. After working on a file, just stop, rather than providing an explanation of what you did.\nAnswer the user's question directly, avoiding any elaboration, explanation, introduction, conclusion, or excessive details. One word answers are best. You MUST avoid text before/after your response, such as \"The answer is <answer>.\", \"Here is the content of the file...\" or \"Based on the information provided, the answer is...\" or \"Here is what I will do next...\".\n\nHere are some examples to demonstrate appropriate verbosity:\n<example>\nuser: 2 + 2\nassistant: 4\n</example>\n\n<example>\nuser: what is 2+2?\nassistant: 4\n</example>\n\n<example>\nuser: is 11 a prime number?\nassistant: Yes\n</example>\n\n<example>\nuser: what command should I run to list files in the current directory?\nassistant: ls\n</example>\n\n<example>\nuser: what command should I run to watch files in the current directory?\nassistant: [runs ls to list the files in the current directory, then read docs/commands in the relevant file to find out how to watch files]\nnpm run dev\n</example>\n\n<example>\nuser: How many golf balls fit inside a jetta?\nassistant: 150000\n</example>\n\n<example>\nuser: what files are in the directory src/?\nassistant: [runs ls and sees foo.c, bar.c, baz.c]\nuser: which file contains the implementation of foo?\nassistant: src/foo.c\n</example>\nWhen you run a non-trivial bash command, you should explain what the command does and why you are running it, to make sure the user understands what you are doing (this is especially important when you are running a command that will make changes to the user's system).\nRemember that your output will be displayed on a command line interface. Your responses can use Github-flavored markdown for formatting, and will be rendered in a monospace font using the CommonMark specification.\nOutput text to communicate with the user; all text you output outside of tool use is displayed to the user. Only use tools to complete tasks. Never use tools like Bash or code comments as means to communicate with the user during the session.\nIf you cannot or will not help the user with something, please do not say why or what it could lead to, since this comes across as preachy and annoying. Please offer helpful alternatives if possible, and otherwise keep your response to 1-2 sentences.\nOnly use emojis if the user explicitly requests it. Avoid using emojis in all communication unless asked.\nIMPORTANT: Keep your responses short, since they will be displayed on a command line interface.\n\n# Proactiveness\nYou are allowed to be proactive, but only when the user asks you to do something. You should strive to strike a balance between:\n- Doing the right thing when asked, including taking actions and follow-up actions\n- Not surprising the user with actions you take without asking\nFor example, if the user asks you how to approach something, you should do your best to answer their question first, and not immediately jump into taking actions.\n\n# Professional objectivity\nPrioritize technical accuracy and truthfulness over validating the user's beliefs. Focus on facts and problem-solving, providing direct, objective technical info without any unnecessary superlatives, praise, or emotional validation. It is best for the user if Claude honestly applies the same rigorous standards to all ideas and disagrees when necessary, even if it may not be what the user wants to hear. Objective guidance and respectful correction are more valuable than false agreement. Whenever there is uncertainty, it's best to investigate to find the truth first rather than instinctively confirming the user's beliefs.\n\n# Following conventions\nWhen making changes to files, first understand the file's code conventions. Mimic code style, use existing libraries and utilities, and follow existing patterns.\n- NEVER assume that a given library is available, even if it is well known. Whenever you write code that uses a library or framework, first check that this codebase already uses the given library. For example, you might look at neighboring files, or check the package.json (or cargo.toml, and so on depending on the language).\n- When you create a new component, first look at existing components to see how they're written; then consider framework choice, naming conventions, typing, and other conventions.\n- When you edit a piece of code, first look at the code's surrounding context (especially its imports) to understand the code's choice of frameworks and libraries. Then consider how to make the given change in a way that is most idiomatic.\n- Always follow security best practices. Never introduce code that exposes or logs secrets and keys. Never commit secrets or keys to the repository.\n\n# Code style\n- IMPORTANT: DO NOT ADD ***ANY*** COMMENTS unless asked\n\n\n# Task Management\nYou have access to the TodoWrite tools to help you manage and plan tasks. Use these tools VERY frequently to ensure that you are tracking your tasks and giving the user visibility into your progress.\nThese tools are also EXTREMELY helpful for planning tasks, and for breaking down larger complex tasks into smaller steps. If you do not use this tool when planning, you may forget to do important tasks - and that is unacceptable.\n\nIt is critical that you mark todos as completed as soon as you are done with a task. Do not batch up multiple tasks before marking them as completed.\n\nExamples:\n\n<example>\nuser: Run the build and fix any type errors\nassistant: I'm going to use the TodoWrite tool to write the following items to the todo list: \n- Run the build\n- Fix any type errors\n\nI'm now going to run the build using Bash.\n\nLooks like I found 10 type errors. I'm going to use the TodoWrite tool to write 10 items to the todo list.\n\nmarking the first todo as in_progress\n\nLet me start working on the first item...\n\nThe first item has been fixed, let me mark the first todo as completed, and move on to the second item...\n..\n..\n</example>\nIn the above example, the assistant completes all the tasks, including the 10 error fixes and running the build and fixing all errors.\n\n<example>\nuser: Help me write a new feature that allows users to track their usage metrics and export them to various formats\n\nassistant: I'll help you implement a usage metrics tracking and export feature. Let me first use the TodoWrite tool to plan this task.\nAdding the following todos to the todo list:\n1. Research existing metrics tracking in the codebase\n2. Design the metrics collection system\n3. Implement core metrics tracking functionality\n4. Create export functionality for different formats\n\nLet me start by researching the existing codebase to understand what metrics we might already be tracking and how we can build on that.\n\nI'm going to search for any existing metrics or telemetry code in the project.\n\nI've found some existing telemetry code. Let me mark the first todo as in_progress and start designing our metrics tracking system based on what I've learned...\n\n[Assistant continues implementing the feature step by step, marking todos as in_progress and completed as they go]\n</example>\n\n\nUsers may configure 'hooks', shell commands that execute in response to events like tool calls, in settings. Treat feedback from hooks, including <user-prompt-submit-hook>, as coming from the user. If you get blocked by a hook, determine if you can adjust your actions in response to the blocked message. If not, ask the user to check their hooks configuration.\n\n# Doing tasks\nThe user will primarily request you perform software engineering tasks. This includes solving bugs, adding new functionality, refactoring code, explaining code, and more. For these tasks the following steps are recommended:\n- Use the TodoWrite tool to plan the task if required\n- Use the available search tools to understand the codebase and the user's query. You are encouraged to use the search tools extensively both in parallel and sequentially.\n- Implement the solution using all tools available to you\n- Verify the solution if possible with tests. NEVER assume specific test framework or test script. Check the README or search codebase to determine the testing approach.\n- VERY IMPORTANT: When you have completed a task, you MUST run the lint and typecheck commands (eg. npm run lint, npm run typecheck, ruff, etc.) with Bash if they were provided to you to ensure your code is correct. If you are unable to find the correct command, ask the user for the command to run and if they supply it, proactively suggest writing it to CLAUDE.md so that you will know to run it next time.\nNEVER commit changes unless the user explicitly asks you to. It is VERY IMPORTANT to only commit when explicitly asked, otherwise the user will feel that you are being too proactive.\n\n- Tool results and user messages may include <system-reminder> tags. <system-reminder> tags contain useful information and reminders. They are automatically added by the system, and bear no direct relation to the specific tool results or user messages in which they appear.\n\n\n# Tool usage policy\n- When doing file search, prefer to use the Task tool in order to reduce context usage.\n- You should proactively use the Task tool with specialized agents when the task at hand matches the agent's description.\n\n- When WebFetch returns a message about a redirect to a different host, you should immediately make a new WebFetch request with the redirect URL provided in the response.\n- You have the capability to call multiple tools in a single response. When multiple independent pieces of information are requested, batch your tool calls together for optimal performance. When making multiple bash tool calls, you MUST send a single message with multiple tools calls to run the calls in parallel. For example, if you need to run \"git status\" and \"git diff\", send a single message with two tool calls to run the calls in parallel.\n- If the user specifies that they want you to run tools \"in parallel\", you MUST send a single message with multiple tool use content blocks. For example, if you need to launch multiple agents in parallel, send a single message with multiple Task tool calls.\n\n\n\n\nHere is useful information about the environment you are running in:\n<env>\nWorking directory: /Users/salmanparacha/arch/crates/llm_gateway\nIs directory a git repo: Yes\nPlatform: darwin\nOS Version: Darwin 25.0.0\nToday's date: 2025-09-25\n</env>\nYou are powered by the model named Sonnet 4. The exact model ID is claude-sonnet-4-20250514.\n\nAssistant knowledge cutoff is January 2025.\n\n\nIMPORTANT: Assist with defensive security tasks only. Refuse to create, modify, or improve code that may be used maliciously. Do not assist with credential discovery or harvesting, including bulk crawling for 2025-09-25T22:19:13.499582010Z SSH keys, browser cookies, or cryptocurrency wallets. Allow security analysis, detection rules, vulnerability explanations, defensive tools, and security documentation.\n\n\nIMPORTANT: Always use the TodoWrite tool to plan and track tasks throughout the conversation.\n\n# Code References\n\nWhen referencing specific functions or pieces of code include the pattern `file_path:line_number` to allow the user to easily navigate to the source code location.\n\n<example>\nuser: Where are errors from the client handled?\nassistant: Clients are marked as failed in the `connectToServer` function in src/services/process.ts:712.\n</example>\n",
                    "cache_control": {
                        "type": "ephemeral"
                    }
                }
            ],
            "tools": [
                {
                    "name": "Task",
                    "description": "Launch a new agent to handle complex, multi-step tasks autonomously. \n\nAvailable agent types and the tools they have access to:\n- general-purpose: General-purpose agent for researching complex questions, searching for code, and executing multi-step tasks. When you are searching for a keyword or file and are not confident that you will find the right match in the first few tries use this agent to perform the search for you. (Tools: *)\n- statusline-setup: Use this agent to configure the user's Claude Code status line setting. (Tools: Read, Edit)\n- output-style-setup: Use this agent to create a Claude Code output style. (Tools: Read, Write, Edit, Glob, Grep)\n\nWhen using the Task tool, you must specify a subagent_type parameter to select which agent type to use.\n\nWhen NOT to use the Agent tool:\n- If you want to read a specific file path, use the Read or Glob tool instead of the Agent tool, to find the match more quickly\n- If you are searching for a specific class definition like \"class Foo\", use the Glob tool instead, to find the match more quickly\n- If you are searching for code within a specific file or set of 2-3 files, use the Read tool instead of the Agent tool, to find the match more quickly\n- Other tasks that are not related to the agent descriptions above\n\n\nUsage notes:\n1. Launch multiple agents concurrently whenever possible, to maximize performance; to do that, use a single message with multiple tool uses\n2. When the agent is done, it will return a single message back to you. The result returned by the agent is not visible to the user. To show the user the result, you should send a text message back to the user with a concise summary of the result.\n3. Each agent invocation is stateless. You will not be able to send additional messages to the agent, nor will the agent be able to communicate with you outside of its final report. Therefore, your prompt should contain a highly detailed task description for the agent to perform autonomously and you should specify exactly what information the agent should return back to you in its final and only message to you.\n4. The agent's outputs should generally be trusted\n5. Clearly tell the agent whether you expect it to write code or just to do research (search, file reads, web fetches, etc.), since it is not aware of the user's intent\n6. If the agent description mentions that it should be used proactively, then you should try your best to use it without the user having to ask for it first. Use your judgement.\n7. If the user specifies that they want you to run agents \"in parallel\", you MUST send a single message with multiple Task tool use content blocks. For example, if you need to launch both a code-reviewer agent and a test-runner agent in parallel, send a single message with both tool calls.\n\nExample usage:\n\n<example_agent_descriptions>\n\"code-reviewer\": use this agent after you are done writing a signficant piece of code\n\"greeting-responder\": use this agent when to respond to user greetings with a friendly joke\n</example_agent_description>\n\n<example>\nuser: \"Please write a function that checks if a number is prime\"\nassistant: Sure let me write a function that checks if a number is prime\nassistant: First let me use the Write tool to write a function that checks if a number is prime\nassistant: I'm going to use the Write tool to write the following code:\n<code>\nfunction isPrime(n) {\n  if (n <= 1) return false\n  for (let i = 2; i * i <= n; i++) {\n    if (n % i === 0) return false\n  }\n  return true\n}\n</code>\n<commentary>\nSince a signficant piece of code was written and the task was completed, now use the code-reviewer agent to review the code\n</commentary>\nassistant: Now let me use the code-reviewer agent to review the code\nassistant: Uses the Task tool to launch the with the code-reviewer agent \n</example>\n\n<example>\nuser: \"Hello\"\n<commentary>\nSince the user is greeting, use the greeting-responder agent to respond with a friendly joke\n</commentary>\nassistant: \"I'm going to use the Task tool to launch the with the greeting-responder agent\"\n</example>\n",
                    "input_schema": {
                        "type": "object",
                        "properties": {
                            "description": {
                                "type": "string",
                                "description": "A short (3-5 word) description of the task"
                            },
                            "prompt": {
                                "type": "string",
                                "description": "The task for the agent to perform"
                            },
                            "subagent_type": {
                                "type": "string",
                                "description": "The type of specialized agent to use for this task"
                            }
                        },
                        "required": [
                            "description",
                            "prompt",
                            "subagent_type"
                        ],
                        "additionalProperties": false,
                        "$schema": "http://json-schema.org/draft-07/schema#"
                    }
                }
            ]
        });

        // Deserialize the complex MessagesRequest
        let deserialized_request: MessagesRequest =
            serde_json::from_value(complex_request_json.clone()).unwrap();

        // Verify basic fields
        assert_eq!(deserialized_request.model, "claude-sonnet-4-20250514");
        assert_eq!(deserialized_request.temperature, Some(1.0));
        assert_eq!(deserialized_request.messages.len(), 3);

        // Verify system message with cache_control
        if let Some(MessagesSystemPrompt::Blocks(ref system_blocks)) = deserialized_request.system {
            assert_eq!(system_blocks.len(), 2);
            if let MessagesContentBlock::Text {
                text,
                cache_control,
            } = &system_blocks[0]
            {
                assert_eq!(
                    text,
                    "You are Claude Code, Anthropic's official CLI for Claude."
                );
                assert_eq!(cache_control, &Some(MessagesCacheControl::Ephemeral));
            } else {
                panic!("Expected text system message with cache_control");
            }
        } else {
            panic!("Expected system blocks");
        }

        // Verify tool_use message with cache_control
        let assistant_message = &deserialized_request.messages[1];
        assert_eq!(assistant_message.role, MessagesRole::Assistant);
        if let MessagesMessageContent::Blocks(ref content_blocks) = assistant_message.content {
            if let MessagesContentBlock::ToolUse {
                id,
                name,
                input,
                cache_control,
            } = &content_blocks[0]
            {
                assert_eq!(id, "call_kV50LtJQKHvvzZui5TW56DUl");
                assert_eq!(name, "TodoWrite");
                assert_eq!(cache_control, &Some(MessagesCacheControl::Ephemeral));
                // Verify the complex input structure
                assert!(input.get("todos").is_some());
                let todos = input.get("todos").unwrap().as_array().unwrap();
                assert_eq!(todos.len(), 3);
            } else {
                panic!("Expected tool_use message with cache_control");
            }
        } else {
            panic!("Expected content blocks in assistant message");
        }

        // Verify tool_result with string content
        let user_message = &deserialized_request.messages[2];
        assert_eq!(user_message.role, MessagesRole::User);
        if let MessagesMessageContent::Blocks(ref content_blocks) = user_message.content {
            if let MessagesContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } = &content_blocks[0]
            {
                assert_eq!(tool_use_id, "call_kV50LtJQKHvvzZui5TW56DUl");
                if let ToolResultContent::Text(text) = content {
                    assert!(text.contains("Todos have been modified successfully"));
                } else {
                    panic!("Expected string content in tool result");
                }
            } else {
                panic!("Expected tool_result message");
            }

            // Verify text content with cache_control
            if let MessagesContentBlock::Text {
                text,
                cache_control,
            } = &content_blocks[2]
            {
                assert_eq!(text, "try again");
                assert_eq!(cache_control, &Some(MessagesCacheControl::Ephemeral));
            } else {
                panic!("Expected text message with cache_control");
            }
        } else {
            panic!("Expected content blocks in user message");
        }

        // Test serialization round-trip
        let serialized_request = serde_json::to_value(&deserialized_request).unwrap();
        let re_deserialized_request: MessagesRequest =
            serde_json::from_value(serialized_request).unwrap();

        // Verify round-trip consistency
        assert_eq!(deserialized_request.model, re_deserialized_request.model);
        assert_eq!(
            deserialized_request.messages.len(),
            re_deserialized_request.messages.len()
        );
    }

    #[test]
    fn test_anthropic_api_provider_trait_implementation() {
        // Test that AnthropicApi implements ApiDefinition trait correctly
        let api = AnthropicApi::Messages;

        // Test trait methods
        assert_eq!(api.endpoint(), MESSAGES_PATH);
        assert!(api.supports_streaming());
        assert!(api.supports_tools());
        assert!(api.supports_vision());

        // Test from_endpoint trait method
        let found_api = AnthropicApi::from_endpoint(MESSAGES_PATH);
        assert_eq!(found_api, Some(AnthropicApi::Messages));

        let not_found = AnthropicApi::from_endpoint("/v1/unknown");
        assert_eq!(not_found, None);

        // Test all_variants
        let all_variants = AnthropicApi::all_variants();
        assert_eq!(all_variants.len(), 1);
        assert_eq!(all_variants[0], AnthropicApi::Messages);
    }

    #[test]
    fn test_anthropic_thinking_streaming() {
        // Test thinking delta stream event
        let thinking_delta_json = json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {
                "type": "thinking_delta",
                "thinking": ".\n\nI need to consider:\n1. Current"
            }
        });

        let deserialized_event: MessagesStreamEvent =
            serde_json::from_value(thinking_delta_json.clone()).unwrap();
        if let MessagesStreamEvent::ContentBlockDelta { index, ref delta } = deserialized_event {
            assert_eq!(index, 0);
            if let MessagesContentDelta::ThinkingDelta { thinking } = delta {
                assert_eq!(thinking, ".\n\nI need to consider:\n1. Current");
            } else {
                panic!("Expected thinking delta");
            }
        } else {
            panic!("Expected content block delta event");
        }

        // Test that thinking delta is returned by content_delta()
        assert_eq!(
            deserialized_event.content_delta(),
            Some(".\n\nI need to consider:\n1. Current")
        );

        let serialized_event_json = serde_json::to_value(&deserialized_event).unwrap();
        assert_eq!(thinking_delta_json, serialized_event_json);
    }

    #[test]
    fn test_anthropic_thinking_request_config() {
        // Test thinking config with budget_tokens
        let request_json = json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [
                {
                    "role": "user",
                    "content": "Test message"
                }
            ],
            "max_tokens": 2048,
            "thinking": {
                "type": "enabled",
                "budget_tokens": 1024
            }
        });

        let deserialized_request: MessagesRequest =
            serde_json::from_value(request_json.clone()).unwrap();
        assert_eq!(deserialized_request.model, "claude-sonnet-4-20250514");
        assert_eq!(deserialized_request.max_tokens, 2048);

        if let Some(thinking) = &deserialized_request.thinking {
            assert_eq!(thinking.thinking_type, "enabled");
            assert_eq!(thinking.budget_tokens, Some(1024));
        } else {
            panic!("Expected thinking config");
        }

        let serialized_json = serde_json::to_value(&deserialized_request).unwrap();
        assert_eq!(request_json, serialized_json);
    }
}
