use crate::apis::amazon_bedrock::{
    AnyChoice, AutoChoice, ContentBlock, ConversationRole, ConverseRequest, InferenceConfiguration,
    Message as BedrockMessage, SystemContentBlock, Tool as BedrockTool,
    ToolChoice as BedrockToolChoice, ToolChoiceSpec, ToolConfiguration, ToolInputSchema,
    ToolSpecDefinition,
};
use crate::apis::anthropic::{
    MessagesContentBlock, MessagesMessage, MessagesMessageContent, MessagesRequest, MessagesRole,
    MessagesSystemPrompt, MessagesTool, MessagesToolChoice, MessagesToolChoiceType,
    ToolResultContent,
};
use crate::apis::openai::{
    ChatCompletionsRequest, Message, MessageContent, Role, Tool, ToolChoice, ToolChoiceType,
};

use crate::apis::openai_responses::{
    InputContent, InputItem, InputParam, MessageRole, Modality, ReasoningEffort,
    ResponsesAPIRequest, Tool as ResponsesTool, ToolChoice as ResponsesToolChoice,
};
use crate::clients::TransformError;
use crate::transforms::lib::ExtractText;
use crate::transforms::lib::*;
use crate::transforms::*;

type AnthropicMessagesRequest = MessagesRequest;

// ============================================================================
// MAIN REQUEST TRANSFORMATIONS
// ============================================================================

impl From<Message> for MessagesSystemPrompt {
    fn from(val: Message) -> Self {
        let system_text = match val.content {
            MessageContent::Text(text) => text,
            MessageContent::Parts(parts) => parts.extract_text(),
        };
        MessagesSystemPrompt::Single(system_text)
    }
}

impl TryFrom<Message> for MessagesMessage {
    type Error = TransformError;

    fn try_from(message: Message) -> Result<Self, Self::Error> {
        let role = match message.role {
            Role::User => MessagesRole::User,
            Role::Assistant => MessagesRole::Assistant,
            Role::Tool => {
                // Tool messages become user messages with tool results
                let tool_call_id = message.tool_call_id.ok_or_else(|| {
                    TransformError::MissingField(
                        "tool_call_id required for Tool messages".to_string(),
                    )
                })?;

                return Ok(MessagesMessage {
                    role: MessagesRole::User,
                    content: MessagesMessageContent::Blocks(vec![
                        MessagesContentBlock::ToolResult {
                            tool_use_id: tool_call_id,
                            is_error: None,
                            content: ToolResultContent::Blocks(vec![MessagesContentBlock::Text {
                                text: message.content.extract_text(),
                                cache_control: None,
                            }]),
                            cache_control: None,
                        },
                    ]),
                });
            }
            Role::System => {
                return Err(TransformError::UnsupportedConversion(
                    "System messages should be handled separately".to_string(),
                ));
            }
        };

        let content_blocks = convert_openai_message_to_anthropic_content(&message)?;
        let content = build_anthropic_content(content_blocks);

        Ok(MessagesMessage { role, content })
    }
}

impl TryFrom<Message> for BedrockMessage {
    type Error = TransformError;

