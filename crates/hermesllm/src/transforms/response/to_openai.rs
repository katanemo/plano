use crate::apis::amazon_bedrock::{ConverseOutput, ConverseResponse, StopReason};
use crate::apis::anthropic::{MessagesContentBlock, MessagesResponse, MessagesUsage};
use crate::apis::openai::{
    ChatCompletionsResponse, Choice, FinishReason, MessageContent, ResponseMessage, Role, Usage,
};
use crate::apis::openai_responses::ResponsesAPIResponse;
use crate::clients::TransformError;
use crate::transforms::lib::*;

// ============================================================================
// MAIN RESPONSE TRANSFORMATIONS
// ============================================================================

// Usage Conversions
impl From<MessagesUsage> for Usage {
    fn from(val: MessagesUsage) -> Self {
        Usage {
            prompt_tokens: val.input_tokens,
            completion_tokens: val.output_tokens,
            total_tokens: val.input_tokens + val.output_tokens,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        }
    }
}

impl TryFrom<ChatCompletionsResponse> for ResponsesAPIResponse {
    type Error = TransformError;

    fn try_from(resp: ChatCompletionsResponse) -> Result<Self, Self::Error> {
        use crate::apis::openai_responses::{
            IncompleteDetails, IncompleteReason, OutputContent, OutputItem, OutputItemStatus,
            ResponseStatus, ResponseUsage, ResponsesAPIResponse,
        };

        // Convert the first choice's message to output items
        let output = if let Some(choice) = resp.choices.first() {
            let mut items = Vec::new();

            // Create a message output item from the response message
            let mut content = Vec::new();

            // Add text content if present
            if let Some(text) = &choice.message.content {
                content.push(OutputContent::OutputText {
                    text: text.clone(),
                    annotations: vec![],
                    logprobs: None,
                });
            }

            // Add audio content if present (audio is a Value, need to handle it carefully)
            if let Some(audio) = &choice.message.audio {
                // Audio is serde_json::Value, try to extract data and transcript
                if let Some(audio_obj) = audio.as_object() {
                    let data = audio_obj
                        .get("data")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    let transcript = audio_obj
                        .get("transcript")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    content.push(OutputContent::OutputAudio { data, transcript });
                }
            }

            // Add refusal content if present
            if let Some(refusal) = &choice.message.refusal {
                content.push(OutputContent::Refusal {
                    refusal: refusal.clone(),
                });
            }

            // Only add the message item if there's actual content (text, audio, or refusal)
            // Don't add empty message items when there are only tool calls
            if !content.is_empty() {
                // Generate message ID: strip common prefixes to avoid double-prefixing
                let message_id = if resp.id.starts_with("msg_") {
                    resp.id.clone()
                } else if resp.id.starts_with("resp_") {
                    format!("msg_{}", &resp.id[5..]) // Strip "resp_" prefix
                } else if resp.id.starts_with("chatcmpl-") {
                    format!("msg_{}", &resp.id[9..]) // Strip "chatcmpl-" prefix
                } else {
                    format!("msg_{}", resp.id)
                };

                items.push(OutputItem::Message {
                    id: message_id,
                    status: OutputItemStatus::Completed,
                    role: match choice.message.role {
                        Role::User => "user".to_string(),
                        Role::Assistant => "assistant".to_string(),
                        Role::System => "system".to_string(),
                        Role::Tool => "tool".to_string(),
                    },
                    content,
                });
            }

            // Add tool calls as function call items if present
            if let Some(tool_calls) = &choice.message.tool_calls {
                for tool_call in tool_calls {
                    items.push(OutputItem::FunctionCall {
                        id: format!("func_{}", tool_call.id),
                        status: OutputItemStatus::Completed,
                        call_id: tool_call.id.clone(),
                        name: Some(tool_call.function.name.clone()),
                        arguments: Some(tool_call.function.arguments.clone()),
                    });
                }
            }

            items
        } else {
            vec![]
        };

        // Convert finish_reason to status
        let status = if let Some(choice) = resp.choices.first() {
            match choice.finish_reason {
                Some(FinishReason::Stop) => ResponseStatus::Completed,
                Some(FinishReason::ToolCalls) => ResponseStatus::Completed,
                Some(FinishReason::Length) => ResponseStatus::Incomplete,
                Some(FinishReason::ContentFilter) => ResponseStatus::Failed,
                _ => ResponseStatus::Completed,
            }
        } else {
            ResponseStatus::Completed
        };

        // Convert usage
        let usage = ResponseUsage {
            input_tokens: resp.usage.prompt_tokens as i32,
            output_tokens: resp.usage.completion_tokens as i32,
            total_tokens: resp.usage.total_tokens as i32,
            input_tokens_details: resp.usage.prompt_tokens_details.map(|details| {
                crate::apis::openai_responses::TokenDetails {
                    cached_tokens: details.cached_tokens.unwrap_or(0) as i32,
                }
            }),
            output_tokens_details: resp.usage.completion_tokens_details.map(|details| {
                crate::apis::openai_responses::OutputTokenDetails {
                    reasoning_tokens: details.reasoning_tokens.unwrap_or(0) as i32,
                }
            }),
        };

        // Set incomplete_details if status is incomplete
        let incomplete_details = if matches!(status, ResponseStatus::Incomplete) {
            Some(IncompleteDetails {
                reason: IncompleteReason::MaxOutputTokens,
            })
        } else {
            None
        };

        Ok(ResponsesAPIResponse {
            // Generate proper resp_ prefixed ID if not already present
            id: if resp.id.starts_with("resp_") {
                resp.id
            } else {
                format!("resp_{}", uuid::Uuid::new_v4().to_string().replace("-", ""))
            },
            object: "response".to_string(),
            created_at: resp.created as i64,
            status,
            background: Some(false),
            error: None,
            incomplete_details,
            instructions: None,
            max_output_tokens: None,
            max_tool_calls: None,
            model: resp.model,
            output,
            usage: Some(usage),
            parallel_tool_calls: true,
            conversation: None,
            previous_response_id: None,
            tools: vec![],
            tool_choice: "auto".to_string(),
            temperature: 1.0,
            top_p: 1.0,
            metadata: resp.metadata.unwrap_or_default(),
            truncation: None,
            reasoning: Some(crate::apis::openai_responses::Reasoning {
                effort: None,
                summary: None,
            }),
            store: None,
            text: None,
            audio: None,
            modalities: None,
            service_tier: resp.service_tier,
            top_logprobs: None,
        })
    }
}

