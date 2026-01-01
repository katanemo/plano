use crate::apis::amazon_bedrock::{
    AnyChoice, AutoChoice, ContentBlock, ConversationRole, ConverseRequest, ImageBlock,
    ImageSource, InferenceConfiguration, Message as BedrockMessage, SystemContentBlock,
    Tool as BedrockTool, ToolChoice as BedrockToolChoice, ToolChoiceSpec, ToolConfiguration,
    ToolInputSchema, ToolResultBlock, ToolResultContentBlock, ToolResultStatus, ToolSpecDefinition,
    ToolUseBlock,
};
use crate::apis::anthropic::{
    MessagesMessage, MessagesMessageContent, MessagesRequest, MessagesRole, MessagesStopReason,
    MessagesSystemPrompt, MessagesTool, MessagesToolChoice, MessagesToolChoiceType, MessagesUsage,
    ToolResultContent,
};
use crate::apis::openai::{
    ChatCompletionsRequest, ContentPart, FinishReason, Function, FunctionChoice, Message,
    MessageContent, Role, Tool, ToolCall, ToolChoice, ToolChoiceType, Usage,
};
use crate::clients::TransformError;
use crate::transforms::lib::*;

type AnthropicMessagesRequest = MessagesRequest;

// Conversion from Anthropic MessagesRequest to OpenAI ChatCompletionsRequest
impl TryFrom<AnthropicMessagesRequest> for ChatCompletionsRequest {
    type Error = TransformError;

    fn try_from(req: AnthropicMessagesRequest) -> Result<Self, Self::Error> {
        let mut openai_messages: Vec<Message> = Vec::new();

        // Convert system prompt to system message if present
        if let Some(system) = req.system {
            openai_messages.push(system.into());
        }

        // Convert messages
        for message in req.messages {
            let converted_messages: Vec<Message> = message.try_into()?;
            openai_messages.extend(converted_messages);
        }

        // Convert tools and tool choice
        let openai_tools = req.tools.map(convert_anthropic_tools);
        let (openai_tool_choice, parallel_tool_calls) =
            convert_anthropic_tool_choice(req.tool_choice);

        let mut _chat_completions_req: ChatCompletionsRequest = ChatCompletionsRequest {
            model: req.model,
            messages: openai_messages,
            temperature: req.temperature,
            top_p: req.top_p,
            max_completion_tokens: Some(req.max_tokens),
            stream: req.stream,
            stop: req.stop_sequences,
            tools: openai_tools,
            tool_choice: openai_tool_choice,
            parallel_tool_calls,
            ..Default::default()
        };
        _chat_completions_req.suppress_max_tokens_if_o3();
        _chat_completions_req.fix_temperature_if_gpt5();
        Ok(_chat_completions_req)
    }
}

// Conversion from Anthropic MessagesRequest to Amazon Bedrock ConverseRequest
impl TryFrom<AnthropicMessagesRequest> for ConverseRequest {
    type Error = TransformError;

    fn try_from(req: AnthropicMessagesRequest) -> Result<Self, Self::Error> {
        // Convert system prompt to SystemContentBlock if present
        let system: Option<Vec<SystemContentBlock>> = req.system.map(|system_prompt| {
            let text = match system_prompt {
                MessagesSystemPrompt::Single(text) => text,
                MessagesSystemPrompt::Blocks(blocks) => blocks.extract_text(),
            };
            vec![SystemContentBlock::Text { text }]
        });

        // Convert messages to Bedrock format
        let messages = if req.messages.is_empty() {
            None
        } else {
            let mut bedrock_messages = Vec::new();
            for anthropic_message in req.messages {
                let bedrock_message: BedrockMessage = anthropic_message.try_into()?;
                bedrock_messages.push(bedrock_message);
            }
            Some(bedrock_messages)
        };

        // Build inference configuration
        // Anthropic always requires max_tokens, so we should always include inferenceConfig
        let inference_config = Some(InferenceConfiguration {
            max_tokens: Some(req.max_tokens),
            temperature: req.temperature,
            top_p: req.top_p,
            stop_sequences: req.stop_sequences,
        });

        // Convert tools and tool choice to ToolConfiguration
        // Only include toolConfig if we have actual tools (Bedrock requires at least 1 tool)
        let tool_config = req.tools.and_then(|anthropic_tools| {
            if anthropic_tools.is_empty() {
                return None;
            }

            let tools = anthropic_tools
                .into_iter()
                .map(|tool| BedrockTool::ToolSpec {
                    tool_spec: ToolSpecDefinition {
                        name: tool.name,
                        description: tool.description,
                        input_schema: ToolInputSchema {
                            json: tool.input_schema,
                        },
                    },
                })
                .collect();

            let tool_choice = req.tool_choice.map(|choice| {
                match choice.kind {
                    MessagesToolChoiceType::Auto => BedrockToolChoice::Auto {
                        auto: AutoChoice {},
                    },
                    MessagesToolChoiceType::Any => BedrockToolChoice::Any { any: AnyChoice {} },
                    MessagesToolChoiceType::None => BedrockToolChoice::Auto {
                        auto: AutoChoice {},
                    }, // Bedrock doesn't have explicit "none"
                    MessagesToolChoiceType::Tool => {
                        if let Some(name) = choice.name {
                            BedrockToolChoice::Tool {
                                tool: ToolChoiceSpec { name },
                            }
                        } else {
                            BedrockToolChoice::Auto {
                                auto: AutoChoice {},
                            }
                        }
                    }
                }
            });

            Some(ToolConfiguration {
                tools: Some(tools),
                tool_choice,
            })
        });

        Ok(ConverseRequest {
            model_id: req.model,
            messages,
            system,
            inference_config,
            tool_config,
            stream: req.stream.unwrap_or(false),
            guardrail_config: None,
            additional_model_request_fields: None,
            additional_model_response_field_paths: None,
            performance_config: None,
            prompt_variables: None,
            request_metadata: None,
            metadata: None,
        })
    }
}

