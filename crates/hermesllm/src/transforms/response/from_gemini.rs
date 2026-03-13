use crate::apis::anthropic::MessagesResponse;
use crate::apis::gemini::GenerateContentResponse;
use crate::apis::openai::{
    ChatCompletionsResponse, ChatCompletionsStreamResponse, Choice, FinishReason,
    FunctionCall as OpenAIFunctionCall, MessageDelta, ResponseMessage, Role, StreamChoice,
    ToolCall as OpenAIToolCall, Usage,
};
use crate::clients::TransformError;

// ============================================================================
// Gemini GenerateContentResponse -> OpenAI ChatCompletionsResponse
// ============================================================================

fn map_finish_reason(gemini_reason: Option<&str>) -> Option<FinishReason> {
    gemini_reason.map(|r| match r {
        "STOP" => FinishReason::Stop,
        "MAX_TOKENS" => FinishReason::Length,
        "SAFETY" | "RECITATION" => FinishReason::ContentFilter,
        _ => FinishReason::Stop,
    })
}

impl TryFrom<GenerateContentResponse> for ChatCompletionsResponse {
    type Error = TransformError;

    fn try_from(resp: GenerateContentResponse) -> Result<Self, Self::Error> {
        let candidates = resp.candidates.unwrap_or_default();
        let candidate = candidates.first();

        let mut content_text = String::new();
        let mut tool_calls: Vec<OpenAIToolCall> = Vec::new();

        if let Some(candidate) = candidate {
            if let Some(ref content) = candidate.content {
                for (i, part) in content.parts.iter().enumerate() {
                    if let Some(ref text) = part.text {
                        content_text.push_str(text);
                    }
                    if let Some(ref fc) = part.function_call {
                        tool_calls.push(OpenAIToolCall {
                            id: format!("call_{}", i),
                            call_type: "function".to_string(),
                            function: OpenAIFunctionCall {
                                name: fc.name.clone(),
                                arguments: serde_json::to_string(&fc.args).unwrap_or_default(),
                            },
                        });
                    }
                }
            }
        }

        let finish_reason = candidate
            .and_then(|c| map_finish_reason(c.finish_reason.as_deref()))
            .unwrap_or(FinishReason::Stop);

        let message_content = if content_text.is_empty() {
            None
        } else {
            Some(content_text)
        };

        let tool_calls_opt = if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        };

        let choice = Choice {
            index: 0,
            message: ResponseMessage {
                role: Role::Assistant,
                content: message_content,
                tool_calls: tool_calls_opt,
                refusal: None,
                annotations: None,
                audio: None,
                function_call: None,
            },
            finish_reason: Some(finish_reason),
            logprobs: None,
        };

        let usage = resp
            .usage_metadata
            .map(|um| Usage {
                prompt_tokens: um.prompt_token_count.unwrap_or(0),
                completion_tokens: um.candidates_token_count.unwrap_or(0),
                total_tokens: um.total_token_count.unwrap_or(0),
                prompt_tokens_details: None,
                completion_tokens_details: None,
            })
            .unwrap_or_default();

        Ok(ChatCompletionsResponse {
            id: format!(
                "chatcmpl-gemini-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
            ),
            object: Some("chat.completion".to_string()),
            created: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            model: resp.model_version.unwrap_or_else(|| "gemini".to_string()),
            choices: vec![choice],
            usage,
            system_fingerprint: None,
            service_tier: None,
            metadata: None,
        })
    }
}

// ============================================================================
// Gemini GenerateContentResponse -> Anthropic MessagesResponse (via OpenAI)
// ============================================================================

impl TryFrom<GenerateContentResponse> for MessagesResponse {
    type Error = TransformError;

    fn try_from(resp: GenerateContentResponse) -> Result<Self, Self::Error> {
        // Chain: Gemini -> OpenAI -> Anthropic
        let chat_resp = ChatCompletionsResponse::try_from(resp)?;
        MessagesResponse::try_from(chat_resp)
    }
}

// ============================================================================
// Gemini GenerateContentResponse -> OpenAI ChatCompletionsStreamResponse
// ============================================================================

impl TryFrom<GenerateContentResponse> for ChatCompletionsStreamResponse {
    type Error = TransformError;

    fn try_from(resp: GenerateContentResponse) -> Result<Self, Self::Error> {
        let candidates = resp.candidates.unwrap_or_default();
        let candidate = candidates.first();

        let mut delta_content: Option<String> = None;

        if let Some(candidate) = candidate {
            if let Some(ref content) = candidate.content {
                let mut text_parts = Vec::new();

                for part in content.parts.iter() {
                    if let Some(ref text) = part.text {
                        text_parts.push(text.clone());
                    }
                }

                if !text_parts.is_empty() {
                    delta_content = Some(text_parts.join(""));
                }
            }
        }

        let finish_reason = candidate.and_then(|c| map_finish_reason(c.finish_reason.as_deref()));

        let role = candidate
            .and_then(|c| c.content.as_ref())
            .and_then(|c| c.role.as_deref())
            .map(|r| match r {
                "model" => Role::Assistant,
                _ => Role::User,
            });

        Ok(ChatCompletionsStreamResponse {
            id: format!(
                "chatcmpl-gemini-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
            ),
            object: Some("chat.completion.chunk".to_string()),
            created: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            model: resp.model_version.unwrap_or_else(|| "gemini".to_string()),
            choices: vec![StreamChoice {
                index: 0,
                delta: MessageDelta {
                    role,
                    content: delta_content,
                    tool_calls: None,
                    refusal: None,
                    function_call: None,
                },
                finish_reason,
                logprobs: None,
            }],
            usage: None,
            system_fingerprint: None,
            service_tier: None,
        })
    }
}

