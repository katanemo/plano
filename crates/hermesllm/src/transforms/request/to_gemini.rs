use crate::apis::gemini::{
    Content, FunctionCall, FunctionCallingConfig, FunctionDeclaration, FunctionResponse,
    GenerateContentRequest, GenerationConfig, Part, Tool, ToolConfig,
};
use crate::apis::openai::{ChatCompletionsRequest, Role, ToolChoice, ToolChoiceType};

use crate::apis::anthropic::MessagesRequest;
use crate::clients::TransformError;
use crate::transforms::lib::ExtractText;

// ============================================================================
// OpenAI ChatCompletions -> Gemini GenerateContent
// ============================================================================

impl TryFrom<ChatCompletionsRequest> for GenerateContentRequest {
    type Error = TransformError;

    fn try_from(req: ChatCompletionsRequest) -> Result<Self, Self::Error> {
        let mut contents: Vec<Content> = Vec::new();
        let mut system_instruction: Option<Content> = None;

        for msg in &req.messages {
            match msg.role {
                Role::System => {
                    let text = msg.content.extract_text();
                    system_instruction = Some(Content {
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
                    let text = msg.content.extract_text();
                    contents.push(Content {
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
                    let mut parts = Vec::new();

                    // Check for tool calls
                    if let Some(tool_calls) = &msg.tool_calls {
                        for tc in tool_calls {
                            let args: serde_json::Value =
                                serde_json::from_str(&tc.function.arguments).unwrap_or_default();
                            parts.push(Part {
                                text: None,
                                inline_data: None,
                                function_call: Some(FunctionCall {
                                    name: tc.function.name.clone(),
                                    args,
                                }),
                                function_response: None,
                            });
                        }
                    }

                    // Also include text content if present
                    let text = msg.content.extract_text();
                    if !text.is_empty() {
                        parts.push(Part {
                            text: Some(text),
                            inline_data: None,
                            function_call: None,
                            function_response: None,
                        });
                    }

                    if !parts.is_empty() {
                        contents.push(Content {
                            role: Some("model".to_string()),
                            parts,
                        });
                    }
                }
                Role::Tool => {
                    let text = msg.content.extract_text();
                    let tool_call_id = msg.tool_call_id.clone().unwrap_or_default();
                    let response_value = serde_json::from_str(&text)
                        .unwrap_or_else(|_| serde_json::json!({"result": text}));

                    contents.push(Content {
                        role: Some("user".to_string()),
                        parts: vec![Part {
                            text: None,
                            inline_data: None,
                            function_call: None,
                            function_response: Some(FunctionResponse {
                                name: tool_call_id,
                                response: response_value,
                            }),
                        }],
                    });
                }
            }
        }

        // Convert generation config
        let generation_config = {
            let gc = GenerationConfig {
                temperature: req.temperature,
                top_p: req.top_p,
                top_k: None,
                max_output_tokens: req.max_completion_tokens.or(req.max_tokens),
                stop_sequences: req.stop,
                response_mime_type: None,
                candidate_count: None,
                presence_penalty: req.presence_penalty,
                frequency_penalty: req.frequency_penalty,
            };
            // Only include if any field is set
            if gc.temperature.is_some()
                || gc.top_p.is_some()
                || gc.max_output_tokens.is_some()
                || gc.stop_sequences.is_some()
                || gc.presence_penalty.is_some()
                || gc.frequency_penalty.is_some()
            {
                Some(gc)
            } else {
                None
            }
        };

        // Convert tools
        let tools = req.tools.map(|openai_tools| {
            let declarations: Vec<FunctionDeclaration> = openai_tools
                .iter()
                .map(|t| FunctionDeclaration {
                    name: t.function.name.clone(),
                    description: t.function.description.clone(),
                    parameters: Some(t.function.parameters.clone()),
                })
                .collect();
            vec![Tool {
                function_declarations: Some(declarations),
                code_execution: None,
            }]
        });

        // Convert tool_choice
        let tool_config = req.tool_choice.and_then(|tc| {
            let mode = match tc {
                ToolChoice::Type(t) => match t {
                    ToolChoiceType::Auto => Some("AUTO".to_string()),
                    ToolChoiceType::None => Some("NONE".to_string()),
                    ToolChoiceType::Required => Some("ANY".to_string()),
                },
                ToolChoice::Function { .. } => Some("AUTO".to_string()),
            };
            mode.map(|m| ToolConfig {
                function_calling_config: FunctionCallingConfig { mode: m },
            })
        });

        Ok(GenerateContentRequest {
            model: req.model,
            contents,
            generation_config,
            tools,
            tool_config,
            safety_settings: None,
            system_instruction,
            cached_content: None,
            metadata: req.metadata,
        })
    }
}

// ============================================================================
// Anthropic Messages -> Gemini GenerateContent (via OpenAI)
// ============================================================================

impl TryFrom<MessagesRequest> for GenerateContentRequest {
    type Error = TransformError;

    fn try_from(req: MessagesRequest) -> Result<Self, Self::Error> {
        // Chain: Anthropic -> OpenAI -> Gemini
        let chat_req = ChatCompletionsRequest::try_from(req)?;
        GenerateContentRequest::try_from(chat_req)
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
    fn test_openai_to_gemini_basic() {
        let req: ChatCompletionsRequest = serde_json::from_value(json!({
            "model": "gemini-pro",
            "messages": [
                {"role": "system", "content": "You are helpful"},
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": "Hi there!"},
                {"role": "user", "content": "How are you?"}
            ],
            "temperature": 0.7,
            "max_tokens": 1024
        }))
        .unwrap();

        let gemini_req = GenerateContentRequest::try_from(req).unwrap();

        // System should be in system_instruction
        assert!(gemini_req.system_instruction.is_some());
        let sys = gemini_req.system_instruction.as_ref().unwrap();
        assert_eq!(sys.parts[0].text.as_deref(), Some("You are helpful"));

        // 3 content messages (user, model, user)
        assert_eq!(gemini_req.contents.len(), 3);
        assert_eq!(gemini_req.contents[0].role.as_deref(), Some("user"));
        assert_eq!(gemini_req.contents[1].role.as_deref(), Some("model"));
        assert_eq!(gemini_req.contents[2].role.as_deref(), Some("user"));

        // Generation config
        assert_eq!(
            gemini_req.generation_config.as_ref().unwrap().temperature,
            Some(0.7)
        );
        assert_eq!(
            gemini_req
                .generation_config
                .as_ref()
                .unwrap()
                .max_output_tokens,
            Some(1024)
        );
    }

    #[test]
    fn test_openai_to_gemini_with_tools() {
        let req: ChatCompletionsRequest = serde_json::from_value(json!({
            "model": "gemini-pro",
            "messages": [
                {"role": "user", "content": "What's the weather?"}
            ],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "description": "Get weather",
                    "parameters": {"type": "object", "properties": {"location": {"type": "string"}}}
                }
            }],
            "tool_choice": "auto"
        }))
        .unwrap();

        let gemini_req = GenerateContentRequest::try_from(req).unwrap();
        assert!(gemini_req.tools.is_some());
        let tools = gemini_req.tools.as_ref().unwrap();
        assert_eq!(tools.len(), 1);
        let decls = tools[0].function_declarations.as_ref().unwrap();
        assert_eq!(decls[0].name, "get_weather");

        assert!(gemini_req.tool_config.is_some());
        assert_eq!(
            gemini_req
                .tool_config
                .as_ref()
                .unwrap()
                .function_calling_config
                .mode,
            "AUTO"
        );
    }

    #[test]
    fn test_openai_to_gemini_with_tool_calls() {
        let req: ChatCompletionsRequest = serde_json::from_value(json!({
            "model": "gemini-pro",
            "messages": [
                {"role": "user", "content": "What's the weather?"},
                {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call_123",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"location\": \"NYC\"}"
                        }
                    }]
                },
                {
                    "role": "tool",
                    "tool_call_id": "call_123",
                    "content": "Sunny, 72F"
                }
            ]
        }))
        .unwrap();

        let gemini_req = GenerateContentRequest::try_from(req).unwrap();
        assert_eq!(gemini_req.contents.len(), 3);

        // Assistant with function_call
        let model_content = &gemini_req.contents[1];
        assert_eq!(model_content.role.as_deref(), Some("model"));
        assert!(model_content.parts[0].function_call.is_some());

        // Tool response
        let tool_content = &gemini_req.contents[2];
        assert_eq!(tool_content.role.as_deref(), Some("user"));
        assert!(tool_content.parts[0].function_response.is_some());
    }
}