// Message Conversions
impl TryFrom<MessagesMessage> for Vec<Message> {
    type Error = TransformError;

    fn try_from(message: MessagesMessage) -> Result<Self, Self::Error> {
        let mut result = Vec::new();

        match message.content {
            MessagesMessageContent::Single(text) => {
                result.push(Message {
                    role: message.role.into(),
                    content: MessageContent::Text(text),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                });
            }
            MessagesMessageContent::Blocks(blocks) => {
                let (content_parts, tool_calls, tool_results) = blocks.split_for_openai()?;
                // Add tool result messages
                for (tool_use_id, result_text, _is_error) in tool_results {
                    result.push(Message {
                        role: Role::Tool,
                        content: MessageContent::Text(result_text),
                        name: None,
                        tool_calls: None,
                        tool_call_id: Some(tool_use_id),
                    });
                }

                // Only create main message if there's actual content or tool calls
                // Skip creating empty content messages (e.g., when message only contains tool_result blocks)
                if !content_parts.is_empty() || !tool_calls.is_empty() {
                    let content = build_openai_content(content_parts, &tool_calls);
                    let main_message = Message {
                        role: message.role.into(),
                        content,
                        name: None,
                        tool_calls: if tool_calls.is_empty() {
                            None
                        } else {
                            Some(tool_calls)
                        },
                        tool_call_id: None,
                    };
                    result.push(main_message);
                }
            }
        }

        Ok(result)
    }
}

// Role Conversions
impl From<MessagesRole> for Role {
    fn from(val: MessagesRole) -> Self {
        match val {
            MessagesRole::User => Role::User,
            MessagesRole::Assistant => Role::Assistant,
        }
    }
}

impl From<FinishReason> for MessagesStopReason {
    fn from(val: FinishReason) -> Self {
        match val {
            FinishReason::Stop => MessagesStopReason::EndTurn,
            FinishReason::Length => MessagesStopReason::MaxTokens,
            FinishReason::ToolCalls => MessagesStopReason::ToolUse,
            FinishReason::ContentFilter => MessagesStopReason::Refusal,
            FinishReason::FunctionCall => MessagesStopReason::ToolUse,
        }
    }
}

impl From<Usage> for MessagesUsage {
    fn from(val: Usage) -> Self {
        MessagesUsage {
            input_tokens: val.prompt_tokens,
            output_tokens: val.completion_tokens,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        }
    }
}

