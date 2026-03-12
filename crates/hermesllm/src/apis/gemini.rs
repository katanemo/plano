use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_with::skip_serializing_none;
use std::collections::HashMap;

use super::ApiDefinition;
use crate::providers::request::{ProviderRequest, ProviderRequestError};
use crate::providers::response::TokenUsage;
use crate::providers::streaming_response::ProviderStreamResponse;
use crate::transforms::lib::ExtractText;
use crate::GENERATE_CONTENT_PATH_SUFFIX;

// ============================================================================
// GEMINI GENERATE CONTENT API ENUMERATION
// ============================================================================

/// Enum for all supported Gemini GenerateContent APIs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GeminiApi {
    GenerateContent,
    StreamGenerateContent,
}

impl ApiDefinition for GeminiApi {
    fn endpoint(&self) -> &'static str {
        match self {
            GeminiApi::GenerateContent => ":generateContent",
            GeminiApi::StreamGenerateContent => ":streamGenerateContent",
        }
    }

    fn from_endpoint(endpoint: &str) -> Option<Self> {
        if endpoint.ends_with(":streamGenerateContent") {
            Some(GeminiApi::StreamGenerateContent)
        } else if endpoint.ends_with(GENERATE_CONTENT_PATH_SUFFIX) {
            Some(GeminiApi::GenerateContent)
        } else {
            None
        }
    }

    fn supports_streaming(&self) -> bool {
        match self {
            GeminiApi::GenerateContent => false,
            GeminiApi::StreamGenerateContent => true,
        }
    }

    fn supports_tools(&self) -> bool {
        true
    }

    fn supports_vision(&self) -> bool {
        true
    }

    fn all_variants() -> Vec<Self> {
        vec![GeminiApi::GenerateContent, GeminiApi::StreamGenerateContent]
    }
}

