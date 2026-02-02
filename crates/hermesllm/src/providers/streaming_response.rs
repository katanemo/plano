use serde::Serialize;
use std::convert::TryFrom;

use crate::apis::amazon_bedrock::ConverseStreamEvent;
use crate::apis::anthropic::MessagesStreamEvent;
use crate::apis::openai::ChatCompletionsStreamResponse;
use crate::apis::openai_responses::ResponsesAPIStreamEvent;
use crate::apis::streaming_shapes::sse::SseEvent;
use crate::apis::streaming_shapes::sse::SseStreamBuffer;
use crate::apis::streaming_shapes::{
    anthropic_streaming_buffer::AnthropicMessagesStreamBuffer,
    chat_completions_streaming_buffer::OpenAIChatCompletionsStreamBuffer,
    passthrough_streaming_buffer::PassthroughStreamBuffer,
};

use crate::clients::endpoints::SupportedAPIsFromClient;
use crate::clients::endpoints::SupportedUpstreamAPIs;

// ============================================================================
// SSE STREAM BUFFER FACTORY
// ============================================================================

/// Check if streaming buffering is needed based on client and upstream API combination.
pub fn needs_buffering(
    client_api: &SupportedAPIsFromClient,
    upstream_api: &SupportedUpstreamAPIs,
) -> bool {
    match (client_api, upstream_api) {
        // Same APIs - no buffering needed
        (
            SupportedAPIsFromClient::OpenAIChatCompletions(_),
            SupportedUpstreamAPIs::OpenAIChatCompletions(_),
        ) => false,
        (
            SupportedAPIsFromClient::AnthropicMessagesAPI(_),
            SupportedUpstreamAPIs::AnthropicMessagesAPI(_),
        ) => false,
        (
            SupportedAPIsFromClient::OpenAIResponsesAPI(_),
            SupportedUpstreamAPIs::OpenAIResponsesAPI(_),
        ) => false,

        // Different APIs - buffering needed
        _ => true,
    }
}

/// Factory pattern for creating SSE stream buffers based on client and upstream API combination.
/// # Example
/// ```ignore
/// use hermesllm::clients::endpoints::{SupportedAPIsFromClient, SupportedUpstreamAPIs};
/// use hermesllm::apis::streaming_shapes::sse::SseStreamBuffer;
///
/// // Transformation needed: OpenAI upstream -> Anthropic client
/// let mut buffer = SseStreamBuffer::try_from((&client_api, &upstream_api))?;
///
/// // Add transformed events
/// let transformed = SseEvent::try_from((raw_event, &client_api, &upstream_api))?;
/// buffer.add_transformed_event(transformed);
///
/// // Flush to wire
/// let bytes = buffer.into_bytes();
/// ```
impl TryFrom<(&SupportedAPIsFromClient, &SupportedUpstreamAPIs)> for SseStreamBuffer {
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn try_from(
        (client_api, upstream_api): (&SupportedAPIsFromClient, &SupportedUpstreamAPIs),
    ) -> Result<Self, Self::Error> {
        // If APIs match, use passthrough - no buffering/transformation needed
        if !needs_buffering(client_api, upstream_api) {
            return Ok(SseStreamBuffer::Passthrough(PassthroughStreamBuffer::new()));
        }

        // APIs differ - use appropriate buffer for client API
        match client_api {
            SupportedAPIsFromClient::OpenAIChatCompletions(_) => Ok(
                SseStreamBuffer::OpenAIChatCompletions(OpenAIChatCompletionsStreamBuffer::new()),
            ),
            SupportedAPIsFromClient::AnthropicMessagesAPI(_) => Ok(
                SseStreamBuffer::AnthropicMessages(AnthropicMessagesStreamBuffer::new()),
            ),
            SupportedAPIsFromClient::OpenAIResponsesAPI(_) => {
                Ok(SseStreamBuffer::OpenAIResponses(Box::default()))
            }
        }
    }
}

// ============================================================================
// PROVIDER STREAM RESPONSE TYPES
// ============================================================================

#[derive(Serialize, Debug, Clone)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)]
pub enum ProviderStreamResponseType {
    ChatCompletionsStreamResponse(ChatCompletionsStreamResponse),
    MessagesStreamEvent(MessagesStreamEvent),
    ConverseStreamEvent(ConverseStreamEvent),
    ResponseAPIStreamEvent(Box<ResponsesAPIStreamEvent>),
}

pub trait ProviderStreamResponse: Send + Sync {
    /// Get the content delta for this chunk
    fn content_delta(&self) -> Option<&str>;

    /// Check if this is the final chunk in the stream
    fn is_final(&self) -> bool;

    /// Get role information if available
    fn role(&self) -> Option<&str>;

    /// Get event type for SSE streaming (used by Anthropic)
    fn event_type(&self) -> Option<&str>;
}

impl ProviderStreamResponse for ProviderStreamResponseType {
    fn content_delta(&self) -> Option<&str> {
        match self {
            ProviderStreamResponseType::ChatCompletionsStreamResponse(resp) => resp.content_delta(),
            ProviderStreamResponseType::MessagesStreamEvent(resp) => resp.content_delta(),
            ProviderStreamResponseType::ConverseStreamEvent(resp) => resp.content_delta(),
            ProviderStreamResponseType::ResponseAPIStreamEvent(_resp) => None, // ResponsesAPI does not have content deltas
        }
    }

    fn is_final(&self) -> bool {
        match self {
            ProviderStreamResponseType::ChatCompletionsStreamResponse(resp) => resp.is_final(),
            ProviderStreamResponseType::MessagesStreamEvent(resp) => resp.is_final(),
            ProviderStreamResponseType::ConverseStreamEvent(resp) => resp.is_final(),
            ProviderStreamResponseType::ResponseAPIStreamEvent(resp) => resp.is_final(),
        }
    }

    fn role(&self) -> Option<&str> {
        match self {
            ProviderStreamResponseType::ChatCompletionsStreamResponse(resp) => resp.role(),
            ProviderStreamResponseType::MessagesStreamEvent(resp) => resp.role(),
            ProviderStreamResponseType::ConverseStreamEvent(resp) => resp.role(),
            ProviderStreamResponseType::ResponseAPIStreamEvent(resp) => resp.role(),
        }
    }

