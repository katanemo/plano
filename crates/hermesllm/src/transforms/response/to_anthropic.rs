use crate::apis::amazon_bedrock::{ConverseOutput, ConverseResponse, StopReason};
use crate::apis::anthropic::{
    MessagesContentBlock, MessagesResponse, MessagesRole, MessagesStopReason, MessagesUsage,
};
use crate::apis::openai::ChatCompletionsResponse;
use crate::clients::TransformError;
use crate::transforms::lib::*;

// ============================================================================
// STANDARD RUST TRAIT IMPLEMENTATIONS - Using Into/TryFrom for convenience
// ============================================================================

impl TryFrom<ChatCompletionsResponse> for MessagesResponse {
    type Error = TransformError;

    fn try_from(resp: ChatCompletionsResponse) -> Result<Self, Self::Error> {
        let choice = resp
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| TransformError::MissingField("choices".to_string()))?;

        let content = convert_openai_message_to_anthropic_content(&choice.message.to_message())?;
        let stop_reason = choice
            .finish_reason
            .map(|fr| fr.into())
            .unwrap_or(MessagesStopReason::EndTurn);

        let usage = MessagesUsage {
            input_tokens: resp.usage.prompt_tokens,
            output_tokens: resp.usage.completion_tokens,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        };

        Ok(MessagesResponse {
            id: resp.id,
            obj_type: "message".to_string(),
            role: MessagesRole::Assistant,
            content,
            model: resp.model,
            stop_reason,
            stop_sequence: None,
            usage,
            container: None,
        })
    }
}

impl TryFrom<ConverseResponse> for MessagesResponse {
    type Error = TransformError;