// System Prompt Conversions
impl From<MessagesSystemPrompt> for Message {
    fn from(val: MessagesSystemPrompt) -> Self {
        let system_content = match val {
            MessagesSystemPrompt::Single(text) => MessageContent::Text(text),
            MessagesSystemPrompt::Blocks(blocks) => MessageContent::Text(blocks.extract_text()),
        };

        Message {
            role: Role::System,
            content: system_content,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }
}

//Utility Functions
/// Convert Anthropic tools to OpenAI format
fn convert_anthropic_tools(tools: Vec<MessagesTool>) -> Vec<Tool> {
    tools
        .into_iter()
        .map(|tool| Tool {
            tool_type: "function".to_string(),
            function: Function {
                name: tool.name,
                description: tool.description,
                parameters: tool.input_schema,
                strict: None,
            },
        })
        .collect()
}

/// Convert Anthropic tool choice to OpenAI format
fn convert_anthropic_tool_choice(
    tool_choice: Option<MessagesToolChoice>,
) -> (Option<ToolChoice>, Option<bool>) {
    match tool_choice {
        Some(choice) => {
            let openai_choice = match choice.kind {
                MessagesToolChoiceType::Auto => ToolChoice::Type(ToolChoiceType::Auto),
                MessagesToolChoiceType::Any => ToolChoice::Type(ToolChoiceType::Required),
                MessagesToolChoiceType::None => ToolChoice::Type(ToolChoiceType::None),
                MessagesToolChoiceType::Tool => {
                    if let Some(name) = choice.name {
                        ToolChoice::Function {
                            choice_type: "function".to_string(),
                            function: FunctionChoice { name },
                        }
                    } else {
                        ToolChoice::Type(ToolChoiceType::Auto)
                    }
                }
            };
            let parallel = choice.disable_parallel_tool_use.map(|disable| !disable);
            (Some(openai_choice), parallel)
        }
        None => (None, None),
    }
}

/// Build OpenAI message content from parts and tool calls
fn build_openai_content(
    content_parts: Vec<ContentPart>,
    tool_calls: &[ToolCall],
) -> MessageContent {
    if content_parts.len() == 1 && tool_calls.is_empty() {
        match &content_parts[0] {
            ContentPart::Text { text } => MessageContent::Text(text.clone()),
            _ => MessageContent::Parts(content_parts),
        }
    } else if content_parts.is_empty() {
        MessageContent::Text("".to_string())
    } else {
        MessageContent::Parts(content_parts)
    }
}

impl TryFrom<MessagesMessage> for BedrockMessage {
    type Error = TransformError;