    fn event_type(&self) -> Option<&str> {
        match self {
            ProviderStreamResponseType::ChatCompletionsStreamResponse(_resp) => None, // OpenAI doesn't use event types
            ProviderStreamResponseType::MessagesStreamEvent(resp) => resp.event_type(),
            ProviderStreamResponseType::ConverseStreamEvent(resp) => resp.event_type(), // Bedrock doesn't use event types
            ProviderStreamResponseType::ResponseAPIStreamEvent(resp) => resp.event_type(),
        }
    }
}

impl From<ProviderStreamResponseType> for String {
    fn from(val: ProviderStreamResponseType) -> String {
        match val {
            ProviderStreamResponseType::MessagesStreamEvent(event) => {
                // Use the Into<String> implementation for proper SSE formatting with event lines
                event.into()
            }
            ProviderStreamResponseType::ConverseStreamEvent(event) => {
                // Use the Into<String> implementation for proper SSE formatting with event lines
                event.into()
            }
            ProviderStreamResponseType::ResponseAPIStreamEvent(event) => {
                // Use the Into<String> implementation for proper SSE formatting with event lines
                // Clone to work around Box<T> ownership
                let cloned = (*event).clone();
                cloned.into()
            }
            ProviderStreamResponseType::ChatCompletionsStreamResponse(_) => {
                // For OpenAI, use simple data line format
                let json = serde_json::to_string(&val).unwrap_or_default();
                format!("data: {}\n\n", json)
            }
        }
    }
}

// Stream response transformation logic for client API compatibility
impl TryFrom<(&[u8], &SupportedAPIsFromClient, &SupportedUpstreamAPIs)>
    for ProviderStreamResponseType
{
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn try_from(
        (bytes, client_api, upstream_api): (
            &[u8],
            &SupportedAPIsFromClient,
            &SupportedUpstreamAPIs,
        ),
    ) -> Result<Self, Self::Error> {
        // Special case: Handle [DONE] marker for OpenAI -> Anthropic conversion
        if bytes == b"[DONE]"
            && matches!(client_api, SupportedAPIsFromClient::AnthropicMessagesAPI(_))
        {
            return Ok(ProviderStreamResponseType::MessagesStreamEvent(
                crate::apis::anthropic::MessagesStreamEvent::MessageStop,
            ));
        }
        match (upstream_api, client_api) {
            // OpenAI upstream
            (
                SupportedUpstreamAPIs::OpenAIChatCompletions(_),
                SupportedAPIsFromClient::OpenAIChatCompletions(_),
            ) => {
                let resp = serde_json::from_slice(bytes)?;
                Ok(ProviderStreamResponseType::ChatCompletionsStreamResponse(
                    resp,
                ))
            }
            (
                SupportedUpstreamAPIs::OpenAIChatCompletions(_),
                SupportedAPIsFromClient::AnthropicMessagesAPI(_),
            ) => {
                let openai_resp: crate::apis::openai::ChatCompletionsStreamResponse =
                    serde_json::from_slice(bytes)?;
                let anthropic_resp = openai_resp.try_into()?;
                Ok(ProviderStreamResponseType::MessagesStreamEvent(
                    anthropic_resp,
                ))
            }
            (
                SupportedUpstreamAPIs::OpenAIChatCompletions(_),
                SupportedAPIsFromClient::OpenAIResponsesAPI(_),
            ) => {
                let openai_resp: crate::apis::openai::ChatCompletionsStreamResponse =
                    serde_json::from_slice(bytes)?;
                let responses_resp: ResponsesAPIStreamEvent = openai_resp.try_into()?;
                Ok(ProviderStreamResponseType::ResponseAPIStreamEvent(
                    Box::new(responses_resp),
                ))
            }

            // OpenAI ResponsesAPI upstream
            (
                SupportedUpstreamAPIs::OpenAIResponsesAPI(_),
                SupportedAPIsFromClient::OpenAIResponsesAPI(_),
            ) => {
                let resp = serde_json::from_slice(bytes)?;
                Ok(ProviderStreamResponseType::ResponseAPIStreamEvent(resp))
            }
            // Anthropic upstream
            (
                SupportedUpstreamAPIs::AnthropicMessagesAPI(_),
                SupportedAPIsFromClient::AnthropicMessagesAPI(_),
            ) => {
                let resp = serde_json::from_slice(bytes)?;
                Ok(ProviderStreamResponseType::MessagesStreamEvent(resp))
            }
            (
                SupportedUpstreamAPIs::AnthropicMessagesAPI(_),
                SupportedAPIsFromClient::OpenAIChatCompletions(_),
            ) => {
                let anthropic_resp: crate::apis::anthropic::MessagesStreamEvent =
                    serde_json::from_slice(bytes)?;
                let openai_resp = anthropic_resp.try_into()?;
                Ok(ProviderStreamResponseType::ChatCompletionsStreamResponse(
                    openai_resp,
                ))
            }

            // Amazon Bedrock ConverseStream upstream
            (
                SupportedUpstreamAPIs::AmazonBedrockConverseStream(_),
                SupportedAPIsFromClient::AnthropicMessagesAPI(_),
            ) => {
                let bedrock_resp: crate::apis::amazon_bedrock::ConverseStreamEvent =
                    serde_json::from_slice(bytes)?;
                let anthropic_resp = bedrock_resp.try_into()?;
                Ok(ProviderStreamResponseType::MessagesStreamEvent(
                    anthropic_resp,
                ))
            }
            (
                SupportedUpstreamAPIs::AmazonBedrockConverseStream(_),
                SupportedAPIsFromClient::OpenAIResponsesAPI(_),
            ) => {
                // Chain: Bedrock -> ChatCompletions -> ResponsesAPI
                let bedrock_resp: crate::apis::amazon_bedrock::ConverseStreamEvent =
                    serde_json::from_slice(bytes)?;
                let chat_resp: crate::apis::openai::ChatCompletionsStreamResponse =
                    bedrock_resp.try_into()?;
                let responses_resp: ResponsesAPIStreamEvent = chat_resp.try_into()?;
                Ok(ProviderStreamResponseType::ResponseAPIStreamEvent(
                    Box::new(responses_resp),
                ))
            }
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Unsupported API combination for response transformation",
            )
            .into()),
        }
    }
}