    fn try_from(resp: ConverseResponse) -> Result<Self, Self::Error> {
        // Extract the message from the ConverseOutput
        let message = match resp.output {
            ConverseOutput::Message { message } => message,
        };

        // Convert Bedrock message content to Anthropic content blocks
        let content = convert_bedrock_message_to_anthropic_content(&message)?;

        // Convert Bedrock ConversationRole to Anthropic MessagesRole
        let role = match message.role {
            crate::apis::amazon_bedrock::ConversationRole::User => MessagesRole::User,
            crate::apis::amazon_bedrock::ConversationRole::Assistant => MessagesRole::Assistant,
        };

        // Convert Bedrock stop reason to Anthropic stop reason
        let stop_reason = match resp.stop_reason {
            StopReason::EndTurn => MessagesStopReason::EndTurn,
            StopReason::ToolUse => MessagesStopReason::ToolUse,
            StopReason::MaxTokens => MessagesStopReason::MaxTokens,
            StopReason::StopSequence => MessagesStopReason::EndTurn,
            StopReason::GuardrailIntervened => MessagesStopReason::Refusal,
            StopReason::ContentFiltered => MessagesStopReason::Refusal,
        };

        // Convert token usage
        let usage = MessagesUsage {
            input_tokens: resp.usage.input_tokens,
            output_tokens: resp.usage.output_tokens,
            cache_creation_input_tokens: resp.usage.cache_write_input_tokens,
            cache_read_input_tokens: resp.usage.cache_read_input_tokens,
        };

        // Generate a response ID (Bedrock doesn't provide one)
        let id = format!(
            "bedrock-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );

        // Extract model ID from trace information if available, otherwise use fallback
        let model = resp
            .trace
            .as_ref()
            .and_then(|trace| trace.prompt_router.as_ref())
            .map(|router| router.invoked_model_id.clone())
            .unwrap_or_else(|| "bedrock-model".to_string());

        Ok(MessagesResponse {
            id,
            obj_type: "message".to_string(),
            role,
            content,
            model,
            stop_reason,
            stop_sequence: None, // TODO: Could extract from additional_model_response_fields if needed
            usage,
            container: None,
        })
    }
}

/// Convert Bedrock Message to Anthropic content blocks
///
/// This function handles the conversion between Amazon Bedrock Converse API format
/// and Anthropic's Messages API format. Key differences handled:
///
/// 1. **Image/Document Sources**: Bedrock supports base64 and S3 locations, while
///    Anthropic supports base64, URLs, and file IDs. Currently only base64 is supported.
/// 2. **Tool Result Status**: Bedrock uses enum status (Success/Error), Anthropic uses
///    boolean is_error field.
/// 3. **Document Names**: Bedrock includes optional document names, Anthropic doesn't.
/// 4. **JSON Content**: Bedrock has native JSON content blocks, converted to text for Anthropic.
///
/// Note on S3/URL handling: Converting S3 locations or URLs would require async operations
/// to download and convert to base64, which is not implemented in this synchronous function.
fn convert_bedrock_message_to_anthropic_content(
    message: &crate::apis::amazon_bedrock::Message,
) -> Result<Vec<MessagesContentBlock>, TransformError> {
    use crate::apis::amazon_bedrock::ContentBlock;

    let mut content_blocks = Vec::new();

    for content_block in &message.content {
        match content_block {
            ContentBlock::Text { text } => {
                content_blocks.push(MessagesContentBlock::Text {
                    text: text.clone(),
                    cache_control: None,
                });
            }
            ContentBlock::ToolUse { tool_use } => {
                content_blocks.push(MessagesContentBlock::ToolUse {
                    id: tool_use.tool_use_id.clone(),
                    name: tool_use.name.clone(),
                    input: tool_use.input.clone(),
                    cache_control: None,
                });
            }
            ContentBlock::ToolResult { tool_result } => {
                // Convert tool result content blocks
                let mut tool_result_blocks = Vec::new();
                for result_content in &tool_result.content {
                    match result_content {
                        crate::apis::amazon_bedrock::ToolResultContentBlock::Text { text } => {
                            tool_result_blocks.push(MessagesContentBlock::Text {
                                text: text.clone(),
                                cache_control: None,
                            });
                        }
                        crate::apis::amazon_bedrock::ToolResultContentBlock::Image { source } => {
                            // Convert Bedrock ImageSource to Anthropic format
                            match source {
                                crate::apis::amazon_bedrock::ImageSource::Base64 {
                                    media_type,
                                    data,
                                } => {
                                    tool_result_blocks.push(MessagesContentBlock::Image {
                                        source:
                                            crate::apis::anthropic::MessagesImageSource::Base64 {
                                                media_type: media_type.clone(),
                                                data: data.clone(),
                                            },
                                    });
                                } // Note: S3Location is not yet implemented in the current Bedrock API definition
                                  // but would need async handling when added
                            }
                        }
                        crate::apis::amazon_bedrock::ToolResultContentBlock::Json { json } => {
                            // Convert JSON content to text representation
                            tool_result_blocks.push(MessagesContentBlock::Text {
                                text: serde_json::to_string(&json).unwrap_or_default(),
                                cache_control: None,
                            });
                        }
                    }
                }

                use crate::apis::anthropic::ToolResultContent;
                content_blocks.push(MessagesContentBlock::ToolResult {
                    tool_use_id: tool_result.tool_use_id.clone(),
                    is_error: tool_result
                        .status
                        .as_ref()
                        .map(|s| matches!(s, crate::apis::amazon_bedrock::ToolResultStatus::Error)),
                    content: ToolResultContent::Blocks(tool_result_blocks),
                    cache_control: None,
                });
            }
            ContentBlock::Image { image } => {
                // Convert Bedrock ImageSource to Anthropic format
                match &image.source {
                    crate::apis::amazon_bedrock::ImageSource::Base64 { media_type, data } => {
                        content_blocks.push(MessagesContentBlock::Image {
                            source: crate::apis::anthropic::MessagesImageSource::Base64 {
                                media_type: media_type.clone(),
                                data: data.clone(),
                            },
                        });
                    } // Note: S3Location would require async handling if implemented
                }
            }
            ContentBlock::Document { document } => {
                // Convert Bedrock DocumentSource to Anthropic format
                // Note: Bedrock's 'name' field is lost in conversion as Anthropic doesn't support it
                match &document.source {
                    crate::apis::amazon_bedrock::DocumentSource::Base64 { media_type, data } => {
                        content_blocks.push(MessagesContentBlock::Document {
                            source: crate::apis::anthropic::MessagesDocumentSource::Base64 {
                                media_type: media_type.clone(),
                                data: data.clone(),
                            },
                        });
                    } // Note: S3Location would require async handling if implemented
                }
            }
            ContentBlock::GuardContent { guard_content } => {
                // Convert guard content to text block
                if let Some(guard_text) = &guard_content.text {
                    content_blocks.push(MessagesContentBlock::Text {
                        text: guard_text.text.clone(),
                        cache_control: None,
                    });
                }
            }
        }
    }

    Ok(content_blocks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apis::amazon_bedrock::{
        BedrockTokenUsage, ContentBlock, ConversationRole, ConverseOutput, ConverseResponse,
        ConverseTrace, Message as BedrockMessage, PromptRouterTrace, StopReason,
        ToolResultContentBlock, ToolResultStatus,
    };
    use crate::apis::anthropic::{
        MessagesContentBlock, MessagesResponse, MessagesRole, MessagesStopReason, ToolResultContent,
    };
    use serde_json::json;

    #[test]
    fn test_bedrock_to_anthropic_basic_response() {
        let bedrock_response = ConverseResponse {
            output: ConverseOutput::Message {
                message: BedrockMessage {
                    role: ConversationRole::Assistant,
                    content: vec![ContentBlock::Text {
                        text: "Hello! How can I help you today?".to_string(),
                    }],
                },
            },
            stop_reason: StopReason::EndTurn,
            usage: BedrockTokenUsage {
                input_tokens: 10,
                output_tokens: 25,
                total_tokens: 35,
                cache_write_input_tokens: None,
                cache_read_input_tokens: None,
                ..Default::default()
            },
            metrics: None,
            trace: None,
            additional_model_response_fields: None,
            performance_config: None,
        };

        let anthropic_response: MessagesResponse = bedrock_response.try_into().unwrap();

        assert_eq!(anthropic_response.obj_type, "message");
        assert_eq!(anthropic_response.role, MessagesRole::Assistant);
        assert_eq!(anthropic_response.model, "bedrock-model");
        assert_eq!(anthropic_response.stop_reason, MessagesStopReason::EndTurn);
        assert!(anthropic_response.id.starts_with("bedrock-"));

        // Check content
        assert_eq!(anthropic_response.content.len(), 1);
        if let MessagesContentBlock::Text { text, .. } = &anthropic_response.content[0] {
            assert_eq!(text, "Hello! How can I help you today?");
        } else {
            panic!("Expected text content block");
        }

        // Check usage
        assert_eq!(anthropic_response.usage.input_tokens, 10);
        assert_eq!(anthropic_response.usage.output_tokens, 25);
        assert_eq!(anthropic_response.usage.cache_creation_input_tokens, None);
        assert_eq!(anthropic_response.usage.cache_read_input_tokens, None);
    }

    #[test]
    fn test_bedrock_to_anthropic_with_tool_use() {
        let bedrock_response = ConverseResponse {
            output: ConverseOutput::Message {
                message: BedrockMessage {
                    role: ConversationRole::Assistant,
                    content: vec![
                        ContentBlock::Text {
                            text: "I'll help you check the weather.".to_string(),
                        },
                        ContentBlock::ToolUse {
                            tool_use: crate::apis::amazon_bedrock::ToolUseBlock {
                                tool_use_id: "tool_12345".to_string(),
                                name: "get_weather".to_string(),
                                input: json!({
                                    "location": "San Francisco"
                                }),
                            },
                        },
                    ],
                },
            },
            stop_reason: StopReason::ToolUse,
            usage: BedrockTokenUsage {
                input_tokens: 15,
                output_tokens: 30,
                total_tokens: 45,
                cache_write_input_tokens: None,
                cache_read_input_tokens: None,
                ..Default::default()
            },
            metrics: None,
            trace: None,
            additional_model_response_fields: None,
            performance_config: None,
        };

        let anthropic_response: MessagesResponse = bedrock_response.try_into().unwrap();

        assert_eq!(anthropic_response.stop_reason, MessagesStopReason::ToolUse);
        assert_eq!(anthropic_response.content.len(), 2);

        // Check text content
        if let MessagesContentBlock::Text { text, .. } = &anthropic_response.content[0] {
            assert_eq!(text, "I'll help you check the weather.");
        } else {
            panic!("Expected text content block");
        }

        // Check tool use content
        if let MessagesContentBlock::ToolUse {
            id, name, input, ..
        } = &anthropic_response.content[1]
        {
            assert_eq!(id, "tool_12345");
            assert_eq!(name, "get_weather");
            assert_eq!(input["location"], "San Francisco");
        } else {
            panic!("Expected tool use content block");
        }
    }

    #[test]
    fn test_bedrock_to_anthropic_stop_reason_conversions() {
        let test_cases = vec![
            (StopReason::EndTurn, MessagesStopReason::EndTurn),
            (StopReason::ToolUse, MessagesStopReason::ToolUse),
            (StopReason::MaxTokens, MessagesStopReason::MaxTokens),
            (StopReason::StopSequence, MessagesStopReason::EndTurn),
            (StopReason::GuardrailIntervened, MessagesStopReason::Refusal),
            (StopReason::ContentFiltered, MessagesStopReason::Refusal),
        ];

        for (bedrock_stop_reason, expected_anthropic_stop_reason) in test_cases {
            let bedrock_response = ConverseResponse {
                output: ConverseOutput::Message {
                    message: BedrockMessage {
                        role: ConversationRole::Assistant,
                        content: vec![ContentBlock::Text {
                            text: "Test response".to_string(),
                        }],
                    },
                },
                stop_reason: bedrock_stop_reason,
                usage: BedrockTokenUsage {
                    input_tokens: 5,
                    output_tokens: 10,
                    total_tokens: 15,
                    ..Default::default()
                },
                metrics: None,
                trace: None,
                additional_model_response_fields: None,
                performance_config: None,
            };

            let anthropic_response: MessagesResponse = bedrock_response.try_into().unwrap();
            assert_eq!(
                anthropic_response.stop_reason,
                expected_anthropic_stop_reason
            );
        }
    }

    #[test]
    fn test_bedrock_to_anthropic_with_cache_tokens() {
        let bedrock_response = ConverseResponse {
            output: ConverseOutput::Message {
                message: BedrockMessage {
                    role: ConversationRole::Assistant,
                    content: vec![ContentBlock::Text {
                        text: "Cached response".to_string(),
                    }],
                },
            },
            stop_reason: StopReason::EndTurn,
            usage: BedrockTokenUsage {
                input_tokens: 100,
                output_tokens: 50,
                total_tokens: 150,
                cache_write_input_tokens: Some(20),
                cache_read_input_tokens: Some(10),
                ..Default::default()
            },
            metrics: None,
            trace: None,
            additional_model_response_fields: None,
            performance_config: None,
        };

        let anthropic_response: MessagesResponse = bedrock_response.try_into().unwrap();

        assert_eq!(anthropic_response.usage.input_tokens, 100);
        assert_eq!(anthropic_response.usage.output_tokens, 50);
        assert_eq!(
            anthropic_response.usage.cache_creation_input_tokens,
            Some(20)
        );
        assert_eq!(anthropic_response.usage.cache_read_input_tokens, Some(10));
    }

    #[test]
    fn test_bedrock_to_anthropic_with_tool_result() {
        let bedrock_response = ConverseResponse {
            output: ConverseOutput::Message {
                message: BedrockMessage {
                    role: ConversationRole::Assistant,
                    content: vec![
                        ContentBlock::Text {
                            text: "Here's the weather information:".to_string(),
                        },
                        ContentBlock::ToolResult {
                            tool_result: crate::apis::amazon_bedrock::ToolResultBlock {
                                tool_use_id: "tool_67890".to_string(),
                                content: vec![ToolResultContentBlock::Text {
                                    text: "Temperature: 72°F, Sunny".to_string(),
                                }],
                                status: Some(ToolResultStatus::Success),
                            },
                        },
                    ],
                },
            },
            stop_reason: StopReason::EndTurn,
            usage: BedrockTokenUsage {
                input_tokens: 20,
                output_tokens: 35,
                total_tokens: 55,
                cache_write_input_tokens: None,
                cache_read_input_tokens: None,
                ..Default::default()
            },
            metrics: None,
            trace: None,
            additional_model_response_fields: None,
            performance_config: None,
        };

        let anthropic_response: MessagesResponse = bedrock_response.try_into().unwrap();

        assert_eq!(anthropic_response.content.len(), 2);

        // Check text content
        if let MessagesContentBlock::Text { text, .. } = &anthropic_response.content[0] {
            assert_eq!(text, "Here's the weather information:");
        } else {
            panic!("Expected text content block");
        }

        // Check tool result content
        if let MessagesContentBlock::ToolResult {
            tool_use_id,
            content,
            ..
        } = &anthropic_response.content[1]
        {
            assert_eq!(tool_use_id, "tool_67890");
            if let ToolResultContent::Blocks(blocks) = content {
                assert_eq!(blocks.len(), 1);
                if let MessagesContentBlock::Text { text, .. } = &blocks[0] {
                    assert_eq!(text, "Temperature: 72°F, Sunny");
                } else {
                    panic!("Expected text content in tool result");
                }
            } else {
                panic!("Expected blocks in tool result content");
            }
        } else {
            panic!("Expected tool result content block");
        }
    }

    #[test]
    fn test_bedrock_to_anthropic_mixed_content() {
        let bedrock_response = ConverseResponse {
            output: ConverseOutput::Message {
                message: BedrockMessage {
                    role: ConversationRole::Assistant,
                    content: vec![
                        ContentBlock::Text {
                            text: "I can help with multiple tasks.".to_string(),
                        },
                        ContentBlock::ToolUse {
                            tool_use: crate::apis::amazon_bedrock::ToolUseBlock {
                                tool_use_id: "tool_1".to_string(),
                                name: "search".to_string(),
                                input: json!({"query": "weather"}),
                            },
                        },
                        ContentBlock::Text {
                            text: "Let me also check another source.".to_string(),
                        },
                        ContentBlock::ToolUse {
                            tool_use: crate::apis::amazon_bedrock::ToolUseBlock {
                                tool_use_id: "tool_2".to_string(),
                                name: "lookup".to_string(),
                                input: json!({"id": "12345"}),
                            },
                        },
                    ],
                },
            },
            stop_reason: StopReason::ToolUse,
            usage: BedrockTokenUsage {
                input_tokens: 25,
                output_tokens: 40,
                total_tokens: 65,
                cache_write_input_tokens: None,
                cache_read_input_tokens: None,
                ..Default::default()
            },
            metrics: None,
            trace: None,
            additional_model_response_fields: None,
            performance_config: None,
        };

        let anthropic_response: MessagesResponse = bedrock_response.try_into().unwrap();

        assert_eq!(anthropic_response.content.len(), 4);
        assert_eq!(anthropic_response.stop_reason, MessagesStopReason::ToolUse);

        // Verify the sequence: text -> tool_use -> text -> tool_use
        if let MessagesContentBlock::Text { text, .. } = &anthropic_response.content[0] {
            assert_eq!(text, "I can help with multiple tasks.");
        } else {
            panic!("Expected first content to be text");
        }

        if let MessagesContentBlock::ToolUse { id, name, .. } = &anthropic_response.content[1] {
            assert_eq!(id, "tool_1");
            assert_eq!(name, "search");
        } else {
            panic!("Expected second content to be tool use");
        }

        if let MessagesContentBlock::Text { text, .. } = &anthropic_response.content[2] {
            assert_eq!(text, "Let me also check another source.");
        } else {
            panic!("Expected third content to be text");
        }

        if let MessagesContentBlock::ToolUse { id, name, .. } = &anthropic_response.content[3] {
            assert_eq!(id, "tool_2");
            assert_eq!(name, "lookup");
        } else {
            panic!("Expected fourth content to be tool use");
        }
    }

    #[test]
    fn test_convert_bedrock_message_to_anthropic_content() {
        let bedrock_message = BedrockMessage {
            role: ConversationRole::Assistant,
            content: vec![
                ContentBlock::Text {
                    text: "Hello world!".to_string(),
                },
                ContentBlock::ToolUse {
                    tool_use: crate::apis::amazon_bedrock::ToolUseBlock {
                        tool_use_id: "test_tool".to_string(),
                        name: "test_function".to_string(),
                        input: json!({"param": "value"}),
                    },
                },
            ],
        };

        let content_blocks =
            convert_bedrock_message_to_anthropic_content(&bedrock_message).unwrap();

        assert_eq!(content_blocks.len(), 2);

        if let MessagesContentBlock::Text { text, .. } = &content_blocks[0] {
            assert_eq!(text, "Hello world!");
        } else {
            panic!("Expected text content block");
        }

        if let MessagesContentBlock::ToolUse {
            id, name, input, ..
        } = &content_blocks[1]
        {
            assert_eq!(id, "test_tool");
            assert_eq!(name, "test_function");
            assert_eq!(input["param"], "value");
        } else {
            panic!("Expected tool use content block");
        }
    }

    #[test]
    fn test_bedrock_to_anthropic_role_conversion() {
        // Test Assistant role
        let assistant_response = ConverseResponse {
            output: ConverseOutput::Message {
                message: BedrockMessage {
                    role: ConversationRole::Assistant,
                    content: vec![ContentBlock::Text {
                        text: "I am an assistant".to_string(),
                    }],
                },
            },
            stop_reason: StopReason::EndTurn,
            usage: BedrockTokenUsage {
                input_tokens: 5,
                output_tokens: 10,
                total_tokens: 15,
                ..Default::default()
            },
            metrics: None,
            trace: None,
            additional_model_response_fields: None,
            performance_config: None,
        };

        let anthropic_response: MessagesResponse = assistant_response.try_into().unwrap();
        assert_eq!(anthropic_response.role, MessagesRole::Assistant);

        // Test User role
        let user_response = ConverseResponse {
            output: ConverseOutput::Message {
                message: BedrockMessage {
                    role: ConversationRole::User,
                    content: vec![ContentBlock::Text {
                        text: "I am a user".to_string(),
                    }],
                },
            },
            stop_reason: StopReason::EndTurn,
            usage: BedrockTokenUsage {
                input_tokens: 5,
                output_tokens: 10,
                total_tokens: 15,
                ..Default::default()
            },
            metrics: None,
            trace: None,
            additional_model_response_fields: None,
            performance_config: None,
        };

        let anthropic_response: MessagesResponse = user_response.try_into().unwrap();
        assert_eq!(anthropic_response.role, MessagesRole::User);
    }

    #[test]
    fn test_bedrock_to_anthropic_model_extraction() {
        // Test model extraction from trace information
        let bedrock_response = ConverseResponse {
            output: ConverseOutput::Message {
                message: BedrockMessage {
                    role: ConversationRole::Assistant,
                    content: vec![ContentBlock::Text {
                        text: "Test response".to_string(),
                    }],
                },
            },
            stop_reason: StopReason::EndTurn,
            usage: BedrockTokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                total_tokens: 15,
                ..Default::default()
            },
            metrics: None,
            trace: Some(ConverseTrace {
                guardrail: None,
                prompt_router: Some(PromptRouterTrace {
                    invoked_model_id: "anthropic.claude-3-sonnet-20240229-v1:0".to_string(),
                }),
            }),
            additional_model_response_fields: None,
            performance_config: None,
        };

        let anthropic_response: MessagesResponse = bedrock_response.try_into().unwrap();

        // Should extract model ID from trace
        assert_eq!(
            anthropic_response.model,
            "anthropic.claude-3-sonnet-20240229-v1:0"
        );

        // Test fallback when no trace information is available
        let bedrock_response_no_trace = ConverseResponse {
            output: ConverseOutput::Message {
                message: BedrockMessage {
                    role: ConversationRole::Assistant,
                    content: vec![ContentBlock::Text {
                        text: "Test response".to_string(),
                    }],
                },
            },
            stop_reason: StopReason::EndTurn,
            usage: BedrockTokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                total_tokens: 15,
                ..Default::default()
            },
            metrics: None,
            trace: None,
            additional_model_response_fields: None,
            performance_config: None,
        };

        let anthropic_response_fallback: MessagesResponse =
            bedrock_response_no_trace.try_into().unwrap();

        // Should use fallback model name
        assert_eq!(anthropic_response_fallback.model, "bedrock-model");
    }
}