// ============================================================================
// REVERSE: OpenAI ChatCompletionsResponse -> Gemini GenerateContentResponse
// ============================================================================

impl TryFrom<ChatCompletionsResponse> for GenerateContentResponse {
    type Error = TransformError;

    fn try_from(resp: ChatCompletionsResponse) -> Result<Self, Self::Error> {
        use crate::apis::gemini::{Candidate, Content, FunctionCall, Part, UsageMetadata};

        let candidates = if let Some(choice) = resp.choices.first() {
            let mut parts = Vec::new();

            // Text content
            if let Some(ref content) = choice.message.content {
                if !content.is_empty() {
                    parts.push(Part {
                        text: Some(content.clone()),
                        inline_data: None,
                        function_call: None,
                        function_response: None,
                    });
                }
            }

            // Tool calls
            if let Some(ref tool_calls) = choice.message.tool_calls {
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

            if parts.is_empty() {
                parts.push(Part {
                    text: Some(String::new()),
                    inline_data: None,
                    function_call: None,
                    function_response: None,
                });
            }

            let finish_reason = choice.finish_reason.as_ref().map(|fr| match fr {
                FinishReason::Stop => "STOP".to_string(),
                FinishReason::Length => "MAX_TOKENS".to_string(),
                FinishReason::ContentFilter => "SAFETY".to_string(),
                FinishReason::ToolCalls => "STOP".to_string(),
                FinishReason::FunctionCall => "STOP".to_string(),
            });

            vec![Candidate {
                content: Some(Content {
                    role: Some("model".to_string()),
                    parts,
                }),
                finish_reason,
                safety_ratings: None,
            }]
        } else {
            vec![]
        };

        let usage_metadata = Some(UsageMetadata {
            prompt_token_count: Some(resp.usage.prompt_tokens),
            candidates_token_count: Some(resp.usage.completion_tokens),
            total_token_count: Some(resp.usage.total_tokens),
        });

        Ok(GenerateContentResponse {
            candidates: Some(candidates),
            usage_metadata,
            prompt_feedback: None,
            model_version: Some(resp.model),
        })
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
    fn test_gemini_to_openai_response() {
        let resp: GenerateContentResponse = serde_json::from_value(json!({
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
            },
            "modelVersion": "gemini-2.0-flash"
        }))
        .unwrap();

        let openai_resp = ChatCompletionsResponse::try_from(resp).unwrap();
        assert_eq!(openai_resp.choices.len(), 1);
        let msg = &openai_resp.choices[0].message;
        assert_eq!(msg.content.as_deref(), Some("Hello! How can I help?"));
        assert_eq!(
            openai_resp.choices[0].finish_reason,
            Some(FinishReason::Stop)
        );
        assert_eq!(openai_resp.usage.prompt_tokens, 5);
        assert_eq!(openai_resp.usage.completion_tokens, 7);
    }

    #[test]
    fn test_gemini_to_openai_stream_response() {
        let resp: GenerateContentResponse = serde_json::from_value(json!({
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [{"text": "Hello"}]
                }
            }]
        }))
        .unwrap();

        let stream_resp = ChatCompletionsStreamResponse::try_from(resp).unwrap();
        assert_eq!(stream_resp.choices.len(), 1);
        assert_eq!(
            stream_resp.choices[0].delta.content,
            Some("Hello".to_string())
        );
        assert_eq!(stream_resp.choices[0].delta.role, Some(Role::Assistant));
    }

    #[test]
    fn test_gemini_to_openai_with_function_call() {
        let resp: GenerateContentResponse = serde_json::from_value(json!({
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
        }))
        .unwrap();

        let openai_resp = ChatCompletionsResponse::try_from(resp).unwrap();
        let msg = &openai_resp.choices[0].message;
        assert!(msg.tool_calls.is_some());
        let tc = msg.tool_calls.as_ref().unwrap();
        assert_eq!(tc[0].function.name, "get_weather");
    }

    #[test]
    fn test_openai_to_gemini_response() {
        let resp: ChatCompletionsResponse = serde_json::from_value(json!({
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "Hello!"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 5, "completion_tokens": 7, "total_tokens": 12}
        }))
        .unwrap();

        let gemini_resp = GenerateContentResponse::try_from(resp).unwrap();
        let candidates = gemini_resp.candidates.as_ref().unwrap();
        assert_eq!(candidates.len(), 1);
        let parts = &candidates[0].content.as_ref().unwrap().parts;
        assert_eq!(parts[0].text.as_deref(), Some("Hello!"));
        assert_eq!(candidates[0].finish_reason.as_deref(), Some("STOP"));
    }

    #[test]
    fn test_finish_reason_mapping() {
        assert_eq!(map_finish_reason(Some("STOP")), Some(FinishReason::Stop));
        assert_eq!(
            map_finish_reason(Some("MAX_TOKENS")),
            Some(FinishReason::Length)
        );
        assert_eq!(
            map_finish_reason(Some("SAFETY")),
            Some(FinishReason::ContentFilter)
        );
        assert_eq!(
            map_finish_reason(Some("RECITATION")),
            Some(FinishReason::ContentFilter)
        );
        assert_eq!(map_finish_reason(None), None);
    }
}