impl TryFrom<MessagesResponse> for ChatCompletionsResponse {
    type Error = TransformError;

    fn try_from(resp: MessagesResponse) -> Result<Self, Self::Error> {
        let content = convert_anthropic_content_to_openai(&resp.content)?;
        let finish_reason: FinishReason = resp.stop_reason.into();
        let tool_calls = resp.content.extract_tool_calls()?;

        // Convert MessageContent to String for response
        let content_string = match content {
            MessageContent::Text(text) => Some(text),
            MessageContent::Parts(parts) => {
                let text = parts.extract_text();
                if text.is_empty() {
                    None
                } else {
                    Some(text)
                }
            }
        };

        let message = ResponseMessage {
            role: Role::Assistant,
            content: content_string,
            refusal: None,
            annotations: None,
            audio: None,
            function_call: None,
            tool_calls,
        };

        let choice = Choice {
            index: 0,
            message,
            finish_reason: Some(finish_reason),
            logprobs: None,
        };

        let usage = Usage {
            prompt_tokens: resp.usage.input_tokens,
            completion_tokens: resp.usage.output_tokens,
            total_tokens: resp.usage.input_tokens + resp.usage.output_tokens,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        };

        Ok(ChatCompletionsResponse {
            id: resp.id,
            object: Some("chat.completion".to_string()),
            created: current_timestamp(),
            model: resp.model,
            choices: vec![choice],
            usage,
            ..Default::default()
        })
    }
}

impl TryFrom<ConverseResponse> for ChatCompletionsResponse {
    type Error = TransformError;