    fn try_from(message: Message) -> Result<Self, Self::Error> {
        let role = match message.role {
            Role::User => ConversationRole::User,
            Role::Assistant => ConversationRole::Assistant,
            Role::Tool => ConversationRole::User, // Tool results become user messages in Bedrock
            Role::System => {
                return Err(TransformError::UnsupportedConversion(
                    "System messages should be handled separately in Bedrock".to_string(),
                ));
            }
        };

        let mut content_blocks = Vec::new();

        // Handle different message types
        match message.role {
            Role::User => {
                // Convert user message content to content blocks
                match message.content {
                    MessageContent::Text(text) => {
                        if !text.is_empty() {
                            content_blocks.push(ContentBlock::Text { text });
                        }
                    }
                    MessageContent::Parts(parts) => {
                        // Convert OpenAI content parts to Bedrock ContentBlocks
                        for part in parts {
                            match part {
                                crate::apis::openai::ContentPart::Text { text } => {
                                    if !text.is_empty() {
                                        content_blocks.push(ContentBlock::Text { text });
                                    }
                                }
                                crate::apis::openai::ContentPart::ImageUrl { image_url } => {
                                    // Convert image URL to Bedrock image format
                                    if image_url.url.starts_with("data:") {
                                        if let Some((media_type, data)) =
                                            parse_data_url(&image_url.url)
                                        {
                                            content_blocks.push(ContentBlock::Image {
                                                image: crate::apis::amazon_bedrock::ImageBlock {
                                                    source: crate::apis::amazon_bedrock::ImageSource::Base64 {
                                                        media_type,
                                                        data,
                                                    },
                                                },
                                            });
                                        } else {
                                            return Err(TransformError::UnsupportedConversion(
                                                format!(
                                                    "Invalid data URL format: {}",
                                                    image_url.url
                                                ),
                                            ));
                                        }
                                    } else {
                                        return Err(TransformError::UnsupportedConversion(
                                            "Only base64 data URLs are supported for images in Bedrock".to_string()
                                        ));
                                    }
                                }
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
            }
            Role::Assistant => {
                // Handle text content - but only add if non-empty OR if we don't have tool calls
                let text_content = message.content.extract_text();
                let has_tool_calls = message
                    .tool_calls
                    .as_ref()
                    .is_some_and(|calls| !calls.is_empty());

                // Add text content if it's non-empty, or if we have no tool calls (to avoid empty content)
                if !text_content.is_empty() {
                    content_blocks.push(ContentBlock::Text { text: text_content });
                } else if !has_tool_calls {
                    // If we have empty content and no tool calls, add a minimal placeholder
                    // This prevents the "blank text field" error
                    content_blocks.push(ContentBlock::Text {
                        text: " ".to_string(),
                    });
                }

                // Convert tool calls to ToolUse content blocks
                if let Some(tool_calls) = message.tool_calls {
                    for tool_call in tool_calls {
                        // Parse the arguments string as JSON
                        let input: serde_json::Value =
                            serde_json::from_str(&tool_call.function.arguments).map_err(|e| {
                                TransformError::UnsupportedConversion(format!(
                                    "Failed to parse tool arguments as JSON: {}. Arguments: {}",
                                    e, tool_call.function.arguments
                                ))
                            })?;

                        content_blocks.push(ContentBlock::ToolUse {
                            tool_use: crate::apis::amazon_bedrock::ToolUseBlock {
                                tool_use_id: tool_call.id,
                                name: tool_call.function.name,
                                input,
                            },
                        });
                    }
                }

                // Bedrock requires at least one content block
                if content_blocks.is_empty() {
                    content_blocks.push(ContentBlock::Text {
                        text: " ".to_string(),
                    });
                }
            }
            Role::Tool => {
                // Tool messages become user messages with ToolResult content blocks
                let tool_call_id = message.tool_call_id.ok_or_else(|| {
                    TransformError::MissingField(
                        "tool_call_id required for Tool messages".to_string(),
                    )
                })?;

                let tool_content = message.content.extract_text();

                // Create ToolResult content block
                let tool_result_content = if tool_content.is_empty() {
                    // Even for tool results, we need non-empty content
                    vec![crate::apis::amazon_bedrock::ToolResultContentBlock::Text {
                        text: " ".to_string(),
                    }]
                } else {
                    vec![crate::apis::amazon_bedrock::ToolResultContentBlock::Text {
                        text: tool_content,
                    }]
                };

                content_blocks.push(ContentBlock::ToolResult {
                    tool_result: crate::apis::amazon_bedrock::ToolResultBlock {
                        tool_use_id: tool_call_id,
                        content: tool_result_content,
                        status: Some(crate::apis::amazon_bedrock::ToolResultStatus::Success), // Default to success
                    },
                });
            }
            Role::System => {
                // Already handled above with early return
                unreachable!()
            }
        }

        Ok(BedrockMessage {
            role,
            content: content_blocks,
        })
    }
}

impl TryFrom<ResponsesAPIRequest> for ChatCompletionsRequest {
    type Error = TransformError;

    fn try_from(req: ResponsesAPIRequest) -> Result<Self, Self::Error> {
        // Convert input to messages
        let messages = match req.input {
            InputParam::Text(text) => {
                // Simple text input becomes a user message
                vec![Message {
                    role: Role::User,
                    content: MessageContent::Text(text),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                }]
            }
            InputParam::Items(items) => {
                // Convert input items to messages
                let mut converted_messages = Vec::new();

                // Add instructions as system message if present
                if let Some(instructions) = &req.instructions {
                    converted_messages.push(Message {
                        role: Role::System,
                        content: MessageContent::Text(instructions.clone()),
                        name: None,
                        tool_call_id: None,
                        tool_calls: None,
                    });
                }

                // Convert each input item
                for item in items {
                    if let InputItem::Message(input_msg) = item {
                        let role = match input_msg.role {
                            MessageRole::User => Role::User,
                            MessageRole::Assistant => Role::Assistant,
                            MessageRole::System => Role::System,
                            MessageRole::Developer => Role::System, // Map developer to system
                        };

                        // Convert content based on MessageContent type
                        let content = match &input_msg.content {
                            crate::apis::openai_responses::MessageContent::Text(text) => {
                                // Simple text content
                                MessageContent::Text(text.clone())
                            }
                            crate::apis::openai_responses::MessageContent::Items(content_items) => {
                                // Check if it's a single text item (can use simple text format)
                                if content_items.len() == 1 {
                                    if let InputContent::InputText { text } = &content_items[0] {
                                        MessageContent::Text(text.clone())
                                    } else {
                                        // Single non-text item - use parts format
                                        MessageContent::Parts(
                                            content_items.iter()
                                                .filter_map(|c| match c {
                                                    InputContent::InputText { text } => {
                                                        Some(crate::apis::openai::ContentPart::Text { text: text.clone() })
                                                    }
                                                    InputContent::InputImage { image_url, .. } => {
                                                        Some(crate::apis::openai::ContentPart::ImageUrl {
                                                            image_url: crate::apis::openai::ImageUrl {
                                                                url: image_url.clone(),
                                                                detail: None,
                                                            }
                                                        })
                                                    }
                                                    InputContent::InputFile { .. } => None, // Skip files for now
                                                    InputContent::InputAudio { .. } => None, // Skip audio for now
                                                })
                                                .collect()
                                        )
                                    }
                                } else {
                                    // Multiple content items - convert to parts
                                    MessageContent::Parts(
                                        content_items
                                            .iter()
                                            .filter_map(|c| match c {
                                                InputContent::InputText { text } => {
                                                    Some(crate::apis::openai::ContentPart::Text {
                                                        text: text.clone(),
                                                    })
                                                }
                                                InputContent::InputImage { image_url, .. } => Some(
                                                    crate::apis::openai::ContentPart::ImageUrl {
                                                        image_url: crate::apis::openai::ImageUrl {
                                                            url: image_url.clone(),
                                                            detail: None,
                                                        },
                                                    },
                                                ),
                                                InputContent::InputFile { .. } => None, // Skip files for now
                                                InputContent::InputAudio { .. } => None, // Skip audio for now
                                            })
                                            .collect(),
                                    )
                                }
                            }
                        };

                        converted_messages.push(Message {
                            role,
                            content,
                            name: None,
                            tool_call_id: None,
                            tool_calls: None,
                        });
                    }
                }

                converted_messages
            }
        };

        // Build the ChatCompletionsRequest
        Ok(ChatCompletionsRequest {
            model: req.model,
            messages,
            temperature: req.temperature,
            top_p: req.top_p,
            max_completion_tokens: req.max_output_tokens.map(|t| t as u32),
            stream: req.stream,
            metadata: req.metadata,
            user: req.user,
            store: req.store,
            service_tier: req.service_tier,
            top_logprobs: req.top_logprobs.map(|t| t as u32),
            modalities: req.modalities.map(|mods| {
                mods.into_iter().map(|m| {
                    match m {
                        Modality::Text => "text".to_string(),
                        Modality::Audio => "audio".to_string(),
                    }
                }).collect()
            }),
            stream_options: req.stream_options.map(|opts| {
                crate::apis::openai::StreamOptions {
                    include_usage: opts.include_usage,
                }
            }),
            reasoning_effort: req.reasoning_effort.map(|effort| {
                match effort {
                    ReasoningEffort::Low => "low".to_string(),
                    ReasoningEffort::Medium => "medium".to_string(),
                    ReasoningEffort::High => "high".to_string(),
                }
            }),
            tools: req.tools.map(|tools| {
                tools.into_iter().map(|tool| {

                    // Only convert Function tools - other types are not supported in ChatCompletions
                    match tool {
                        ResponsesTool::Function { name, description, parameters, strict } => Ok(Tool {
                            tool_type: "function".to_string(),
                            function: crate::apis::openai::Function {
                                name,
                                description,
                                parameters: parameters.unwrap_or_else(|| serde_json::json!({
                                    "type": "object",
                                    "properties": {}
                                })),
                                strict,
                            }
                        }),
                        ResponsesTool::FileSearch { .. } => Err(TransformError::UnsupportedConversion(
                            "FileSearch tool is not supported in ChatCompletions API. Only function tools are supported.".to_string()
                        )),
                        ResponsesTool::WebSearchPreview { .. } => Err(TransformError::UnsupportedConversion(
                            "WebSearchPreview tool is not supported in ChatCompletions API. Only function tools are supported.".to_string()
                        )),
                        ResponsesTool::CodeInterpreter => Err(TransformError::UnsupportedConversion(
                            "CodeInterpreter tool is not supported in ChatCompletions API. Only function tools are supported.".to_string()
                        )),
                        ResponsesTool::Computer { .. } => Err(TransformError::UnsupportedConversion(
                            "Computer tool is not supported in ChatCompletions API. Only function tools are supported.".to_string()
                        )),
                    }
                }).collect::<Result<Vec<_>, _>>()
            }).transpose()?,
            tool_choice: req.tool_choice.map(|choice| {
                match choice {
                    ResponsesToolChoice::String(s) => {
                        match s.as_str() {
                            "auto" => ToolChoice::Type(ToolChoiceType::Auto),
                            "required" => ToolChoice::Type(ToolChoiceType::Required),
                            "none" => ToolChoice::Type(ToolChoiceType::None),
                            _ => ToolChoice::Type(ToolChoiceType::Auto), // Default to auto for unknown strings
                        }
                    }
                    ResponsesToolChoice::Named { function, .. } => ToolChoice::Function {
                        choice_type: "function".to_string(),
                        function: crate::apis::openai::FunctionChoice { name: function.name }
                    }
                }
            }),
            parallel_tool_calls: req.parallel_tool_calls,
            ..Default::default()
        })
    }
}

impl TryFrom<ChatCompletionsRequest> for AnthropicMessagesRequest {
    type Error = TransformError;

    fn try_from(req: ChatCompletionsRequest) -> Result<Self, Self::Error> {
        let mut system_prompt = None;
        let mut messages = Vec::new();

        for message in req.messages {
            match message.role {
                Role::System => {
                    system_prompt = Some(message.into());
                }
                _ => {
                    let anthropic_message: MessagesMessage = message.try_into()?;
                    messages.push(anthropic_message);
                }
            }
        }

        // Convert tools and tool choice
        let anthropic_tools = req.tools.map(convert_openai_tools);
        let anthropic_tool_choice =
            convert_openai_tool_choice(req.tool_choice, req.parallel_tool_calls);

        Ok(AnthropicMessagesRequest {
            model: req.model,
            system: system_prompt,
            messages,
            max_tokens: req
                .max_completion_tokens
                .or(req.max_tokens)
                .unwrap_or(DEFAULT_MAX_TOKENS),
            container: None,
            mcp_servers: None,
            service_tier: None,
            thinking: None,
            temperature: req.temperature,
            top_p: req.top_p,
            top_k: None, // OpenAI doesn't have top_k
            stream: req.stream,
            stop_sequences: req.stop,
            tools: anthropic_tools,
            tool_choice: anthropic_tool_choice,
            metadata: None,
        })
    }
}

impl TryFrom<ChatCompletionsRequest> for ConverseRequest {
    type Error = TransformError;

    fn try_from(req: ChatCompletionsRequest) -> Result<Self, Self::Error> {
        // Separate system messages from user/assistant messages
        let mut system_messages = Vec::new();
        let mut conversation_messages = Vec::new();

        for message in req.messages {
            match message.role {
                Role::System => {
                    let system_text = match message.content {
                        MessageContent::Text(text) => text,
                        MessageContent::Parts(parts) => parts.extract_text(),
                    };
                    system_messages.push(SystemContentBlock::Text { text: system_text });
                }
                _ => {
                    let bedrock_message: BedrockMessage = message.try_into()?;
                    conversation_messages.push(bedrock_message);
                }
            }
        }

        // Convert system messages
        let system = if system_messages.is_empty() {
            None
        } else {
            Some(system_messages)
        };

        // Convert conversation messages
        let messages = if conversation_messages.is_empty() {
            None
        } else {
            Some(conversation_messages)
        };

        // Build inference configuration
        let max_tokens = req.max_completion_tokens.or(req.max_tokens);
        let inference_config = if max_tokens.is_some()
            || req.temperature.is_some()
            || req.top_p.is_some()
            || req.stop.is_some()
        {
            Some(InferenceConfiguration {
                max_tokens,
                temperature: req.temperature,
                top_p: req.top_p,
                stop_sequences: req.stop,
            })
        } else {
            None
        };

        // Convert tools and tool choice to ToolConfiguration
        let tool_config = if req.tools.is_some() || req.tool_choice.is_some() {
            let tools = req.tools.map(|openai_tools| {
                openai_tools
                    .into_iter()
                    .map(|tool| BedrockTool::ToolSpec {
                        tool_spec: ToolSpecDefinition {
                            name: tool.function.name,
                            description: tool.function.description,
                            input_schema: ToolInputSchema {
                                json: tool.function.parameters,
                            },
                        },
                    })
                    .collect()
            });

            let tool_choice = req
                .tool_choice
                .map(|choice| {
                    match choice {
                        ToolChoice::Type(tool_type) => match tool_type {
                            ToolChoiceType::Auto => BedrockToolChoice::Auto {
                                auto: AutoChoice {},
                            },
                            ToolChoiceType::Required => {
                                BedrockToolChoice::Any { any: AnyChoice {} }
                            }
                            ToolChoiceType::None => BedrockToolChoice::Auto {
                                auto: AutoChoice {},
                            }, // Bedrock doesn't have explicit "none"
                        },
                        ToolChoice::Function { function, .. } => BedrockToolChoice::Tool {
                            tool: ToolChoiceSpec {
                                name: function.name,
                            },
                        },
                    }
                })
                .or_else(|| {
                    // If tools are present but no tool_choice specified, default to "auto"
                    if tools.is_some() {
                        Some(BedrockToolChoice::Auto {
                            auto: AutoChoice {},
                        })
                    } else {
                        None
                    }
                });

            Some(ToolConfiguration { tools, tool_choice })
        } else {
            None
        };

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

/// Convert OpenAI tools to Anthropic format
fn convert_openai_tools(tools: Vec<Tool>) -> Vec<MessagesTool> {
    tools
        .into_iter()
        .map(|tool| MessagesTool {
            name: tool.function.name,
            description: tool.function.description,
            input_schema: tool.function.parameters,
        })
        .collect()
}

/// Convert OpenAI tool choice to Anthropic format
fn convert_openai_tool_choice(
    tool_choice: Option<ToolChoice>,
    parallel_tool_calls: Option<bool>,
) -> Option<MessagesToolChoice> {
    tool_choice.map(|choice| match choice {
        ToolChoice::Type(tool_type) => match tool_type {
            ToolChoiceType::Auto => MessagesToolChoice {
                kind: MessagesToolChoiceType::Auto,
                name: None,
                disable_parallel_tool_use: parallel_tool_calls.map(|p| !p),
            },
            ToolChoiceType::Required => MessagesToolChoice {
                kind: MessagesToolChoiceType::Any,
                name: None,
                disable_parallel_tool_use: parallel_tool_calls.map(|p| !p),
            },
            ToolChoiceType::None => MessagesToolChoice {
                kind: MessagesToolChoiceType::None,
                name: None,
                disable_parallel_tool_use: None,
            },
        },
        ToolChoice::Function { function, .. } => MessagesToolChoice {
            kind: MessagesToolChoiceType::Tool,
            name: Some(function.name),
            disable_parallel_tool_use: parallel_tool_calls.map(|p| !p),
        },
    })
}

/// Build Anthropic message content from content blocks
fn build_anthropic_content(content_blocks: Vec<MessagesContentBlock>) -> MessagesMessageContent {
    if content_blocks.len() == 1 {
        match &content_blocks[0] {
            MessagesContentBlock::Text { text, .. } => MessagesMessageContent::Single(text.clone()),
            _ => MessagesMessageContent::Blocks(content_blocks),
        }
    } else if content_blocks.is_empty() {
        MessagesMessageContent::Single("".to_string())
    } else {
        MessagesMessageContent::Blocks(content_blocks)
    }
}

/// Parse a data URL into media type and base64 data
/// Supports format: data:image/jpeg;base64,<data>
fn parse_data_url(url: &str) -> Option<(String, String)> {
    if !url.starts_with("data:") {
        return None;
    }

    let without_prefix = &url[5..]; // Remove "data:" prefix
    let parts: Vec<&str> = without_prefix.splitn(2, ',').collect();

    if parts.len() != 2 {
        return None;
    }

    let header = parts[0];
    let data = parts[1];

    // Parse header: "image/jpeg;base64" or just "image/jpeg"
    let header_parts: Vec<&str> = header.split(';').collect();
    if header_parts.is_empty() {
        return None;
    }

    let media_type = header_parts[0].to_string();

    // Check if it's base64 encoded
    if header_parts.len() > 1 && header_parts[1] == "base64" {
        Some((media_type, data.to_string()))
    } else {
        // For now, only support base64 encoding
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apis::amazon_bedrock::{
        ContentBlock, ConversationRole, ConverseRequest, SystemContentBlock,
        ToolChoice as BedrockToolChoice,
    };
    use crate::apis::openai::{
        ChatCompletionsRequest, Function, FunctionChoice, Message, MessageContent, Role, Tool,
        ToolChoice, ToolChoiceType,
    };
    use serde_json::json;

    #[test]
    fn test_openai_to_bedrock_basic_request() {
        let openai_request = ChatCompletionsRequest {
            model: "gpt-4".to_string(),
            messages: vec![
                Message {
                    role: Role::System,
                    content: MessageContent::Text("You are a helpful assistant.".to_string()),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                },
                Message {
                    role: Role::User,
                    content: MessageContent::Text("Hello, how are you?".to_string()),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                },
            ],
            temperature: Some(0.7),
            top_p: Some(0.9),
            max_completion_tokens: Some(1000),
            stop: Some(vec!["STOP".to_string()]),
            stream: Some(false),
            tools: None,
            tool_choice: None,
            ..Default::default()
        };

        let bedrock_request: ConverseRequest = openai_request.try_into().unwrap();

        assert_eq!(bedrock_request.model_id, "gpt-4");
        assert!(bedrock_request.system.is_some());
        assert_eq!(bedrock_request.system.as_ref().unwrap().len(), 1);

        if let SystemContentBlock::Text { text } = &bedrock_request.system.as_ref().unwrap()[0] {
            assert_eq!(text, "You are a helpful assistant.");
        } else {
            panic!("Expected system text block");
        }

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
    fn test_openai_to_bedrock_with_tools() {
        let openai_request = ChatCompletionsRequest {
            model: "gpt-4".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Text("What's the weather like?".to_string()),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            }],
            temperature: None,
            top_p: None,
            max_completion_tokens: Some(1000),
            stop: None,
            stream: None,
            tools: Some(vec![Tool {
                tool_type: "function".to_string(),
                function: Function {
                    name: "get_weather".to_string(),
                    description: Some("Get current weather information".to_string()),
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "location": {
                                "type": "string",
                                "description": "The city name"
                            }
                        },
                        "required": ["location"]
                    }),
                    strict: None,
                },
            }]),
            tool_choice: Some(ToolChoice::Function {
                choice_type: "function".to_string(),
                function: FunctionChoice {
                    name: "get_weather".to_string(),
                },
            }),
            ..Default::default()
        };

        let bedrock_request: ConverseRequest = openai_request.try_into().unwrap();

        assert_eq!(bedrock_request.model_id, "gpt-4");
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
    fn test_openai_to_bedrock_auto_tool_choice() {
        let openai_request = ChatCompletionsRequest {
            model: "gpt-4".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Text("Help me with something".to_string()),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            }],
            temperature: None,
            top_p: None,
            max_completion_tokens: Some(500),
            stop: None,
            stream: None,
            tools: Some(vec![Tool {
                tool_type: "function".to_string(),
                function: Function {
                    name: "help_tool".to_string(),
                    description: Some("A helpful tool".to_string()),
                    parameters: json!({
                        "type": "object",
                        "properties": {}
                    }),
                    strict: None,
                },
            }]),
            tool_choice: Some(ToolChoice::Type(ToolChoiceType::Auto)),
            ..Default::default()
        };

        let bedrock_request: ConverseRequest = openai_request.try_into().unwrap();

        assert!(bedrock_request.tool_config.is_some());
        let tool_config = bedrock_request.tool_config.as_ref().unwrap();
        assert!(matches!(
            tool_config.tool_choice,
            Some(BedrockToolChoice::Auto { .. })
        ));
    }

    #[test]
    fn test_openai_to_bedrock_multi_message_conversation() {
        let openai_request = ChatCompletionsRequest {
            model: "gpt-4".to_string(),
            messages: vec![
                Message {
                    role: Role::System,
                    content: MessageContent::Text("Be concise".to_string()),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                },
                Message {
                    role: Role::User,
                    content: MessageContent::Text("Hello".to_string()),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                },
                Message {
                    role: Role::Assistant,
                    content: MessageContent::Text("Hi there! How can I help you?".to_string()),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                },
                Message {
                    role: Role::User,
                    content: MessageContent::Text("What's 2+2?".to_string()),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                },
            ],
            temperature: Some(0.5),
            top_p: None,
            max_completion_tokens: Some(100),
            stop: None,
            stream: None,
            tools: None,
            tool_choice: None,
            ..Default::default()
        };

        let bedrock_request: ConverseRequest = openai_request.try_into().unwrap();

        assert!(bedrock_request.messages.is_some());
        let messages = bedrock_request.messages.as_ref().unwrap();
        assert_eq!(messages.len(), 3); // System message is separate
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
    fn test_openai_message_to_bedrock_conversion() {
        let openai_message = Message {
            role: Role::User,
            content: MessageContent::Text("Test message".to_string()),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        };

        let bedrock_message: BedrockMessage = openai_message.try_into().unwrap();

        assert_eq!(bedrock_message.role, ConversationRole::User);
        assert_eq!(bedrock_message.content.len(), 1);

        if let ContentBlock::Text { text } = &bedrock_message.content[0] {
            assert_eq!(text, "Test message");
        } else {
            panic!("Expected text content block");
        }
    }
}