    fn try_from(message: MessagesMessage) -> Result<Self, Self::Error> {
        let role = match message.role {
            MessagesRole::User => ConversationRole::User,
            MessagesRole::Assistant => ConversationRole::Assistant,
        };

        let mut content_blocks = Vec::new();

        // Convert content blocks
        match message.content {
            MessagesMessageContent::Single(text) => {
                if !text.is_empty() {
                    content_blocks.push(ContentBlock::Text { text });
                }
            }
            MessagesMessageContent::Blocks(blocks) => {
                for block in blocks {
                    match block {
                        crate::apis::anthropic::MessagesContentBlock::Text { text, .. } => {
                            if !text.is_empty() {
                                content_blocks.push(ContentBlock::Text { text });
                            }
                        }
                        crate::apis::anthropic::MessagesContentBlock::ToolUse {
                            id,
                            name,
                            input,
                            ..
                        } => {
                            content_blocks.push(ContentBlock::ToolUse {
                                tool_use: ToolUseBlock {
                                    tool_use_id: id,
                                    name,
                                    input,
                                },
                            });
                        }
                        crate::apis::anthropic::MessagesContentBlock::ToolResult {
                            tool_use_id,
                            is_error,
                            content,
                            ..
                        } => {
                            // Convert Anthropic ToolResultContent to Bedrock ToolResultContentBlock
                            let tool_result_content = match content {
                                ToolResultContent::Text(text) => {
                                    vec![ToolResultContentBlock::Text { text }]
                                }
                                ToolResultContent::Blocks(blocks) => {
                                    let mut result_blocks = Vec::new();
                                    for result_block in blocks {
                                        if let crate::apis::anthropic::MessagesContentBlock::Text { text, .. } = result_block {
                                            result_blocks.push(ToolResultContentBlock::Text { text });
                                        }
                                    }
                                    result_blocks
                                }
                            };

                            // Ensure we have at least one content block
                            let final_content = if tool_result_content.is_empty() {
                                vec![ToolResultContentBlock::Text {
                                    text: " ".to_string(),
                                }]
                            } else {
                                tool_result_content
                            };

                            let status = if is_error.unwrap_or(false) {
                                Some(ToolResultStatus::Error)
                            } else {
                                Some(ToolResultStatus::Success)
                            };

                            content_blocks.push(ContentBlock::ToolResult {
                                tool_result: ToolResultBlock {
                                    tool_use_id,
                                    content: final_content,
                                    status,
                                },
                            });
                        }
                        crate::apis::anthropic::MessagesContentBlock::Image { source } => {
                            // Convert Anthropic image to Bedrock image format
                            match source {
                                crate::apis::anthropic::MessagesImageSource::Base64 {
                                    media_type,
                                    data,
                                } => {
                                    content_blocks.push(ContentBlock::Image {
                                        image: ImageBlock {
                                            source: ImageSource::Base64 { media_type, data },
                                        },
                                    });
                                }
                                crate::apis::anthropic::MessagesImageSource::Url { .. } => {
                                    // Bedrock doesn't support URL-based images, skip for now
                                    // Could potentially download and convert to base64, but not implemented
                                }
                            }
                        }
                        // Skip other content types for now (Thinking, Document, etc.)
                        _ => {}
                    }
                }
            }
        }

        // Ensure we have at least one content block
        if content_blocks.is_empty() {
            content_blocks.push(ContentBlock::Text {
                text: " ".to_string(),
            });
        }

        Ok(BedrockMessage {
            role,
            content: content_blocks,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apis::amazon_bedrock::{
        ContentBlock, ConversationRole, ConverseRequest, SystemContentBlock,
        ToolChoice as BedrockToolChoice,
    };
    use crate::apis::anthropic::{
        MessagesMessage, MessagesMessageContent, MessagesRequest, MessagesRole,
        MessagesSystemPrompt, MessagesTool, MessagesToolChoice, MessagesToolChoiceType,
    };
    use serde_json::json;

    #[test]
    fn test_anthropic_to_bedrock_basic_request() {
        let anthropic_request = MessagesRequest {
            model: "claude-3-5-sonnet-20241022".to_string(),
            messages: vec![MessagesMessage {
                role: MessagesRole::User,
                content: MessagesMessageContent::Single("Hello, how are you?".to_string()),
            }],
            max_tokens: 1000,
            container: None,
            mcp_servers: None,
            system: Some(MessagesSystemPrompt::Single(
                "You are a helpful assistant.".to_string(),
            )),
            metadata: None,
            service_tier: None,
            thinking: None,
            temperature: Some(0.7),
            top_p: Some(0.9),
            top_k: None,
            stream: Some(false),
            stop_sequences: Some(vec!["STOP".to_string()]),
            tools: None,
            tool_choice: None,
        };

        let bedrock_request: ConverseRequest = anthropic_request.try_into().unwrap();

        assert_eq!(bedrock_request.model_id, "claude-3-5-sonnet-20241022");
        assert!(bedrock_request.system.is_some());
        assert_eq!(bedrock_request.system.as_ref().unwrap().len(), 1);
        assert!(bedrock_request.messages.is_some());
        let messages = bedrock_request.messages.as_ref().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, ConversationRole::User);

        if let ContentBlock::Text { text } = &messages[0].content[0] {
            assert_eq!(text, "Hello, how are you?");
        } else {
            panic!("Expected text content block");
        }

        let inference_config = bedrock_request.inference_config.as_ref().unwrap();
        assert_eq!(inference_config.temperature, Some(0.7));
        assert_eq!(inference_config.top_p, Some(0.9));
        assert_eq!(inference_config.max_tokens, Some(1000));
        assert_eq!(
            inference_config.stop_sequences,
            Some(vec!["STOP".to_string()])
        );
    }

    #[test]
    fn test_anthropic_to_bedrock_with_tools() {
        let anthropic_request = MessagesRequest {
            model: "claude-3-5-sonnet-20241022".to_string(),
            messages: vec![MessagesMessage {
                role: MessagesRole::User,
                content: MessagesMessageContent::Single("What's the weather like?".to_string()),
            }],
            max_tokens: 1000,
            container: None,
            mcp_servers: None,
            system: None,
            metadata: None,
            service_tier: None,
            thinking: None,
            temperature: None,
            top_p: None,
            top_k: None,
            stream: None,
            stop_sequences: None,
            tools: Some(vec![MessagesTool {
                name: "get_weather".to_string(),
                description: Some("Get current weather information".to_string()),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "location": {
                            "type": "string",
                            "description": "The city name"
                        }
                    },
                    "required": ["location"]
                }),
            }]),
            tool_choice: Some(MessagesToolChoice {
                kind: MessagesToolChoiceType::Tool,
                name: Some("get_weather".to_string()),
                disable_parallel_tool_use: None,
            }),
        };

        let bedrock_request: ConverseRequest = anthropic_request.try_into().unwrap();