// ============================================================================
// REQUEST TYPES
// ============================================================================

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GenerateContentRequest {
    /// Internal model field — not part of Gemini wire format (model is in the URL).
    /// Populated during parsing and used for routing.
    #[serde(skip_serializing, default)]
    pub model: String,

    pub contents: Vec<Content>,
    pub generation_config: Option<GenerationConfig>,
    pub tools: Option<Vec<Tool>>,
    pub tool_config: Option<ToolConfig>,
    pub safety_settings: Option<Vec<SafetySetting>>,
    pub system_instruction: Option<Content>,
    pub cached_content: Option<String>,

    #[serde(skip_serializing)]
    pub metadata: Option<HashMap<String, Value>>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Content {
    pub role: Option<String>,
    pub parts: Vec<Part>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    pub text: Option<String>,
    pub inline_data: Option<InlineData>,
    pub function_call: Option<FunctionCall>,
    pub function_response: Option<FunctionResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineData {
    pub mime_type: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionCall {
    pub name: String,
    pub args: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionResponse {
    pub name: String,
    pub response: Value,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GenerationConfig {
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub stop_sequences: Option<Vec<String>>,
    pub response_mime_type: Option<String>,
    pub candidate_count: Option<u32>,
    pub presence_penalty: Option<f32>,
    pub frequency_penalty: Option<f32>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    pub function_declarations: Option<Vec<FunctionDeclaration>>,
    pub code_execution: Option<Value>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionDeclaration {
    pub name: String,
    pub description: Option<String>,
    pub parameters: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolConfig {
    pub function_calling_config: FunctionCallingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionCallingConfig {
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafetySetting {
    pub category: String,
    pub threshold: String,
}

// ============================================================================
// RESPONSE TYPES
// ============================================================================

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GenerateContentResponse {
    pub candidates: Option<Vec<Candidate>>,
    pub usage_metadata: Option<UsageMetadata>,
    pub prompt_feedback: Option<PromptFeedback>,
    pub model_version: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub content: Option<Content>,
    pub finish_reason: Option<String>,
    pub safety_ratings: Option<Vec<SafetyRating>>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UsageMetadata {
    pub prompt_token_count: Option<u32>,
    pub candidates_token_count: Option<u32>,
    pub total_token_count: Option<u32>,
}

impl TokenUsage for UsageMetadata {
    fn completion_tokens(&self) -> usize {
        self.candidates_token_count.unwrap_or(0) as usize
    }

    fn prompt_tokens(&self) -> usize {
        self.prompt_token_count.unwrap_or(0) as usize
    }

    fn total_tokens(&self) -> usize {
        self.total_token_count.unwrap_or(0) as usize
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptFeedback {
    pub block_reason: Option<String>,
    pub safety_ratings: Option<Vec<SafetyRating>>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafetyRating {
    pub category: String,
    pub probability: String,
    pub blocked: Option<bool>,
}

// ============================================================================
// PROVIDER REQUEST TRAIT IMPLEMENTATION
// ============================================================================

impl ProviderRequest for GenerateContentRequest {
    fn model(&self) -> &str {
        &self.model
    }

    fn set_model(&mut self, model: String) {
        self.model = model;
    }

    fn is_streaming(&self) -> bool {
        // Gemini uses URL-based streaming, not a field in the request body
        false
    }

    fn extract_messages_text(&self) -> String {
        let mut parts_text = Vec::new();
        for content in &self.contents {
            for part in &content.parts {
                if let Some(text) = &part.text {
                    parts_text.push(text.clone());
                }
            }
        }
        if let Some(system) = &self.system_instruction {
            for part in &system.parts {
                if let Some(text) = &part.text {
                    parts_text.push(text.clone());
                }
            }
        }
        parts_text.join(" ")
    }

    fn get_recent_user_message(&self) -> Option<String> {
        self.contents
            .iter()
            .rev()
            .find(|c| c.role.as_deref() == Some("user"))
            .and_then(|c| {
                c.parts
                    .iter()
                    .filter_map(|p| p.text.clone())
                    .collect::<Vec<_>>()
                    .first()
                    .cloned()
            })
    }

    fn get_tool_names(&self) -> Option<Vec<String>> {
        self.tools.as_ref().map(|tools| {
            tools
                .iter()
                .filter_map(|t| t.function_declarations.as_ref())
                .flatten()
                .map(|f| f.name.clone())
                .collect()
        })
    }

    fn to_bytes(&self) -> Result<Vec<u8>, ProviderRequestError> {
        serde_json::to_vec(self).map_err(|e| ProviderRequestError {
            message: format!("Failed to serialize GenerateContentRequest: {}", e),
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
        self.generation_config
            .as_ref()
            .and_then(|gc| gc.temperature)
    }

    fn get_messages(&self) -> Vec<crate::apis::openai::Message> {
        use crate::apis::openai::{Message, MessageContent, Role};

        let mut messages = Vec::new();

        // Convert system instruction
        if let Some(system) = &self.system_instruction {
            let text = system
                .parts
                .iter()
                .filter_map(|p| p.text.clone())
                .collect::<Vec<_>>()
                .join("");
            if !text.is_empty() {
                messages.push(Message {
                    role: Role::System,
                    content: Some(MessageContent::Text(text)),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                });
            }
        }

        // Convert contents
        for content in &self.contents {
            let role = match content.role.as_deref() {
                Some("model") => Role::Assistant,
                _ => Role::User,
            };

            let text = content
                .parts
                .iter()
                .filter_map(|p| p.text.clone())
                .collect::<Vec<_>>()
                .join("");

            messages.push(Message {
                role,
                content: Some(MessageContent::Text(text)),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            });
        }

        messages
    }

    fn set_messages(&mut self, messages: &[crate::apis::openai::Message]) {
        use crate::apis::openai::Role;

        self.contents.clear();
        self.system_instruction = None;

        for msg in messages {
            let text = msg.content.extract_text();
            match msg.role {
                Role::System => {
                    self.system_instruction = Some(Content {
                        role: Some("user".to_string()),
                        parts: vec![Part {
                            text: Some(text),
                            inline_data: None,
                            function_call: None,
                            function_response: None,
                        }],
                    });
                }
                Role::User => {
                    self.contents.push(Content {
                        role: Some("user".to_string()),
                        parts: vec![Part {
                            text: Some(text),
                            inline_data: None,
                            function_call: None,
                            function_response: None,
                        }],
                    });
                }
                Role::Assistant => {
                    self.contents.push(Content {
                        role: Some("model".to_string()),
                        parts: vec![Part {
                            text: Some(text),
                            inline_data: None,
                            function_call: None,
                            function_response: None,
                        }],
                    });
                }
                Role::Tool => {
                    self.contents.push(Content {
                        role: Some("user".to_string()),
                        parts: vec![Part {
                            text: Some(text),
                            inline_data: None,
                            function_call: None,
                            function_response: None,
                        }],
                    });
                }
            }
        }
    }
}

// ============================================================================
// PROVIDER STREAM RESPONSE TRAIT IMPLEMENTATION
// ============================================================================

impl ProviderStreamResponse for GenerateContentResponse {
    fn content_delta(&self) -> Option<&str> {
        self.candidates
            .as_ref()
            .and_then(|candidates| candidates.first())
            .and_then(|candidate| candidate.content.as_ref())
            .and_then(|content| content.parts.first())
            .and_then(|part| part.text.as_deref())
    }

    fn is_final(&self) -> bool {
        self.candidates
            .as_ref()
            .and_then(|candidates| candidates.first())
            .and_then(|candidate| candidate.finish_reason.as_deref())
            .map(|reason| reason == "STOP" || reason == "MAX_TOKENS" || reason == "SAFETY")
            .unwrap_or(false)
    }

    fn role(&self) -> Option<&str> {
        self.candidates
            .as_ref()
            .and_then(|candidates| candidates.first())
            .and_then(|candidate| candidate.content.as_ref())
            .and_then(|content| content.role.as_deref())
    }

    fn event_type(&self) -> Option<&str> {
        None // Gemini doesn't use SSE event types
    }
}

// ============================================================================
// SERDE PARSING
// ============================================================================

impl TryFrom<&[u8]> for GenerateContentRequest {
    type Error = serde_json::Error;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        serde_json::from_slice(bytes)
    }
}

impl TryFrom<&[u8]> for GenerateContentResponse {
    type Error = serde_json::Error;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        serde_json::from_slice(bytes)
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_gemini_api_from_endpoint() {
        assert_eq!(
            GeminiApi::from_endpoint("/v1beta/models/gemini-pro:generateContent"),
            Some(GeminiApi::GenerateContent)
        );
        assert_eq!(
            GeminiApi::from_endpoint("/v1beta/models/gemini-pro:streamGenerateContent"),
            Some(GeminiApi::StreamGenerateContent)
        );
        assert_eq!(GeminiApi::from_endpoint("/v1/chat/completions"), None);
    }

    #[test]
    fn test_generate_content_request_serde() {
        let json_str = json!({
            "contents": [{
                "role": "user",
                "parts": [{"text": "Hello"}]
            }],
            "generationConfig": {
                "temperature": 0.7,
                "maxOutputTokens": 1024
            }
        });

        let req: GenerateContentRequest = serde_json::from_value(json_str).unwrap();
        assert_eq!(req.contents.len(), 1);
        assert_eq!(req.contents[0].role, Some("user".to_string()));
        assert_eq!(
            req.generation_config.as_ref().unwrap().temperature,
            Some(0.7)
        );
        assert_eq!(
            req.generation_config.as_ref().unwrap().max_output_tokens,
            Some(1024)
        );

        // Roundtrip
        let bytes = serde_json::to_vec(&req).unwrap();
        let req2: GenerateContentRequest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(req2.contents.len(), 1);
    }

    #[test]
    fn test_generate_content_response_serde() {
        let json_str = json!({
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [{"text": "Hello! How can I help?"}]
                },
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 5,
                "candidatesTokenCount": 7,
                "totalTokenCount": 12
            }
        });

        let resp: GenerateContentResponse = serde_json::from_value(json_str).unwrap();
        assert!(resp.candidates.is_some());
        let candidates = resp.candidates.as_ref().unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].finish_reason.as_deref(), Some("STOP"));
        assert_eq!(
            resp.usage_metadata.as_ref().unwrap().prompt_token_count,
            Some(5)
        );
    }

    #[test]
    fn test_generate_content_request_with_tools() {
        let json_str = json!({
            "contents": [{
                "role": "user",
                "parts": [{"text": "What's the weather?"}]
            }],
            "tools": [{
                "functionDeclarations": [{
                    "name": "get_weather",
                    "description": "Get weather info",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "location": {"type": "string"}
                        }
                    }
                }]
            }],
            "toolConfig": {
                "functionCallingConfig": {
                    "mode": "AUTO"
                }
            }
        });

        let req: GenerateContentRequest = serde_json::from_value(json_str).unwrap();
        assert!(req.tools.is_some());
        let tools = req.tools.as_ref().unwrap();
        assert_eq!(tools.len(), 1);
        let decls = tools[0].function_declarations.as_ref().unwrap();
        assert_eq!(decls[0].name, "get_weather");
        assert_eq!(
            req.tool_config
                .as_ref()
                .unwrap()
                .function_calling_config
                .mode,
            "AUTO"
        );
    }

    #[test]
    fn test_generate_content_response_with_function_call() {
        let json_str = json!({
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [{
                        "functionCall": {
                            "name": "get_weather",
                            "args": {"location": "NYC"}
                        }
                    }]
                },
                "finishReason": "STOP"
            }]
        });

        let resp: GenerateContentResponse = serde_json::from_value(json_str).unwrap();
        let candidates = resp.candidates.as_ref().unwrap();
        let parts = &candidates[0].content.as_ref().unwrap().parts;
        assert!(parts[0].function_call.is_some());
        assert_eq!(parts[0].function_call.as_ref().unwrap().name, "get_weather");
    }

    #[test]
    fn test_stream_response_content_delta() {
        let resp = GenerateContentResponse {
            candidates: Some(vec![Candidate {
                content: Some(Content {
                    role: Some("model".to_string()),
                    parts: vec![Part {
                        text: Some("Hello".to_string()),
                        inline_data: None,
                        function_call: None,
                        function_response: None,
                    }],
                }),
                finish_reason: None,
                safety_ratings: None,
            }]),
            usage_metadata: None,
            prompt_feedback: None,
            model_version: None,
        };

        assert_eq!(resp.content_delta(), Some("Hello"));
        assert!(!resp.is_final());
    }

    #[test]
    fn test_stream_response_is_final() {
        let resp = GenerateContentResponse {
            candidates: Some(vec![Candidate {
                content: Some(Content {
                    role: Some("model".to_string()),
                    parts: vec![Part {
                        text: Some("Done".to_string()),
                        inline_data: None,
                        function_call: None,
                        function_response: None,
                    }],
                }),
                finish_reason: Some("STOP".to_string()),
                safety_ratings: None,
            }]),
            usage_metadata: None,
            prompt_feedback: None,
            model_version: None,
        };

        assert!(resp.is_final());
    }

    #[test]
    fn test_provider_request_extract_text() {
        let req = GenerateContentRequest {
            model: "gemini-pro".to_string(),
            contents: vec![Content {
                role: Some("user".to_string()),
                parts: vec![Part {
                    text: Some("Hello world".to_string()),
                    inline_data: None,
                    function_call: None,
                    function_response: None,
                }],
            }],
            system_instruction: Some(Content {
                role: Some("user".to_string()),
                parts: vec![Part {
                    text: Some("Be helpful".to_string()),
                    inline_data: None,
                    function_call: None,
                    function_response: None,
                }],
            }),
            ..Default::default()
        };

        let text = req.extract_messages_text();
        assert!(text.contains("Hello world"));
        assert!(text.contains("Be helpful"));
    }

    #[test]
    fn test_provider_request_get_tool_names() {
        let req = GenerateContentRequest {
            model: "gemini-pro".to_string(),
            contents: vec![],
            tools: Some(vec![Tool {
                function_declarations: Some(vec![
                    FunctionDeclaration {
                        name: "func_a".to_string(),
                        description: None,
                        parameters: None,
                    },
                    FunctionDeclaration {
                        name: "func_b".to_string(),
                        description: None,
                        parameters: None,
                    },
                ]),
                code_execution: None,
            }]),
            ..Default::default()
        };

        let names = req.get_tool_names().unwrap();
        assert_eq!(names, vec!["func_a", "func_b"]);
    }
}
