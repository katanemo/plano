use crate::apis::amazon_bedrock::ConverseResponse;
use crate::apis::anthropic::MessagesResponse;
use crate::apis::openai::ChatCompletionsResponse;
use crate::apis::openai_responses::ResponsesAPIResponse;
use crate::clients::endpoints::SupportedAPIsFromClient;
use crate::clients::endpoints::SupportedUpstreamAPIs;
use crate::providers::id::ProviderId;
use serde::Serialize;
use std::convert::TryFrom;
use std::error::Error;
use std::fmt;

#[derive(Serialize, Debug, Clone)]
#[serde(untagged)]
pub enum ProviderResponseType {
    ChatCompletionsResponse(ChatCompletionsResponse),
    MessagesResponse(MessagesResponse),
    ResponsesAPIResponse(Box<ResponsesAPIResponse>),
}

/// Trait for token usage information
pub trait TokenUsage {
    fn completion_tokens(&self) -> usize;
    fn prompt_tokens(&self) -> usize;
    fn total_tokens(&self) -> usize;
}

pub trait ProviderResponse: Send + Sync {
    /// Get usage information if available - returns dynamic trait object
    fn usage(&self) -> Option<&dyn TokenUsage>;

    /// Extract token counts for metrics
    fn extract_usage_counts(&self) -> Option<(usize, usize, usize)> {
        self.usage()
            .map(|u| (u.prompt_tokens(), u.completion_tokens(), u.total_tokens()))
    }
}

impl ProviderResponse for ProviderResponseType {
    fn usage(&self) -> Option<&dyn TokenUsage> {
        match self {
            ProviderResponseType::ChatCompletionsResponse(resp) => resp.usage(),
            ProviderResponseType::MessagesResponse(resp) => resp.usage(),
            ProviderResponseType::ResponsesAPIResponse(resp) => {
                resp.usage.as_ref().map(|u| u as &dyn TokenUsage)
            }
        }
    }

    fn extract_usage_counts(&self) -> Option<(usize, usize, usize)> {
        match self {
            ProviderResponseType::ChatCompletionsResponse(resp) => resp.extract_usage_counts(),
            ProviderResponseType::MessagesResponse(resp) => resp.extract_usage_counts(),
            ProviderResponseType::ResponsesAPIResponse(resp) => resp.usage.as_ref().map(|u| {
                (
                    u.input_tokens as usize,
                    u.output_tokens as usize,
                    u.total_tokens as usize,
                )
            }),
        }
    }
}

// --- Response transformation logic for client API compatibility ---
impl TryFrom<(&[u8], &SupportedAPIsFromClient, &ProviderId)> for ProviderResponseType {
    type Error = std::io::Error;