        assert_eq!(bedrock_request.model_id, "claude-3-5-sonnet-20241022");
        assert!(bedrock_request.tool_config.is_some());

        let tool_config = bedrock_request.tool_config.as_ref().unwrap();
        assert!(tool_config.tools.is_some());
        let tools = tool_config.tools.as_ref().unwrap();
        assert_eq!(tools.len(), 1);
        let crate::apis::amazon_bedrock::Tool::ToolSpec { tool_spec } = &tools[0];
        assert_eq!(tool_spec.name, "get_weather");
        assert_eq!(
            tool_spec.description,
            Some("Get current weather information".to_string())
        );

        if let Some(BedrockToolChoice::Tool { tool }) = &tool_config.tool_choice {
            assert_eq!(tool.name, "get_weather");
        } else {
            panic!("Expected specific tool choice");
        }
    }

    #[test]
    fn test_anthropic_to_bedrock_auto_tool_choice() {
        let anthropic_request = MessagesRequest {
            model: "claude-3-5-sonnet-20241022".to_string(),
            messages: vec![MessagesMessage {
                role: MessagesRole::User,
                content: MessagesMessageContent::Single("Help me with something".to_string()),
            }],
            max_tokens: 500,
            container: None,
            mcp_servers: None,
            system: None,
            metadata: None,
            service_tier: None,
            thinking: None,
            temperature: None,
            top_p: None,
            top_k: None,
            stream: None,
            stop_sequences: None,
            tools: Some(vec![MessagesTool {
                name: "help_tool".to_string(),
                description: Some("A helpful tool".to_string()),
                input_schema: json!({
                    "type": "object",
                    "properties": {}
                }),
            }]),
            tool_choice: Some(MessagesToolChoice {
                kind: MessagesToolChoiceType::Auto,
                name: None,
                disable_parallel_tool_use: None,
            }),
        };

        let bedrock_request: ConverseRequest = anthropic_request.try_into().unwrap();

        assert!(bedrock_request.tool_config.is_some());
        let tool_config = bedrock_request.tool_config.as_ref().unwrap();
        assert!(matches!(
            tool_config.tool_choice,
            Some(BedrockToolChoice::Auto { .. })
        ));
    }

    #[test]
    fn test_anthropic_to_bedrock_multi_message_conversation() {
        let anthropic_request = MessagesRequest {
            model: "claude-3-5-sonnet-20241022".to_string(),
            messages: vec![
                MessagesMessage {
                    role: MessagesRole::User,
                    content: MessagesMessageContent::Single("Hello".to_string()),
                },
                MessagesMessage {
                    role: MessagesRole::Assistant,
                    content: MessagesMessageContent::Single(
                        "Hi there! How can I help you?".to_string(),
                    ),
                },
                MessagesMessage {
                    role: MessagesRole::User,
                    content: MessagesMessageContent::Single("What's 2+2?".to_string()),
                },
            ],
            max_tokens: 100,
            container: None,
            mcp_servers: None,
            system: Some(MessagesSystemPrompt::Single("Be concise".to_string())),
            metadata: None,
            service_tier: None,
            thinking: None,
            temperature: Some(0.5),
            top_p: None,
            top_k: None,
            stream: None,
            stop_sequences: None,
            tools: None,
            tool_choice: None,
        };

        let bedrock_request: ConverseRequest = anthropic_request.try_into().unwrap();

        assert!(bedrock_request.messages.is_some());
        let messages = bedrock_request.messages.as_ref().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, ConversationRole::User);
        assert_eq!(messages[1].role, ConversationRole::Assistant);
        assert_eq!(messages[2].role, ConversationRole::User);

        // Check system prompt
        assert!(bedrock_request.system.is_some());
        if let SystemContentBlock::Text { text } = &bedrock_request.system.as_ref().unwrap()[0] {
            assert_eq!(text, "Be concise");
        } else {
            panic!("Expected system text block");
        }
    }

    #[test]
    fn test_anthropic_message_to_bedrock_conversion() {
        let anthropic_message = MessagesMessage {
            role: MessagesRole::User,
            content: MessagesMessageContent::Single("Test message".to_string()),
        };

        let bedrock_message: BedrockMessage = anthropic_message.try_into().unwrap();

        assert_eq!(bedrock_message.role, ConversationRole::User);
        assert_eq!(bedrock_message.content.len(), 1);

        if let ContentBlock::Text { text } = &bedrock_message.content[0] {
            assert_eq!(text, "Test message");
        } else {
            panic!("Expected text content block");
        }
    }
}
