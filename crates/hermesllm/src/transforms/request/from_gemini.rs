use crate::apis::gemini::GenerateContentRequest;
use crate::apis::openai::{
    ChatCompletionsRequest, Function, FunctionCall as OpenAIFunctionCall, Message, MessageContent,
    Role, Tool, ToolCall as OpenAIToolCall, ToolChoice, ToolChoiceType,
};

use crate::apis::anthropic::MessagesRequest;
use crate::clients::TransformError;

// ============================================================================
// Gemini GenerateContent -> OpenAI ChatCompletions
// ============================================================================

impl TryFrom<GenerateContentRequest> for ChatCompletionsRequest {
    type Error = TransformError;

    fn try_from(req: GenerateContentRequest) -> Result<Self, Self::Error> {
        let mut messages: Vec<Message> = Vec::new();

        // Convert system instruction
        if let Some(system) = &req.system_instruction {
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
        for content in &req.contents {
            let role = match content.role.as_deref() {
                Some("model") => Role::Assistant,
                _ => Role::User,
            };

            // Check if this content has function_call parts (assistant with tool calls)
            let has_function_calls = content.parts.iter().any(|p| p.function_call.is_some());
            let has_function_responses =
                content.parts.iter().any(|p| p.function_response.is_some());

            if has_function_calls {
                // Convert to assistant message with tool_calls
                let mut tool_calls = Vec::new();
                let mut text_parts = Vec::new();

                for (i, part) in content.parts.iter().enumerate() {
                    if let Some(fc) = &part.function_call {
                        tool_calls.push(OpenAIToolCall {
                            id: format!("call_{}", i),
                            call_type: "function".to_string(),
                            function: OpenAIFunctionCall {
                                name: fc.name.clone(),
                                arguments: serde_json::to_string(&fc.args).unwrap_or_default(),
                            },
                        });
                    } else if let Some(text) = &part.text {
                        text_parts.push(text.clone());
                    }
                }

                let content_text = if text_parts.is_empty() {
                    None
                } else {
                    Some(MessageContent::Text(text_parts.join("")))
                };

                messages.push(Message {
                    role: Role::Assistant,
                    content: content_text,
                    name: None,
                    tool_calls: if tool_calls.is_empty() {
                        None
                    } else {
                        Some(tool_calls)
                    },
                    tool_call_id: None,
                });
            } else if has_function_responses {
                // Convert each function_response to a tool message
                for part in &content.parts {
                    if let Some(fr) = &part.function_response {
                        let result_text = serde_json::to_string(&fr.response).unwrap_or_default();
                        messages.push(Message {
                            role: Role::Tool,
                            content: Some(MessageContent::Text(result_text)),
                            name: None,
                            tool_calls: None,
                            tool_call_id: Some(fr.name.clone()),
                        });
                    }
                }
            } else {
                // Regular text message
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
        }

        // Convert generation config
        let (temperature, top_p, max_tokens, stop, presence_penalty, frequency_penalty) =
            if let Some(gc) = &req.generation_config {
                (
                    gc.temperature,
                    gc.top_p,
                    gc.max_output_tokens,
                    gc.stop_sequences.clone(),
                    gc.presence_penalty,
                    gc.frequency_penalty,
                )
            } else {
                (None, None, None, None, None, None)
            };

        // Convert tools
        let tools = req.tools.and_then(|gemini_tools| {
            let openai_tools: Vec<Tool> = gemini_tools
                .iter()
                .filter_map(|t| t.function_declarations.as_ref())
                .flatten()
                .map(|fd| Tool {
                    tool_type: "function".to_string(),
                    function: Function {
                        name: fd.name.clone(),
                        description: fd.description.clone(),
                        parameters: fd.parameters.clone().unwrap_or_default(),
                        strict: None,
                    },
                })
                .collect();
            if openai_tools.is_empty() {
                None
            } else {
                Some(openai_tools)
            }
        });

        // Convert tool_config
        let tool_choice =
            req.tool_config
                .and_then(|tc| match tc.function_calling_config.mode.as_str() {
                    "AUTO" => Some(ToolChoice::Type(ToolChoiceType::Auto)),
                    "NONE" => Some(ToolChoice::Type(ToolChoiceType::None)),
                    "ANY" => Some(ToolChoice::Type(ToolChoiceType::Required)),
                    _ => None,
                });

        Ok(ChatCompletionsRequest {
            model: req.model,
            messages,
            temperature,
            top_p,
            max_completion_tokens: max_tokens,
            stop,
            tools,
            tool_choice,
            presence_penalty,
            frequency_penalty,
            metadata: req.metadata,
            ..Default::default()
        })
    }
}

// ============================================================================
// Gemini GenerateContent -> Anthropic Messages (via OpenAI)
// ============================================================================

impl TryFrom<GenerateContentRequest> for MessagesRequest {
    type Error = TransformError;

    fn try_from(req: GenerateContentRequest) -> Result<Self, Self::Error> {
        // Chain: Gemini -> OpenAI -> Anthropic
        let chat_req = ChatCompletionsRequest::try_from(req)?;
        MessagesRequest::try_from(chat_req)
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apis::gemini::{Content, FunctionCall, Part};
    use serde_json::json;

    #[test]
    fn test_gemini_to_openai_basic() {
        let req = GenerateContentRequest {
            model: "gemini-pro".to_string(),
            contents: vec![
                Content {
                    role: Some("user".to_string()),
                    parts: vec![Part {
                        text: Some("Hello".to_string()),
                        inline_data: None,
                        function_call: None,
                        function_response: None,
                    }],
                },
                Content {
                    role: Some("model".to_string()),
                    parts: vec![Part {
                        text: Some("Hi there!".to_string()),
                        inline_data: None,
                        function_call: None,
                        function_response: None,
                    }],
                },
            ],
            system_instruction: Some(Content {
                role: Some("user".to_string()),
                parts: vec![Part {
                    text: Some("Be helpful".to_string()),
                    inline_data: None,
                    function_call: None,
                    function_response: None,
                }],
            }),
            generation_config: Some(crate::apis::gemini::GenerationConfig {
                temperature: Some(0.5),
                max_output_tokens: Some(512),
                ..Default::default()
            }),
            ..Default::default()
        };

        let openai_req = ChatCompletionsRequest::try_from(req).unwrap();

        // System + user + assistant = 3 messages
        assert_eq!(openai_req.messages.len(), 3);
        assert_eq!(openai_req.messages[0].role, Role::System);
        assert_eq!(openai_req.messages[1].role, Role::User);
        assert_eq!(openai_req.messages[2].role, Role::Assistant);

        assert_eq!(openai_req.temperature, Some(0.5));
        assert_eq!(openai_req.max_completion_tokens, Some(512));
    }

    #[test]
    fn test_gemini_to_openai_with_function_calls() {
        let req = GenerateContentRequest {
            model: "gemini-pro".to_string(),
            contents: vec![
                Content {
                    role: Some("user".to_string()),
                    parts: vec![Part {
                        text: Some("Weather?".to_string()),
                        inline_data: None,
                        function_call: None,
                        function_response: None,
                    }],
                },
                Content {
                    role: Some("model".to_string()),
                    parts: vec![Part {
                        text: None,
                        inline_data: None,
                        function_call: Some(FunctionCall {
                            name: "get_weather".to_string(),
                            args: json!({"location": "NYC"}),
                        }),
                        function_response: None,
                    }],
                },
            ],
            ..Default::default()
        };

        let openai_req = ChatCompletionsRequest::try_from(req).unwrap();
        assert_eq!(openai_req.messages.len(), 2);
        assert!(openai_req.messages[1].tool_calls.is_some());
        let tc = openai_req.messages[1].tool_calls.as_ref().unwrap();
        assert_eq!(tc[0].function.name, "get_weather");
    }

    #[test]
    fn test_gemini_to_openai_tool_config() {
        let req = GenerateContentRequest {
            model: "gemini-pro".to_string(),
            contents: vec![Content {
                role: Some("user".to_string()),
                parts: vec![Part {
                    text: Some("test".to_string()),
                    inline_data: None,
                    function_call: None,
                    function_response: None,
                }],
            }],
            tool_config: Some(crate::apis::gemini::ToolConfig {
                function_calling_config: crate::apis::gemini::FunctionCallingConfig {
                    mode: "ANY".to_string(),
                },
            }),
            ..Default::default()
        };

        let openai_req = ChatCompletionsRequest::try_from(req).unwrap();
        assert!(openai_req.tool_choice.is_some());
        assert_eq!(
            openai_req.tool_choice.as_ref().unwrap(),
            &ToolChoice::Type(ToolChoiceType::Required)
        );
    }
}