// TryFrom implementation to convert raw bytes to SseEvent with parsed provider response
impl TryFrom<(SseEvent, &SupportedAPIsFromClient, &SupportedUpstreamAPIs)> for SseEvent {
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn try_from(
        (sse_event, client_api, upstream_api): (
            SseEvent,
            &SupportedAPIsFromClient,
            &SupportedUpstreamAPIs,
        ),
    ) -> Result<Self, Self::Error> {
        // Create a new transformed event based on the original
        let mut transformed_event = sse_event;

        // Handle [DONE] marker early - don't try to parse as JSON
        if transformed_event.is_done() {
            // For OpenAI client APIs (ChatCompletions and ResponsesAPI), keep [DONE] as-is
            // For Anthropic client API, it will be transformed via ProviderStreamResponseType
            if matches!(
                client_api,
                SupportedAPIsFromClient::OpenAIChatCompletions(_)
                    | SupportedAPIsFromClient::OpenAIResponsesAPI(_)
            ) {
                // Keep the [DONE] marker as-is for OpenAI clients
                transformed_event.sse_transformed_lines = "data: [DONE]".to_string();
                return Ok(transformed_event);
            }
        }

        // If has data, parse the data as a provider stream response (business logic layer)
        if let Some(data_str) = &transformed_event.data {
            let data_bytes = data_str.as_bytes();
            let transformed_response: ProviderStreamResponseType =
                ProviderStreamResponseType::try_from((data_bytes, client_api, upstream_api))?;

            // Convert to SSE string explicitly to avoid type ambiguity
            let sse_string: String = transformed_response.clone().into();
            transformed_event.sse_transformed_lines = sse_string;
            transformed_event.provider_stream_response = Some(transformed_response);
        }

        // Apply wire format adjustments for cross-API transformations
        // Note: When APIs match (passthrough mode), these adjustments are skipped
        // since PassthroughStreamBuffer will handle events as-is
        if needs_buffering(client_api, upstream_api) {
            match (client_api, upstream_api) {
                (
                    SupportedAPIsFromClient::OpenAIChatCompletions(_),
                    SupportedUpstreamAPIs::AnthropicMessagesAPI(_),
                ) => {
                    // OpenAI clients don't expect separate event: lines
                    // Suppress upstream Anthropic event-only lines
                    if transformed_event.is_event_only() && transformed_event.event.is_some() {
                        transformed_event.sse_transformed_lines = "\n".to_string();
                    }
                }
                _ => {
                    // Other cross-API combinations can be handled here as needed
                }
            }
        } else {
            // Passthrough mode: APIs match, no transformation needed
            // For Anthropic and ResponsesAPI SSE formats, event-only lines are redundant because
            // the Into<String> implementation for MessagesStreamEvent and ResponsesAPIStreamEvent
            // couples event and data lines together. We suppress event-only events to
            // avoid duplicate event: lines in the output.
            match (client_api, upstream_api) {
                (
                    SupportedAPIsFromClient::AnthropicMessagesAPI(_),
                    SupportedUpstreamAPIs::AnthropicMessagesAPI(_),
                )
                | (
                    SupportedAPIsFromClient::OpenAIResponsesAPI(_),
                    SupportedUpstreamAPIs::OpenAIResponsesAPI(_),
                ) => {
                    if transformed_event.is_event_only() && transformed_event.event.is_some() {
                        // Mark as should-skip by clearing sse_transformed_lines
                        // The event line is already included when the data line is transformed
                        transformed_event.sse_transformed_lines = String::new();
                    }
                }
                _ => {
                    // Other passthrough combinations (OpenAI ChatCompletions, etc.) don't have this issue
                }
            }
        }

        Ok(transformed_event)
    }
}