    fn try_from(
        (bytes, client_api, provider_id): (&[u8], &SupportedAPIsFromClient, &ProviderId),
    ) -> Result<Self, Self::Error> {
        let upstream_api = provider_id.compatible_api_for_client(client_api, false);
        match (&upstream_api, client_api) {
            (
                SupportedUpstreamAPIs::OpenAIChatCompletions(_),
                SupportedAPIsFromClient::OpenAIChatCompletions(_),
            ) => {
                let resp: ChatCompletionsResponse = ChatCompletionsResponse::try_from(bytes)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                Ok(ProviderResponseType::ChatCompletionsResponse(resp))
            }
            (
                SupportedUpstreamAPIs::AnthropicMessagesAPI(_),
                SupportedAPIsFromClient::AnthropicMessagesAPI(_),
            ) => {
                let resp: MessagesResponse = serde_json::from_slice(bytes)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                Ok(ProviderResponseType::MessagesResponse(resp))
            }
            (
                SupportedUpstreamAPIs::AnthropicMessagesAPI(_),
                SupportedAPIsFromClient::OpenAIChatCompletions(_),
            ) => {
                let anthropic_resp: MessagesResponse = serde_json::from_slice(bytes)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

                // Transform to OpenAI ChatCompletions format using the transformer
                let chat_resp: ChatCompletionsResponse =
                    anthropic_resp.try_into().map_err(|e| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("Transformation error: {}", e),
                        )
                    })?;
                Ok(ProviderResponseType::ChatCompletionsResponse(chat_resp))
            }
            (
                SupportedUpstreamAPIs::OpenAIChatCompletions(_),
                SupportedAPIsFromClient::AnthropicMessagesAPI(_),
            ) => {
                let openai_resp: ChatCompletionsResponse = ChatCompletionsResponse::try_from(bytes)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

                // Transform to Anthropic Messages format using the transformer
                let messages_resp: MessagesResponse = openai_resp.try_into().map_err(|e| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("Transformation error: {}", e),
                    )
                })?;
                Ok(ProviderResponseType::MessagesResponse(messages_resp))
            }
            // Amazon Bedrock transformations
            (
                SupportedUpstreamAPIs::AmazonBedrockConverse(_),
                SupportedAPIsFromClient::OpenAIChatCompletions(_),
            ) => {
                let bedrock_resp: ConverseResponse = serde_json::from_slice(bytes)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

                // Transform to OpenAI ChatCompletions format using the transformer
                let chat_resp: ChatCompletionsResponse = bedrock_resp.try_into().map_err(|e| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("Transformation error: {}", e),
                    )
                })?;
                Ok(ProviderResponseType::ChatCompletionsResponse(chat_resp))
            }
            (
                SupportedUpstreamAPIs::AmazonBedrockConverse(_),
                SupportedAPIsFromClient::AnthropicMessagesAPI(_),
            ) => {
                let bedrock_resp: ConverseResponse = serde_json::from_slice(bytes)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

                // Transform to Anthropic Messages format using the transformer
                let messages_resp: MessagesResponse = bedrock_resp.try_into().map_err(|e| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("Transformation error: {}", e),
                    )
                })?;
                Ok(ProviderResponseType::MessagesResponse(messages_resp))
            }
            (
                SupportedUpstreamAPIs::OpenAIResponsesAPI(_),
                SupportedAPIsFromClient::OpenAIResponsesAPI(_),
            ) => {
                let resp: ResponsesAPIResponse = ResponsesAPIResponse::try_from(bytes)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                Ok(ProviderResponseType::ResponsesAPIResponse(Box::new(resp)))
            }
            (
                SupportedUpstreamAPIs::OpenAIChatCompletions(_),
                SupportedAPIsFromClient::OpenAIResponsesAPI(_),
            ) => {
                let chat_completions_response: ChatCompletionsResponse =
                    ChatCompletionsResponse::try_from(bytes)
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

                // Transform to ResponsesAPI format using the transformer
                let responses_resp: ResponsesAPIResponse =
                    chat_completions_response.try_into().map_err(|e| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("Transformation error: {}", e),
                        )
                    })?;
                Ok(ProviderResponseType::ResponsesAPIResponse(Box::new(
                    responses_resp,
                )))
            }
            (
                SupportedUpstreamAPIs::AnthropicMessagesAPI(_),
                SupportedAPIsFromClient::OpenAIResponsesAPI(_),
            ) => {
                //Chain transform: Anthropic Messages -> OpenAI ChatCompletions -> ResponsesAPI
                let anthropic_resp: MessagesResponse = serde_json::from_slice(bytes)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

                // Transform to ChatCompletions format using the transformer
                let chat_resp: ChatCompletionsResponse =
                    anthropic_resp.try_into().map_err(|e| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("Transformation error: {}", e),
                        )
                    })?;

                let response_api: ResponsesAPIResponse = chat_resp.try_into().map_err(|e| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("Transformation error: {}", e),
                    )
                })?;
                Ok(ProviderResponseType::ResponsesAPIResponse(Box::new(
                    response_api,
                )))
            }
            (
                SupportedUpstreamAPIs::AmazonBedrockConverse(_),
                SupportedAPIsFromClient::OpenAIResponsesAPI(_),
            ) => {
                // Chain transform: Bedrock Converse -> ChatCompletions -> ResponsesAPI
                let bedrock_resp: ConverseResponse = serde_json::from_slice(bytes)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

                // Transform to ChatCompletions format
                let chat_resp: ChatCompletionsResponse = bedrock_resp.try_into().map_err(|e| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("Bedrock to ChatCompletions transformation error: {}", e),
                    )
                })?;

                // Transform to ResponsesAPI format
                let response_api: ResponsesAPIResponse = chat_resp.try_into().map_err(|e| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!(
                            "ChatCompletions to ResponsesAPI transformation error: {}",
                            e
                        ),
                    )
                })?;
                Ok(ProviderResponseType::ResponsesAPIResponse(Box::new(
                    response_api,
                )))
            }
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Unsupported API combination for response transformation",
            )),
        }
    }
}

#[derive(Debug)]
pub struct ProviderResponseError {
    pub message: String,
    pub source: Option<Box<dyn Error + Send + Sync>>,
}

impl fmt::Display for ProviderResponseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Provider response error: {}", self.message)
    }
}