    fn try_from(resp: ConverseResponse) -> Result<Self, Self::Error> {
        // Extract the message from the ConverseOutput
        let message = match resp.output {
            ConverseOutput::Message { message } => message,
        };

        // Convert Bedrock ConversationRole to OpenAI Role
        let role = match message.role {
            crate::apis::amazon_bedrock::ConversationRole::User => Role::User,
            crate::apis::amazon_bedrock::ConversationRole::Assistant => Role::Assistant,
        };

        // Convert Bedrock message content to OpenAI format
        let (content, tool_calls) = convert_bedrock_message_to_openai(&message)?;

        // Convert Bedrock stop reason to OpenAI finish reason
        let finish_reason = match resp.stop_reason {
            StopReason::EndTurn => FinishReason::Stop,
            StopReason::ToolUse => FinishReason::ToolCalls,
            StopReason::MaxTokens => FinishReason::Length,
            StopReason::StopSequence => FinishReason::Stop,
            StopReason::GuardrailIntervened => FinishReason::ContentFilter,
            StopReason::ContentFiltered => FinishReason::ContentFilter,
        };

        // Create response message
        let response_message = ResponseMessage {
            role,
            content,
            refusal: None,
            annotations: None,
            audio: None,
            function_call: None,
            tool_calls,
        };

        // Create choice
        let choice = Choice {
            index: 0,
            message: response_message,
            finish_reason: Some(finish_reason),
            logprobs: None,
        };

        // Convert token usage
        let usage = Usage {
            prompt_tokens: resp.usage.input_tokens,
            completion_tokens: resp.usage.output_tokens,
            total_tokens: resp.usage.total_tokens,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        };

        // Generate a response ID (using timestamp since Bedrock doesn't provide one)
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

        Ok(ChatCompletionsResponse {
            id,
            object: Some("chat.completion".to_string()),
            created: current_timestamp(),
            model,
            choices: vec![choice],
            usage,
            ..Default::default()
        })
    }
}

/// Convert Bedrock Message to OpenAI content and tool calls
/// This function extracts text content and tool calls from a Bedrock message
fn convert_bedrock_message_to_openai(
    message: &crate::apis::amazon_bedrock::Message,
) -> Result<(Option<String>, Option<Vec<crate::apis::openai::ToolCall>>), TransformError> {
    use crate::apis::amazon_bedrock::ContentBlock;
    use crate::apis::openai::{FunctionCall, ToolCall};

    let mut text_content = String::new();
    let mut tool_calls = Vec::new();

    for content_block in &message.content {
        match content_block {
            ContentBlock::Text { text } => {
                text_content.push_str(text);
            }
            ContentBlock::ToolUse { tool_use } => {
                tool_calls.push(ToolCall {
                    id: tool_use.tool_use_id.clone(),
                    call_type: "function".to_string(),
                    function: FunctionCall {
                        name: tool_use.name.clone(),
                        arguments: serde_json::to_string(&tool_use.input).unwrap_or_default(),
                    },
                });
            }
            _ => continue,
        }
    }

    let content = if text_content.is_empty() {
        None
    } else {
        Some(text_content)
    };
    let tool_calls = if tool_calls.is_empty() {
        None
    } else {
        Some(tool_calls)
    };

    Ok((content, tool_calls))
}