// TryFrom implementation to convert AWS Event Stream DecodedFrame to ProviderStreamResponseType
impl
    TryFrom<(
        &aws_smithy_eventstream::frame::DecodedFrame,
        &SupportedAPIsFromClient,
        &SupportedUpstreamAPIs,
    )> for ProviderStreamResponseType
{
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn try_from(
        (frame, client_api, upstream_api): (
            &aws_smithy_eventstream::frame::DecodedFrame,
            &SupportedAPIsFromClient,
            &SupportedUpstreamAPIs,
        ),
    ) -> Result<Self, Self::Error> {
        use aws_smithy_eventstream::frame::DecodedFrame;

        match frame {
            DecodedFrame::Complete(_) => {
                // We have a complete frame - parse it based on upstream API
                match (upstream_api, client_api) {
                    (
                        SupportedUpstreamAPIs::AmazonBedrockConverseStream(_),
                        SupportedAPIsFromClient::AnthropicMessagesAPI(_),
                    ) => {
                        // Parse the DecodedFrame into ConverseStreamEvent
                        let bedrock_event =
                            crate::apis::amazon_bedrock::ConverseStreamEvent::try_from(frame)?;
                        let anthropic_event: crate::apis::anthropic::MessagesStreamEvent =
                            bedrock_event.try_into()?;

                        Ok(ProviderStreamResponseType::MessagesStreamEvent(
                            anthropic_event,
                        ))
                    }
                    (
                        SupportedUpstreamAPIs::AmazonBedrockConverseStream(_),
                        SupportedAPIsFromClient::OpenAIChatCompletions(_),
                    ) => {
                        // Parse the DecodedFrame into ConverseStreamEvent
                        let bedrock_event =
                            crate::apis::amazon_bedrock::ConverseStreamEvent::try_from(frame)?;
                        let openai_event: crate::apis::openai::ChatCompletionsStreamResponse =
                            bedrock_event.try_into()?;
                        Ok(ProviderStreamResponseType::ChatCompletionsStreamResponse(
                            openai_event,
                        ))
                    }
                    (
                        SupportedUpstreamAPIs::AmazonBedrockConverseStream(_),
                        SupportedAPIsFromClient::OpenAIResponsesAPI(_),
                    ) => {
                        // Parse the DecodedFrame into ConverseStreamEvent
                        let bedrock_event =
                            crate::apis::amazon_bedrock::ConverseStreamEvent::try_from(frame)?;
                        let openai_chat_completions_event: crate::apis::openai::ChatCompletionsStreamResponse =
                            bedrock_event.try_into()?;
                        let openai_responses_api_event: crate::apis::openai_responses::ResponsesAPIStreamEvent =
                            openai_chat_completions_event.try_into()?;

                        Ok(ProviderStreamResponseType::ResponseAPIStreamEvent(
                            Box::new(openai_responses_api_event),
                        ))
                    }
                    _ => Err("Unsupported API combination for event-stream decoding".into()),
                }
            }
            DecodedFrame::Incomplete => {
                Err("Cannot convert incomplete frame to provider response".into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apis::streaming_shapes::amazon_bedrock_binary_frame::BedrockBinaryFrameDecoder;
    use crate::apis::streaming_shapes::sse::SseStreamIter;
    use crate::clients::endpoints::SupportedAPIsFromClient;
    use serde_json::json;

    #[test]
    fn test_sse_event_parsing() {
        // Test valid SSE data line
        let line = "data: {\"id\":\"test\",\"object\":\"chat.completion.chunk\"}\n\n";
        let event: Result<SseEvent, _> = line.parse();
        assert!(event.is_ok());
        let event = event.unwrap();
        // The data field should contain only the JSON content, not the trailing newlines
        assert_eq!(
            event.data,
            Some("{\"id\":\"test\",\"object\":\"chat.completion.chunk\"}".to_string())
        );

        // Test conversion back to line using Display trait
        // The sse_transformed_lines preserves the original format including trailing newlines
        let wire_format = event.to_string();
        assert_eq!(
            wire_format,
            "data: {\"id\":\"test\",\"object\":\"chat.completion.chunk\"}\n\n"
        );

        // Test [DONE] marker - should be valid SSE event
        let done_line = "data: [DONE]";
        let done_result: Result<SseEvent, _> = done_line.parse();
        assert!(done_result.is_ok());
        let done_event = done_result.unwrap();
        assert_eq!(done_event.data, Some("[DONE]".to_string()));
        assert!(done_event.is_done()); // Test the helper method

        // Test non-DONE event
        assert!(!event.is_done());

        // Test empty data - should return error
        let empty_line = "data: ";
        let empty_result: Result<SseEvent, _> = empty_line.parse();
        assert!(empty_result.is_err());

        // Test non-data line - should return error
        let comment_line = ": this is a comment";
        let comment_result: Result<SseEvent, _> = comment_line.parse();
        assert!(comment_result.is_err());
    }

    #[test]
    fn test_sse_event_serde() {
        // Test serialization and deserialization with serde
        let event = SseEvent {
            data: Some(r#"{"id":"test","object":"chat.completion.chunk"}"#.to_string()),
            event: None,
            raw_line: r#"data: {"id":"test","object":"chat.completion.chunk"}

        "#
            .to_string(),
            sse_transformed_lines: r#"data: {"id":"test","object":"chat.completion.chunk"}

        "#
            .to_string(),
            provider_stream_response: None,
        };

        // Test JSON serialization - raw_line should be skipped
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("test"));
        assert!(json.contains("chat.completion.chunk"));
        assert!(!json.contains("raw_line")); // Should be excluded from serialization

        // Test JSON deserialization
        let deserialized: SseEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.data, event.data);
        assert_eq!(deserialized.raw_line, ""); // Should be empty since it's skipped

        // Test round trip for data field only
        assert_eq!(event.data, deserialized.data);
    }

    #[test]
    fn test_sse_event_should_skip() {
        // Test ping message should be skipped
        let ping_event = SseEvent {
            data: Some(r#"{"type": "ping"}"#.to_string()),
            event: None,
            raw_line: r#"data: {"type": "ping"}"#.to_string(),
            sse_transformed_lines: r#"data: {"type": "ping"}"#.to_string(),
            provider_stream_response: None,
        };
        assert!(ping_event.should_skip());
        assert!(!ping_event.is_done());

        // Test normal event should not be skipped
        let normal_event = SseEvent {
            data: Some(r#"{"id": "test", "object": "chat.completion.chunk"}"#.to_string()),
            event: Some("content_block_delta".to_string()),
            raw_line: r#"data: {"id": "test", "object": "chat.completion.chunk"}"#.to_string(),
            sse_transformed_lines: r#"data: {"id": "test", "object": "chat.completion.chunk"}"#
                .to_string(),
            provider_stream_response: None,
        };
        assert!(!normal_event.should_skip());
        assert!(!normal_event.is_done());

        // Test [DONE] event should not be skipped (but is handled separately)
        let done_event = SseEvent {
            data: Some("[DONE]".to_string()),
            event: None,
            raw_line: "data: [DONE]".to_string(),
            sse_transformed_lines: "data: [DONE]".to_string(),
            provider_stream_response: None,
        };
        assert!(!done_event.should_skip());
        assert!(done_event.is_done());
    }

    #[test]
    fn test_sse_stream_iter_filters_ping_messages() {
        // Create test data with ping messages mixed in
        let test_lines = vec![
            "data: {\"id\": \"msg1\", \"object\": \"chat.completion.chunk\"}".to_string(),
            "data: {\"type\": \"ping\"}".to_string(), // This should be filtered out
            "data: {\"id\": \"msg2\", \"object\": \"chat.completion.chunk\"}".to_string(),
            "data: {\"type\": \"ping\"}".to_string(), // This should be filtered out
            "data: [DONE]".to_string(),               // This should end the stream
        ];

        let mut iter = SseStreamIter::new(test_lines.into_iter());

        // First event should be msg1 (ping filtered out)
        let event1 = iter.next().unwrap();
        assert!(event1.data.as_ref().unwrap().contains("msg1"));
        assert!(!event1.should_skip());

        // Second event should be msg2 (ping filtered out)
        let event2 = iter.next().unwrap();
        assert!(event2.data.as_ref().unwrap().contains("msg2"));
        assert!(!event2.should_skip());

        // Third event should be [DONE]
        let done_event = iter.next().unwrap();
        assert!(done_event.is_done());

        // Iterator should end after [DONE]
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_sse_stream_iter_handles_anthropic_events() {
        // Create test data with Anthropic-style event/data pairs
        let test_lines = vec![
            "event: message_start".to_string(),
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_123\"}}".to_string(),
            "event: content_block_delta".to_string(),
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"Hello\"}}".to_string(),
            "data: [DONE]".to_string(),
        ];

        let mut iter = SseStreamIter::new(test_lines.into_iter());

        // First event should be the event: line
        let event1 = iter.next().unwrap();
        assert!(event1.is_event_only());
        assert_eq!(event1.event, Some("message_start".to_string()));
        assert_eq!(event1.data, None);

        // Second event should be the data: line
        let event2 = iter.next().unwrap();
        assert!(!event2.is_event_only());
        assert_eq!(event2.event, None);
        assert!(event2.data.as_ref().unwrap().contains("message_start"));

        // Third event should be another event: line
        let event3 = iter.next().unwrap();
        assert!(event3.is_event_only());
        assert_eq!(event3.event, Some("content_block_delta".to_string()));

        // Fourth event should be the content delta data
        let event4 = iter.next().unwrap();
        assert!(!event4.is_event_only());
        assert!(event4.data.as_ref().unwrap().contains("Hello"));

        // Fifth event should be [DONE]
        let done_event = iter.next().unwrap();
        assert!(done_event.is_done());

        // Iterator should end after [DONE]
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_provider_stream_response_event_type() {
        use crate::apis::anthropic::{MessagesContentDelta, MessagesStreamEvent};
        use crate::apis::openai::ChatCompletionsStreamResponse;

        // Test Anthropic event type
        let anthropic_event = MessagesStreamEvent::ContentBlockDelta {
            index: 0,
            delta: MessagesContentDelta::TextDelta {
                text: "Hello".to_string(),
            },
        };
        let provider_type = ProviderStreamResponseType::MessagesStreamEvent(anthropic_event);
        assert_eq!(provider_type.event_type(), Some("content_block_delta"));

        // Test OpenAI event type (should be None)
        let openai_event = ChatCompletionsStreamResponse {
            id: "test".to_string(),
            object: Some("chat.completion.chunk".to_string()),
            created: 123456789,
            model: "gpt-4".to_string(),
            choices: vec![],
            usage: None,
            system_fingerprint: None,
            service_tier: None,
        };
        let provider_type = ProviderStreamResponseType::ChatCompletionsStreamResponse(openai_event);
        assert_eq!(provider_type.event_type(), None);
    }

    #[test]
    fn test_done_marker_handled_in_stream_response_transformation() {
        use crate::apis::anthropic::AnthropicApi;

        // Test that [DONE] marker is properly converted to MessageStop in the transformation layer
        let done_bytes = b"[DONE]";
        let client_api = SupportedAPIsFromClient::AnthropicMessagesAPI(AnthropicApi::Messages);
        let upstream_api = SupportedUpstreamAPIs::OpenAIChatCompletions(
            crate::apis::openai::OpenAIApi::ChatCompletions,
        );

        let result = ProviderStreamResponseType::try_from((
            done_bytes.as_slice(),
            &client_api,
            &upstream_api,
        ));
        assert!(result.is_ok());

        if let Ok(ProviderStreamResponseType::MessagesStreamEvent(event)) = result {
            // Verify it's a MessageStop event
            assert_eq!(event.event_type(), Some("message_stop"));
            assert!(matches!(
                event,
                crate::apis::anthropic::MessagesStreamEvent::MessageStop
            ));
        } else {
            panic!("Expected MessagesStreamEvent::MessageStop");
        }
    }

    #[test]
    fn test_bedrock_event_stream_decoder_basic() {
        use bytes::BytesMut;

        // Create a simple test with minimal data
        let mut buffer = BytesMut::new();

        // Add some arbitrary bytes (not a real event-stream frame, just for testing the decoder)
        buffer.extend_from_slice(b"test data");

        let mut decoder = BedrockBinaryFrameDecoder::new(&mut buffer);

        // The decoder should return Incomplete for incomplete/invalid data
        // This signals the caller to wait for more data
        let result = decoder.decode_frame();
        assert!(result.is_some());
        assert!(matches!(
            result.unwrap(),
            aws_smithy_eventstream::frame::DecodedFrame::Incomplete
        ));

        // Verify we can still access the buffer
        assert!(decoder.has_remaining());
    }

    #[test]
    fn test_bedrock_event_stream_decoder_with_real_frames() {
        use bytes::BytesMut;
        use std::fs;
        use std::path::PathBuf;

        // Read the actual response.hex file from tests/e2e directory
        let test_file =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/e2e/response.hex");

        // Only run this test if the file exists
        if !test_file.exists() {
            println!("Skipping test - response.hex not found");
            return;
        }

        let response_data = fs::read(&test_file).unwrap();
        let mut buffer = BytesMut::from(&response_data[..]);

        let mut decoder = BedrockBinaryFrameDecoder::new(&mut buffer);
        let mut frame_count = 0;

        // Decode all frames
        loop {
            match decoder.decode_frame() {
                Some(aws_smithy_eventstream::frame::DecodedFrame::Complete(message)) => {
                    frame_count += 1;

                    // Verify we can access headers
                    let event_type = message
                        .headers()
                        .iter()
                        .find(|h| h.name().as_str() == ":event-type")
                        .and_then(|h| h.value().as_string().ok());

                    assert!(event_type.is_some(), "Frame should have :event-type header");
                }
                Some(aws_smithy_eventstream::frame::DecodedFrame::Incomplete) => {
                    // End of buffer, no more complete frames available
                    break;
                }
                None => {
                    // Decode error
                    panic!("Decode error encountered");
                }
            }
        }

        // We should have decoded multiple frames
        assert!(frame_count > 0, "Should have decoded at least one frame");
    }

    #[test]
    fn test_bedrock_event_stream_decoder_chunked_data() {
        use bytes::BytesMut;
        use std::fs;
        use std::path::PathBuf;

        // Read the actual response.hex file
        let test_file =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/e2e/response.hex");

        if !test_file.exists() {
            println!("Skipping test - response.hex not found");
            return;
        }

        let response_data = fs::read(&test_file).unwrap();

        // Simulate chunked network arrivals with realistic chunk sizes
        // Using varying chunk sizes to test partial frame handling
        let mut buffer = BytesMut::new();
        let chunk_size_pattern = [500, 1000, 750, 1200, 800, 1500];
        let mut offset = 0;
        let mut total_frames = 0;
        let mut chunk_num = 0;

        // CRITICAL: Create ONE decoder and reuse it across chunks
        // The MessageFrameDecoder maintains state about partial frames
        let mut decoder = BedrockBinaryFrameDecoder::new(&mut buffer);

        // Process all data in chunks
        while offset < response_data.len() {
            let chunk_size = chunk_size_pattern[chunk_num % chunk_size_pattern.len()];
            chunk_num += 1;

            let end = (offset + chunk_size).min(response_data.len());
            let chunk = &response_data[offset..end];

            // Add new data to the buffer (accessing via buffer_mut())
            decoder.buffer_mut().extend_from_slice(chunk);
            offset = end;

            // Process all available complete frames from this chunk
            loop {
                match decoder.decode_frame() {
                    Some(aws_smithy_eventstream::frame::DecodedFrame::Complete(_)) => {
                        total_frames += 1;
                    }
                    Some(aws_smithy_eventstream::frame::DecodedFrame::Incomplete) => {
                        // Need more data - wait for next chunk
                        break;
                    }
                    None => {
                        // Decode error
                        panic!("Decode error in chunked test");
                    }
                }
            }
        }

        assert!(
            total_frames > 0,
            "Should have decoded frames from chunked data"
        );
    }

    #[test]
    fn test_bedrock_decoded_frame_to_provider_response() {
        test_bedrock_conversion(false);
    }

    #[test]
    #[ignore] // Run with: cargo test -- --ignored --nocapture
    fn test_bedrock_decoded_frame_to_provider_response_verbose() {
        test_bedrock_conversion(true);
    }

    #[test]
    fn test_bedrock_decoded_frame_with_tool_use() {
        test_bedrock_conversion_with_tools(false);
    }

    #[test]
    #[ignore] // Run with: cargo test -- --ignored --nocapture
    fn test_bedrock_decoded_frame_with_tool_use_verbose() {
        test_bedrock_conversion_with_tools(true);
    }

    fn test_bedrock_conversion(verbose: bool) {
        use bytes::BytesMut;
        use std::fs;
        use std::path::PathBuf;

        // Read the actual response.hex file from tests/e2e directory
        let test_file =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/e2e/response.hex");

        // Only run this test if the file exists
        if !test_file.exists() {
            println!("Skipping test - response.hex not found");
            return;
        }

        let response_data = fs::read(&test_file).unwrap();
        let mut buffer = BytesMut::from(&response_data[..]);

        let mut decoder = BedrockBinaryFrameDecoder::new(&mut buffer);

        let client_api = SupportedAPIsFromClient::AnthropicMessagesAPI(
            crate::apis::anthropic::AnthropicApi::Messages,
        );
        let upstream_api = SupportedUpstreamAPIs::AmazonBedrockConverseStream(
            crate::apis::amazon_bedrock::AmazonBedrockApi::ConverseStream,
        );

        let mut conversion_count = 0;
        let mut message_start_seen = false;

        // Decode and convert frames
        loop {
            match decoder.decode_frame() {
                Some(frame @ aws_smithy_eventstream::frame::DecodedFrame::Complete(_)) => {
                    // Convert DecodedFrame to ProviderStreamResponseType
                    let result =
                        ProviderStreamResponseType::try_from((&frame, &client_api, &upstream_api));

                    match result {
                        Ok(provider_response) => {
                            conversion_count += 1;

                            // Verify we got a MessagesStreamEvent
                            assert!(matches!(
                                provider_response,
                                ProviderStreamResponseType::MessagesStreamEvent(_)
                            ));

                            if verbose {
                                // Print the SSE string output
                                let sse_string: String = provider_response.clone().into();
                                println!("{}", sse_string);
                            }

                            // Check for MessageStart event
                            if let ProviderStreamResponseType::MessagesStreamEvent(ref event) =
                                provider_response
                            {
                                if matches!(
                                    event,
                                    crate::apis::anthropic::MessagesStreamEvent::MessageStart { .. }
                                ) {
                                    message_start_seen = true;
                                }
                            }
                        }
                        Err(e) => {
                            println!("Conversion error (frame {}): {}", conversion_count, e);
                        }
                    }
                }
                Some(aws_smithy_eventstream::frame::DecodedFrame::Incomplete) => {
                    // End of buffer
                    break;
                }
                None => {
                    panic!("Decode error");
                }
            }
        }

        assert!(
            conversion_count > 0,
            "Should have converted at least one frame"
        );
        assert!(message_start_seen, "Should have seen MessageStart event");
    }

    fn test_bedrock_conversion_with_tools(verbose: bool) {
        use bytes::BytesMut;
        use std::fs;
        use std::path::PathBuf;

        // Read the actual response_with_tools.hex file from tests/e2e directory
        let test_file = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/e2e/response_with_tools.hex");

        // Only run this test if the file exists
        if !test_file.exists() {
            println!("Skipping test - response_with_tools.hex not found");
            return;
        }

        let response_data = fs::read(&test_file).unwrap();
        let mut buffer = BytesMut::from(&response_data[..]);

        let mut decoder = BedrockBinaryFrameDecoder::new(&mut buffer);

        let client_api = SupportedAPIsFromClient::AnthropicMessagesAPI(
            crate::apis::anthropic::AnthropicApi::Messages,
        );
        let upstream_api = SupportedUpstreamAPIs::AmazonBedrockConverseStream(
            crate::apis::amazon_bedrock::AmazonBedrockApi::ConverseStream,
        );

        let mut conversion_count = 0;
        let mut message_start_seen = false;
        let mut content_block_start_seen = false;
        let mut content_block_delta_tool_use_seen = false;

        // Decode and convert frames
        loop {
            match decoder.decode_frame() {
                Some(frame @ aws_smithy_eventstream::frame::DecodedFrame::Complete(_)) => {
                    // Convert DecodedFrame to ProviderStreamResponseType
                    let result =
                        ProviderStreamResponseType::try_from((&frame, &client_api, &upstream_api));

                    match result {
                        Ok(provider_response) => {
                            conversion_count += 1;

                            // Verify we got a MessagesStreamEvent
                            assert!(matches!(
                                provider_response,
                                ProviderStreamResponseType::MessagesStreamEvent(_)
                            ));

                            if verbose {
                                // Print the SSE string output
                                let sse_string: String = provider_response.clone().into();
                                println!("{}", sse_string);
                            }

                            // Check for specific events related to tool use
                            if let ProviderStreamResponseType::MessagesStreamEvent(ref event) =
                                provider_response
                            {
                                match event {
                                    crate::apis::anthropic::MessagesStreamEvent::MessageStart { .. } => {
                                        message_start_seen = true;
                                    }
                                    crate::apis::anthropic::MessagesStreamEvent::ContentBlockStart { .. } => {
                                        content_block_start_seen = true;
                                    }
                                    crate::apis::anthropic::MessagesStreamEvent::ContentBlockDelta { delta, .. } => {
                                        if matches!(delta, crate::apis::anthropic::MessagesContentDelta::InputJsonDelta { .. }) {
                                            content_block_delta_tool_use_seen = true;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        Err(e) => {
                            println!("Conversion error (frame {}): {}", conversion_count, e);
                        }
                    }
                }
                Some(aws_smithy_eventstream::frame::DecodedFrame::Incomplete) => {
                    // End of buffer
                    break;
                }
                None => {
                    panic!("Decode error");
                }
            }
        }

        assert!(
            conversion_count > 0,
            "Should have converted at least one frame"
        );
        assert!(message_start_seen, "Should have seen MessageStart event");
        assert!(
            content_block_start_seen,
            "Should have seen ContentBlockStart event for tool use"
        );
        assert!(
            content_block_delta_tool_use_seen,
            "Should have seen ContentBlockDelta with ToolUseDelta"
        );
    }

    #[test]
    fn test_sse_event_transformation_openai_to_anthropic_message_delta() {
        use crate::apis::anthropic::AnthropicApi;
        use crate::apis::openai::OpenAIApi;

        // Create an OpenAI stream response with finish_reason (which becomes message_delta in Anthropic)
        let openai_stream_chunk = json!({
            "id": "chatcmpl-123",
            "object": "chat.completion.chunk",
            "created": 1234567890,
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "delta": {},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 25,
                "total_tokens": 35
            }
        });

        // Create SSE event with this data
        let sse_event = SseEvent {
            data: Some(openai_stream_chunk.to_string()),
            event: None,
            raw_line: format!("data: {}", openai_stream_chunk),
            sse_transformed_lines: format!("data: {}", openai_stream_chunk),
            provider_stream_response: None,
        };

        let client_api = SupportedAPIsFromClient::AnthropicMessagesAPI(AnthropicApi::Messages);
        let upstream_api = SupportedUpstreamAPIs::OpenAIChatCompletions(OpenAIApi::ChatCompletions);

        // Transform the event
        let result = SseEvent::try_from((sse_event, &client_api, &upstream_api));
        assert!(result.is_ok());

        let transformed = result.unwrap();

        // NOTE: This test now verifies single-event transformation only.
        // Multi-event injection (content_block_stop + message_delta) is now handled
        // by AnthropicMessagesStreamBuffer, not by TryFrom transformation.
        let buffer = transformed.sse_transformed_lines;

        // Verify the event was transformed to Anthropic format
        // This should contain message_delta with stop_reason and usage
        assert!(
            buffer.contains("event: message_delta")
                || buffer.contains("\"type\":\"message_delta\""),
            "Should contain message_delta in transformed event"
        );

        // Verify usage information is present
        assert!(
            buffer.contains("\"usage\""),
            "Should contain usage information"
        );
    }

    #[test]
    fn test_sse_event_transformation_openai_to_anthropic_content_delta() {
        use crate::apis::anthropic::AnthropicApi;
        use crate::apis::openai::OpenAIApi;

        // Create an OpenAI stream response with content (which becomes content_block_delta in Anthropic)
        let openai_stream_chunk = json!({
            "id": "chatcmpl-123",
            "object": "chat.completion.chunk",
            "created": 1234567890,
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "delta": {"content": "Hello"},
                "finish_reason": null
            }]
        });

        // Create SSE event with this data
        let sse_event = SseEvent {
            data: Some(openai_stream_chunk.to_string()),
            event: None,
            raw_line: format!("data: {}", openai_stream_chunk),
            sse_transformed_lines: format!("data: {}", openai_stream_chunk),
            provider_stream_response: None,
        };

        let client_api = SupportedAPIsFromClient::AnthropicMessagesAPI(AnthropicApi::Messages);
        let upstream_api = SupportedUpstreamAPIs::OpenAIChatCompletions(OpenAIApi::ChatCompletions);

        // Transform the event
        let result = SseEvent::try_from((sse_event, &client_api, &upstream_api));
        assert!(result.is_ok());

        let transformed = result.unwrap();

        // Verify the transformation is a content_block_delta (no extra events injected)
        let buffer = transformed.sse_transformed_lines;
        assert!(
            buffer.contains("event: content_block_delta"),
            "Should contain content_block_delta event"
        );
        assert!(
            !buffer.contains("content_block_start"),
            "Should not inject content_block_start for content delta"
        );
        assert!(
            !buffer.contains("content_block_stop"),
            "Should not inject content_block_stop for content delta"
        );

        // Verify the content is preserved
        assert!(buffer.contains("Hello"), "Should preserve the content text");
    }

    #[test]
    fn test_sse_event_transformation_anthropic_to_openai_suppresses_event_lines() {
        use crate::apis::anthropic::AnthropicApi;
        use crate::apis::openai::OpenAIApi;

        // Create an Anthropic event-only SSE line (no data)
        let sse_event = SseEvent {
            data: None,
            event: Some("message_start".to_string()),
            raw_line: "event: message_start".to_string(),
            sse_transformed_lines: "event: message_start".to_string(),
            provider_stream_response: None,
        };

        let client_api = SupportedAPIsFromClient::OpenAIChatCompletions(OpenAIApi::ChatCompletions);
        let upstream_api = SupportedUpstreamAPIs::AnthropicMessagesAPI(AnthropicApi::Messages);

        // Transform the event
        let result = SseEvent::try_from((sse_event, &client_api, &upstream_api));
        assert!(result.is_ok());

        let transformed = result.unwrap();

        // Verify the event line is suppressed (replaced with just newline)
        assert_eq!(
            transformed.sse_transformed_lines, "\n",
            "Event-only lines should be suppressed to newline for OpenAI"
        );
        assert!(
            transformed.is_event_only(),
            "Should still be marked as event-only"
        );
    }

    #[test]
    fn test_sse_event_transformation_anthropic_to_openai_preserves_data() {
        use crate::apis::anthropic::AnthropicApi;
        use crate::apis::openai::OpenAIApi;

        // Create an Anthropic message_start event with data
        let anthropic_event = json!({
            "type": "message_start",
            "message": {
                "id": "msg_123",
                "type": "message",
                "role": "assistant",
                "content": [],
                "model": "claude-3-sonnet",
                "stop_reason": null,
                "usage": {"input_tokens": 10, "output_tokens": 0}
            }
        });

        let sse_event = SseEvent {
            data: Some(anthropic_event.to_string()),
            event: None,
            raw_line: format!("data: {}", anthropic_event),
            sse_transformed_lines: format!("data: {}", anthropic_event),
            provider_stream_response: None,
        };

        let client_api = SupportedAPIsFromClient::OpenAIChatCompletions(OpenAIApi::ChatCompletions);
        let upstream_api = SupportedUpstreamAPIs::AnthropicMessagesAPI(AnthropicApi::Messages);

        // Transform the event
        let result = SseEvent::try_from((sse_event, &client_api, &upstream_api));
        assert!(result.is_ok());

        let transformed = result.unwrap();

        // Verify data is transformed to OpenAI format
        let buffer = transformed.sse_transformed_lines;
        assert!(buffer.starts_with("data: "), "Should have data: prefix");
        assert!(
            !buffer.contains("event:"),
            "Should not have event: lines for OpenAI"
        );

        // Verify provider response was parsed
        assert!(transformed.provider_stream_response.is_some());
    }

    #[test]
    fn test_sse_event_transformation_no_change_for_matching_apis() {
        use crate::apis::openai::OpenAIApi;

        // Create an OpenAI stream response
        let openai_stream_chunk = json!({
            "id": "chatcmpl-123",
            "object": "chat.completion.chunk",
            "created": 1234567890,
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "delta": {"content": "Hello"},
                "finish_reason": null
            }]
        });

        let original_data = openai_stream_chunk.to_string();
        let sse_event = SseEvent {
            data: Some(original_data.clone()),
            event: None,
            raw_line: format!("data: {}", original_data),
            sse_transformed_lines: format!("data: {}\n\n", original_data),
            provider_stream_response: None,
        };

        let client_api = SupportedAPIsFromClient::OpenAIChatCompletions(OpenAIApi::ChatCompletions);
        let upstream_api = SupportedUpstreamAPIs::OpenAIChatCompletions(OpenAIApi::ChatCompletions);

        // Transform the event
        let result = SseEvent::try_from((sse_event, &client_api, &upstream_api));
        assert!(result.is_ok());

        let transformed = result.unwrap();

        // Verify minimal transformation - just SSE formatting, no API conversion
        let buffer = transformed.sse_transformed_lines;
        assert!(buffer.starts_with("data: "), "Should preserve data: prefix");
        assert!(!buffer.contains("event:"), "Should not add event: lines");

        // Verify provider response was parsed
        assert!(transformed.provider_stream_response.is_some());
    }

    #[test]
    fn test_sse_event_transformation_preserves_provider_response() {
        use crate::apis::anthropic::AnthropicApi;
        use crate::apis::openai::OpenAIApi;

        // Create an OpenAI stream response
        let openai_stream_chunk = json!({
            "id": "chatcmpl-123",
            "object": "chat.completion.chunk",
            "created": 1234567890,
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "delta": {"content": "Test"},
                "finish_reason": null
            }]
        });

        let sse_event = SseEvent {
            data: Some(openai_stream_chunk.to_string()),
            event: None,
            raw_line: format!("data: {}", openai_stream_chunk),
            sse_transformed_lines: format!("data: {}", openai_stream_chunk),
            provider_stream_response: None,
        };

        let client_api = SupportedAPIsFromClient::AnthropicMessagesAPI(AnthropicApi::Messages);
        let upstream_api = SupportedUpstreamAPIs::OpenAIChatCompletions(OpenAIApi::ChatCompletions);

        // Transform the event
        let result = SseEvent::try_from((sse_event, &client_api, &upstream_api));
        assert!(result.is_ok());

        let transformed = result.unwrap();

        // Verify provider_stream_response is populated
        assert!(
            transformed.provider_stream_response.is_some(),
            "Should parse and store provider response"
        );

        // Verify we can access the provider response
        let provider_response = transformed.provider_response();
        assert!(
            provider_response.is_ok(),
            "Should be able to access provider response"
        );

        // Verify the content delta is accessible
        let content = provider_response.unwrap().content_delta();
        assert_eq!(content, Some("Test"), "Should preserve content delta");
    }
}