impl Error for ProviderResponseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e.as_ref() as &(dyn Error + 'static))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apis::anthropic::AnthropicApi;
    use crate::apis::openai::OpenAIApi;
    use crate::clients::endpoints::SupportedAPIsFromClient;
    use crate::providers::id::ProviderId;
    use serde_json::json;

    #[test]
    fn test_openai_response_from_bytes() {
        let resp = json!({
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "gpt-4",
            "choices": [
                {
                    "index": 0,
                    "message": { "role": "assistant", "content": "Hello!" },
                    "finish_reason": "stop"
                }
            ],
            "usage": { "prompt_tokens": 5, "completion_tokens": 7, "total_tokens": 12 },
            "system_fingerprint": null
        });
        let bytes = serde_json::to_vec(&resp).unwrap();
        let result = ProviderResponseType::try_from((
            bytes.as_slice(),
            &SupportedAPIsFromClient::OpenAIChatCompletions(OpenAIApi::ChatCompletions),
            &ProviderId::OpenAI,
        ));
        assert!(result.is_ok());
        match result.unwrap() {
            ProviderResponseType::ChatCompletionsResponse(r) => {
                assert_eq!(r.model, "gpt-4");
                assert_eq!(r.choices.len(), 1);
            }
            _ => panic!("Expected ChatCompletionsResponse variant"),
        }
    }

    #[test]
    fn test_anthropic_response_from_bytes() {
        let resp = json!({
            "id": "msg_01ABC123",
            "type": "message",
            "role": "assistant",
            "content": [
                { "type": "text", "text": "Hello! How can I help you today?" }
            ],
            "model": "claude-3-sonnet-20240229",
            "stop_reason": "end_turn",
            "usage": { "input_tokens": 10, "output_tokens": 25, "cache_creation_input_tokens": 5, "cache_read_input_tokens": 3 }
        });
        let bytes = serde_json::to_vec(&resp).unwrap();
        let result = ProviderResponseType::try_from((
            bytes.as_slice(),
            &SupportedAPIsFromClient::AnthropicMessagesAPI(AnthropicApi::Messages),
            &ProviderId::Anthropic,
        ));
        assert!(result.is_ok());
        match result.unwrap() {
            ProviderResponseType::MessagesResponse(r) => {
                assert_eq!(r.model, "claude-3-sonnet-20240229");
                assert_eq!(r.content.len(), 1);
            }
            _ => panic!("Expected MessagesResponse variant"),
        }
    }

    #[test]
    fn test_anthropic_response_from_bytes_with_openai_provider() {
        // OpenAI provider receives OpenAI response but client expects Anthropic format
        // Upstream API = OpenAI, Client API = Anthropic -> parse OpenAI, convert to Anthropic
        let resp = json!({
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "gpt-4",
            "choices": [
                {
                    "index": 0,
                    "message": { "role": "assistant", "content": "Hello! How can I help you today?" },
                    "finish_reason": "stop"
                }
            ],
            "usage": { "prompt_tokens": 10, "completion_tokens": 25, "total_tokens": 35 }
        });
        let bytes = serde_json::to_vec(&resp).unwrap();
        let result = ProviderResponseType::try_from((
            bytes.as_slice(),
            &SupportedAPIsFromClient::AnthropicMessagesAPI(AnthropicApi::Messages),
            &ProviderId::OpenAI,
        ));
        assert!(result.is_ok());
        match result.unwrap() {
            ProviderResponseType::MessagesResponse(r) => {
                assert_eq!(r.model, "gpt-4");
                assert_eq!(r.usage.input_tokens, 10);
                assert_eq!(r.usage.output_tokens, 25);
            }
            _ => panic!("Expected MessagesResponse variant"),
        }
    }

    #[test]
    fn test_openai_response_from_bytes_with_claude_provider() {
        // Claude provider using OpenAI-compatible API returns OpenAI format response
        // Client API = OpenAI, Provider = Anthropic -> Anthropic returns OpenAI format via their compatible API
        let resp = json!({
            "id": "chatcmpl-01ABC123",
            "object": "chat.completion",
            "created": 1677652288,
            "model": "claude-3-sonnet-20240229",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "Hello! How can I help you today?"
                    },
                    "finish_reason": "stop"
                }
            ],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 25,
                "total_tokens": 35
            }
        });
        let bytes = serde_json::to_vec(&resp).unwrap();
        let result = ProviderResponseType::try_from((
            bytes.as_slice(),
            &SupportedAPIsFromClient::OpenAIChatCompletions(OpenAIApi::ChatCompletions),
            &ProviderId::Anthropic,
        ));
        assert!(result.is_ok());
        match result.unwrap() {
            ProviderResponseType::ChatCompletionsResponse(r) => {
                assert_eq!(r.model, "claude-3-sonnet-20240229");
                assert_eq!(r.usage.prompt_tokens, 10);
                assert_eq!(r.usage.completion_tokens, 25);
            }
            _ => panic!("Expected ChatCompletionsResponse variant"),
        }
    }
}