/// Convert Anthropic content blocks to OpenAI message content
fn convert_anthropic_content_to_openai(
    content: &[MessagesContentBlock],
) -> Result<MessageContent, TransformError> {
    let mut text_parts = Vec::new();

    for block in content {
        match block {
            MessagesContentBlock::Text { text, .. } => {
                text_parts.push(text.clone());
            }
            MessagesContentBlock::Thinking { thinking, .. } => {
                text_parts.push(format!("thinking: {}", thinking));
            }
            _ => {
                // Skip other content types for basic text conversion
                continue;
            }
        }
    }

    Ok(MessageContent::Text(text_parts.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apis::amazon_bedrock::{
        BedrockTokenUsage, ContentBlock, ConversationRole, ConverseOutput, ConverseResponse,
        ConverseTrace, Message as BedrockMessage, PromptRouterTrace, StopReason,
    };
    use crate::apis::openai::{ChatCompletionsResponse, FinishReason, Role};
    use serde_json::json;

    #[test]
    fn test_bedrock_to_openai_basic_response() {
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
                ..Default::default()
            },
            metrics: None,
            trace: None,
            additional_model_response_fields: None,
            performance_config: None,
        };

        let openai_response: ChatCompletionsResponse = bedrock_response.try_into().unwrap();

        assert_eq!(openai_response.object, Some("chat.completion".to_string()));
        assert_eq!(openai_response.model, "bedrock-model");
        assert!(openai_response.id.starts_with("bedrock-"));

        // Check choices
        assert_eq!(openai_response.choices.len(), 1);
        let choice = &openai_response.choices[0];
        assert_eq!(choice.index, 0);
        assert_eq!(choice.message.role, Role::Assistant);
        assert_eq!(
            choice.message.content,
            Some("Hello! How can I help you today?".to_string())
        );
        assert_eq!(choice.finish_reason, Some(FinishReason::Stop));
        assert!(choice.message.tool_calls.is_none());

        // Check usage
        assert_eq!(openai_response.usage.prompt_tokens, 10);
        assert_eq!(openai_response.usage.completion_tokens, 25);
        assert_eq!(openai_response.usage.total_tokens, 35);
    }

    #[test]
    fn test_bedrock_to_openai_with_tool_use() {
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
                ..Default::default()
            },
            metrics: None,
            trace: None,
            additional_model_response_fields: None,
            performance_config: None,
        };

        let openai_response: ChatCompletionsResponse = bedrock_response.try_into().unwrap();

        assert_eq!(
            openai_response.choices[0].finish_reason,
            Some(FinishReason::ToolCalls)
        );
        assert_eq!(
            openai_response.choices[0].message.content,
            Some("I'll help you check the weather.".to_string())
        );

        // Check tool calls
        let tool_calls = openai_response.choices[0]
            .message
            .tool_calls
            .as_ref()
            .unwrap();
        assert_eq!(tool_calls.len(), 1);

        let tool_call = &tool_calls[0];
        assert_eq!(tool_call.id, "tool_12345");
        assert_eq!(tool_call.call_type, "function");
        assert_eq!(tool_call.function.name, "get_weather");

        let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments).unwrap();
        assert_eq!(args["location"], "San Francisco");
    }

    #[test]
    fn test_bedrock_to_openai_stop_reason_conversions() {
        let test_cases = vec![
            (StopReason::EndTurn, FinishReason::Stop),
            (StopReason::ToolUse, FinishReason::ToolCalls),
            (StopReason::MaxTokens, FinishReason::Length),
            (StopReason::StopSequence, FinishReason::Stop),
            (StopReason::GuardrailIntervened, FinishReason::ContentFilter),
            (StopReason::ContentFiltered, FinishReason::ContentFilter),
        ];

        for (bedrock_stop_reason, expected_openai_finish_reason) in test_cases {
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

            let openai_response: ChatCompletionsResponse = bedrock_response.try_into().unwrap();
            assert_eq!(
                openai_response.choices[0].finish_reason,
                Some(expected_openai_finish_reason)
            );
        }
    }

    #[test]
    fn test_bedrock_to_openai_multiple_tool_calls() {
        let bedrock_response = ConverseResponse {
            output: ConverseOutput::Message {
                message: BedrockMessage {
                    role: ConversationRole::Assistant,
                    content: vec![
                        ContentBlock::Text {
                            text: "I'll help with multiple tasks.".to_string(),
                        },
                        ContentBlock::ToolUse {
                            tool_use: crate::apis::amazon_bedrock::ToolUseBlock {
                                tool_use_id: "tool_1".to_string(),
                                name: "search".to_string(),
                                input: json!({"query": "weather"}),
                            },
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
                ..Default::default()
            },
            metrics: None,
            trace: None,
            additional_model_response_fields: None,
            performance_config: None,
        };

        let openai_response: ChatCompletionsResponse = bedrock_response.try_into().unwrap();

        assert_eq!(
            openai_response.choices[0].finish_reason,
            Some(FinishReason::ToolCalls)
        );
        assert_eq!(
            openai_response.choices[0].message.content,
            Some("I'll help with multiple tasks.".to_string())
        );

        // Check multiple tool calls
        let tool_calls = openai_response.choices[0]
            .message
            .tool_calls
            .as_ref()
            .unwrap();
        assert_eq!(tool_calls.len(), 2);

        // First tool call
        assert_eq!(tool_calls[0].id, "tool_1");
        assert_eq!(tool_calls[0].function.name, "search");
        let args1: serde_json::Value =
            serde_json::from_str(&tool_calls[0].function.arguments).unwrap();
        assert_eq!(args1["query"], "weather");

        // Second tool call
        assert_eq!(tool_calls[1].id, "tool_2");
        assert_eq!(tool_calls[1].function.name, "lookup");
        let args2: serde_json::Value =
            serde_json::from_str(&tool_calls[1].function.arguments).unwrap();
        assert_eq!(args2["id"], "12345");
    }

    #[test]
    fn test_bedrock_to_openai_mixed_content() {
        let bedrock_response = ConverseResponse {
            output: ConverseOutput::Message {
                message: BedrockMessage {
                    role: ConversationRole::Assistant,
                    content: vec![
                        ContentBlock::Text {
                            text: "First part. ".to_string(),
                        },
                        ContentBlock::ToolUse {
                            tool_use: crate::apis::amazon_bedrock::ToolUseBlock {
                                tool_use_id: "tool_mid".to_string(),
                                name: "calculate".to_string(),
                                input: json!({"expr": "2+2"}),
                            },
                        },
                        ContentBlock::Text {
                            text: "Second part.".to_string(),
                        },
                    ],
                },
            },
            stop_reason: StopReason::ToolUse,
            usage: BedrockTokenUsage {
                input_tokens: 20,
                output_tokens: 35,
                total_tokens: 55,
                ..Default::default()
            },
            metrics: None,
            trace: None,
            additional_model_response_fields: None,
            performance_config: None,
        };

        let openai_response: ChatCompletionsResponse = bedrock_response.try_into().unwrap();

        // Content should be combined text parts (no separator added)
        assert_eq!(
            openai_response.choices[0].message.content,
            Some("First part. Second part.".to_string())
        );

        // Should have one tool call
        let tool_calls = openai_response.choices[0]
            .message
            .tool_calls
            .as_ref()
            .unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "tool_mid");
        assert_eq!(tool_calls[0].function.name, "calculate");
    }

    #[test]
    fn test_bedrock_to_openai_empty_content() {
        let bedrock_response = ConverseResponse {
            output: ConverseOutput::Message {
                message: BedrockMessage {
                    role: ConversationRole::Assistant,
                    content: vec![ContentBlock::ToolUse {
                        tool_use: crate::apis::amazon_bedrock::ToolUseBlock {
                            tool_use_id: "tool_only".to_string(),
                            name: "action".to_string(),
                            input: json!({}),
                        },
                    }],
                },
            },
            stop_reason: StopReason::ToolUse,
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

        let openai_response: ChatCompletionsResponse = bedrock_response.try_into().unwrap();

        // Content should be None when there's no text
        assert_eq!(openai_response.choices[0].message.content, None);

        // Should have tool call
        let tool_calls = openai_response.choices[0]
            .message
            .tool_calls
            .as_ref()
            .unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "tool_only");
    }

    #[test]
    fn test_convert_bedrock_message_to_openai() {
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

        let (content, tool_calls) = convert_bedrock_message_to_openai(&bedrock_message).unwrap();

        assert_eq!(content, Some("Hello world!".to_string()));

        let tool_calls = tool_calls.unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "test_tool");
        assert_eq!(tool_calls[0].function.name, "test_function");

        let args: serde_json::Value =
            serde_json::from_str(&tool_calls[0].function.arguments).unwrap();
        assert_eq!(args["param"], "value");
    }

    #[test]
    fn test_bedrock_to_openai_role_conversion() {
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

        let openai_response: ChatCompletionsResponse = assistant_response.try_into().unwrap();
        assert_eq!(openai_response.choices[0].message.role, Role::Assistant);

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

        let openai_response: ChatCompletionsResponse = user_response.try_into().unwrap();
        assert_eq!(openai_response.choices[0].message.role, Role::User);
    }

    #[test]
    fn test_bedrock_to_openai_model_extraction() {
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

        let openai_response: ChatCompletionsResponse = bedrock_response.try_into().unwrap();

        // Should extract model ID from trace
        assert_eq!(
            openai_response.model,
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

        let openai_response_fallback: ChatCompletionsResponse =
            bedrock_response_no_trace.try_into().unwrap();

        // Should use fallback model name
        assert_eq!(openai_response_fallback.model, "bedrock-model");
    }

    #[test]
    fn test_bedrock_to_openai_with_multimedia_content() {
        use crate::apis::amazon_bedrock::ImageSource;

        let bedrock_response = ConverseResponse {
            output: ConverseOutput::Message {
                message: BedrockMessage {
                    role: ConversationRole::Assistant,
                    content: vec![
                        ContentBlock::Text {
                            text: "Here's the analysis:".to_string(),
                        },
                        ContentBlock::Image {
                            image: crate::apis::amazon_bedrock::ImageBlock {
                                source: ImageSource::Base64 {
                                    media_type: "image/jpeg".to_string(),
                                    data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==".to_string(),
                                },
                            },
                        }
                    ],
                },
            },
            stop_reason: StopReason::EndTurn,
            usage: BedrockTokenUsage {
                input_tokens: 50,
                output_tokens: 75,
                total_tokens: 125,
                ..Default::default()
            },
            metrics: None,
            trace: None,
            additional_model_response_fields: None,
            performance_config: None,
        };

        let openai_response: ChatCompletionsResponse = bedrock_response.try_into().unwrap();

        assert_eq!(
            openai_response.choices[0].finish_reason,
            Some(FinishReason::Stop)
        );

        let content = openai_response.choices[0].message.content.as_ref().unwrap();

        // Check that text content is preserved (image blocks are currently ignored)
        assert!(content.contains("Here's the analysis:"));
        // Note: Image blocks are not converted to text in the current implementation
    }

    #[test]
    fn test_chat_completions_to_responses_api_basic() {
        use crate::apis::openai_responses::{OutputContent, OutputItem, ResponsesAPIResponse};

        let chat_response = ChatCompletionsResponse {
            id: "resp_6de5512800cf4375a329a473a4f02879".to_string(),
            object: Some("chat.completion".to_string()),
            created: 1677652288,
            model: "gpt-4".to_string(),
            choices: vec![Choice {
                index: 0,
                message: crate::apis::openai::ResponseMessage {
                    role: Role::Assistant,
                    content: Some("Hello! How can I help you?".to_string()),
                    refusal: None,
                    annotations: None,
                    audio: None,
                    function_call: None,
                    tool_calls: None,
                },
                finish_reason: Some(FinishReason::Stop),
                logprobs: None,
            }],
            usage: Usage {
                prompt_tokens: 10,
                completion_tokens: 20,
                total_tokens: 30,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            system_fingerprint: None,
            service_tier: Some("default".to_string()),
            metadata: None,
        };

        let responses_api: ResponsesAPIResponse = chat_response.try_into().unwrap();

        // Response ID should be generated with resp_ prefix
        assert!(
            responses_api.id.starts_with("resp_"),
            "Response ID should start with 'resp_'"
        );
        assert_eq!(
            responses_api.id.len(),
            37,
            "Response ID should be resp_ + 32 char UUID"
        );
        assert_eq!(responses_api.object, "response");
        assert_eq!(responses_api.model, "gpt-4");

        // Check usage conversion
        let usage = responses_api.usage.unwrap();
        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.output_tokens, 20);
        assert_eq!(usage.total_tokens, 30);

        // Check output items
        assert_eq!(responses_api.output.len(), 1);
        match &responses_api.output[0] {
            OutputItem::Message { role, content, .. } => {
                assert_eq!(role, "assistant");
                assert_eq!(content.len(), 1);
                match &content[0] {
                    OutputContent::OutputText { text, .. } => {
                        assert_eq!(text, "Hello! How can I help you?");
                    }
                    _ => panic!("Expected OutputText content"),
                }
            }
            _ => panic!("Expected Message output item"),
        }
    }

    #[test]
    fn test_chat_completions_to_responses_api_with_tool_calls() {
        use crate::apis::openai::{FunctionCall, ToolCall};
        use crate::apis::openai_responses::{OutputItem, ResponsesAPIResponse};

        let chat_response = ChatCompletionsResponse {
            id: "chatcmpl-456".to_string(),
            object: Some("chat.completion".to_string()),
            created: 1677652300,
            model: "gpt-4".to_string(),
            choices: vec![Choice {
                index: 0,
                message: crate::apis::openai::ResponseMessage {
                    role: Role::Assistant,
                    content: Some("Let me check the weather.".to_string()),
                    refusal: None,
                    annotations: None,
                    audio: None,
                    function_call: None,
                    tool_calls: Some(vec![ToolCall {
                        id: "call_abc123".to_string(),
                        call_type: "function".to_string(),
                        function: FunctionCall {
                            name: "get_weather".to_string(),
                            arguments: r#"{"location":"San Francisco"}"#.to_string(),
                        },
                    }]),
                },
                finish_reason: Some(FinishReason::ToolCalls),
                logprobs: None,
            }],
            usage: Usage {
                prompt_tokens: 15,
                completion_tokens: 25,
                total_tokens: 40,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            system_fingerprint: None,
            service_tier: None,
            metadata: None,
        };

        let responses_api: ResponsesAPIResponse = chat_response.try_into().unwrap();

        // Should have 2 output items: message + function call
        assert_eq!(responses_api.output.len(), 2);

        // Check message item
        match &responses_api.output[0] {
            OutputItem::Message { content, .. } => {
                assert_eq!(content.len(), 1);
            }
            _ => panic!("Expected Message output item"),
        }

        // Check function call item
        match &responses_api.output[1] {
            OutputItem::FunctionCall {
                call_id,
                name,
                arguments,
                ..
            } => {
                assert_eq!(call_id, "call_abc123");
                assert_eq!(name.as_ref().unwrap(), "get_weather");
                assert!(arguments.as_ref().unwrap().contains("San Francisco"));
            }
            _ => panic!("Expected FunctionCall output item"),
        }
    }

    #[test]
    fn test_chat_completions_to_responses_api_tool_calls_only() {
        use crate::apis::openai::{FunctionCall, ToolCall};
        use crate::apis::openai_responses::{OutputItem, ResponsesAPIResponse};

        // Test the real-world case where content is null and there are only tool calls
        let chat_response = ChatCompletionsResponse {
            id: "chatcmpl-789".to_string(),
            object: Some("chat.completion".to_string()),
            created: 1764023939,
            model: "gpt-4o-2024-08-06".to_string(),
            choices: vec![Choice {
                index: 0,
                message: crate::apis::openai::ResponseMessage {
                    role: Role::Assistant,
                    content: None, // No text content, only tool calls
                    refusal: None,
                    annotations: None,
                    audio: None,
                    function_call: None,
                    tool_calls: Some(vec![ToolCall {
                        id: "call_oJBtqTJmRfBGlFS55QhMfUUV".to_string(),
                        call_type: "function".to_string(),
                        function: FunctionCall {
                            name: "get_weather".to_string(),
                            arguments: r#"{"location":"San Francisco, CA"}"#.to_string(),
                        },
                    }]),
                },
                finish_reason: Some(FinishReason::ToolCalls),
                logprobs: None,
            }],
            usage: Usage {
                prompt_tokens: 84,
                completion_tokens: 17,
                total_tokens: 101,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            },
            system_fingerprint: Some("fp_7eeb46f068".to_string()),
            service_tier: Some("default".to_string()),
            metadata: None,
        };

        let responses_api: ResponsesAPIResponse = chat_response.try_into().unwrap();

        // Should have only 1 output item: function call (no empty message item)
        assert_eq!(responses_api.output.len(), 1);

        // Check function call item
        match &responses_api.output[0] {
            OutputItem::FunctionCall {
                call_id,
                name,
                arguments,
                ..
            } => {
                assert_eq!(call_id, "call_oJBtqTJmRfBGlFS55QhMfUUV");
                assert_eq!(name.as_ref().unwrap(), "get_weather");
                assert!(arguments.as_ref().unwrap().contains("San Francisco, CA"));
            }
            _ => panic!("Expected FunctionCall output item as first item"),
        }

        // Verify status is Completed for tool_calls finish reason
        assert!(matches!(
            responses_api.status,
            crate::apis::openai_responses::ResponseStatus::Completed
        ));
    }
}
