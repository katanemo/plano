use crate::apis::openai_responses::ResponsesAPIRequest;
use crate::providers::id::ProviderId;
use crate::providers::request::{ProviderRequest, ProviderRequestError, ProviderRequestType};

/// Serialize a provider request for the upstream wire format.
///
/// For most providers this is plain `to_bytes()`. ChatGPT's native /responses
/// backend has wire-format quirks that require post-serialization patching:
///   - `max_output_tokens` must be sent as `maxTokens` for GPT-5.4-era models,
///     but omitted for GPT-5.5, which rejects `maxTokens`
///   - `truncation` must be omitted; ChatGPT Codex rejects it
///   - Structured content arrays (`input_text`/`output_text` typed parts)
///     must be flattened to plain text strings
pub fn serialize_for_upstream(
    request: &ProviderRequestType,
    provider_id: ProviderId,
) -> Result<Vec<u8>, ProviderRequestError> {
    match (provider_id, request) {
        (ProviderId::ChatGPT, ProviderRequestType::ResponsesAPIRequest(req)) => {
            adapt_chatgpt_responses_request(req)
        }
        _ => request.to_bytes(),
    }
}

/// Apply ChatGPT-specific wire-format fixes to a ResponsesAPI request.
///
/// Works at the JSON value level so we can rename keys and restructure
/// content without needing separate serde types for the ChatGPT variant.
fn adapt_chatgpt_responses_request(
    req: &ResponsesAPIRequest,
) -> Result<Vec<u8>, ProviderRequestError> {
    let mut value = serde_json::to_value(req).map_err(|e| ProviderRequestError {
        message: format!(
            "Failed to encode ChatGPT responses request as JSON value: {}",
            e
        ),
        source: Some(Box::new(e)),
    })?;

    if let Some(obj) = value.as_object_mut() {
        let is_gpt_55 = obj
            .get("model")
            .and_then(|v| v.as_str())
            .map(|model| model == "gpt-5.5" || model.starts_with("gpt-5.5-"))
            .unwrap_or(false);

        // ChatGPT rejects `max_output_tokens`. GPT-5.4-era Codex expects
        // `maxTokens`, but GPT-5.5 rejects `maxTokens` too, so omit it there.
        if let Some(max_output_tokens) = obj.remove("max_output_tokens") {
            if !is_gpt_55 && !max_output_tokens.is_null() {
                obj.insert("maxTokens".to_string(), max_output_tokens);
            }
        }

        // ChatGPT Codex rejects this OpenAI Responses field.
        obj.remove("truncation");

        // ChatGPT rejects structured content arrays with typed parts
        // (input_text, output_text); flatten them to plain text strings
        flatten_input_content_parts(obj);

        // ChatGPT does not persist output item references when store=false.
        // OpenClaw uses store=false, so replayed hidden reasoning references
        // must be dropped instead of sent back as `type=reasoning,id=rs_*`.
        // The visible assistant/user transcript remains in the request.
        remove_unstored_reasoning_input_refs(obj);

        // ChatGPT requires remaining reasoning input items to carry a summary
        // array. This covers stored conversations where reasoning refs are valid.
        ensure_reasoning_input_summaries(obj);
    }

    serde_json::to_vec(&value).map_err(|e| ProviderRequestError {
        message: format!(
            "Failed to serialize ChatGPT responses request for upstream: {}",
            e
        ),
        source: Some(Box::new(e)),
    })
}

/// Walk through `input[].content` and collapse typed content-part arrays
/// into plain text strings that ChatGPT accepts.
fn flatten_input_content_parts(obj: &mut serde_json::Map<String, serde_json::Value>) {
    let input = match obj.get_mut("input").and_then(|v| v.as_array_mut()) {
        Some(arr) => arr,
        None => return,
    };

    for item in input {
        let content = match item.as_object_mut().and_then(|m| m.get_mut("content")) {
            Some(c) => c,
            None => continue,
        };

        let parts = match content.as_array() {
            Some(p) => p,
            None => continue,
        };

        let mut saw_text_part = false;
        let text = parts
            .iter()
            .filter_map(|part| {
                let part_obj = part.as_object()?;
                match part_obj.get("type").and_then(|v| v.as_str()) {
                    Some("input_text") | Some("output_text") => {
                        saw_text_part = true;
                        Some(
                            part_obj
                                .get("text")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                        )
                    }
                    _ => None,
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        // Even when all text parts are empty, we still need to collapse the array.
        // Leaving typed parts in-place causes ChatGPT Codex endpoints to reject them.
        if saw_text_part {
            *content = serde_json::Value::String(text);
        }
    }
}

fn remove_unstored_reasoning_input_refs(obj: &mut serde_json::Map<String, serde_json::Value>) {
    if obj.get("store").and_then(|v| v.as_bool()) != Some(false) {
        return;
    }

    let input = match obj.get_mut("input").and_then(|v| v.as_array_mut()) {
        Some(arr) => arr,
        None => return,
    };

    input.retain(|item| {
        let Some(item) = item.as_object() else {
            return true;
        };

        !(item.get("type").and_then(|v| v.as_str()) == Some("reasoning")
            && item.get("id").and_then(|v| v.as_str()).is_some())
    });
}

fn ensure_reasoning_input_summaries(obj: &mut serde_json::Map<String, serde_json::Value>) {
    let input = match obj.get_mut("input").and_then(|v| v.as_array_mut()) {
        Some(arr) => arr,
        None => return,
    };

    for item in input {
        let item = match item.as_object_mut() {
            Some(item) => item,
            None => continue,
        };

        if item.get("type").and_then(|v| v.as_str()) == Some("reasoning")
            && !item.contains_key("summary")
        {
            item.insert("summary".to_string(), serde_json::Value::Array(Vec::new()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apis::openai::OpenAIApi;
    use crate::apis::openai_responses::{
        InputContent, InputItem, InputMessage, InputParam, MessageContent, MessageRole,
        ResponsesAPIRequest,
    };

    fn make_responses_request(
        input: InputParam,
        max_output_tokens: Option<i32>,
    ) -> ResponsesAPIRequest {
        ResponsesAPIRequest {
            model: "gpt-5.4".to_string(),
            input,
            temperature: None,
            max_output_tokens,
            stream: Some(true),
            metadata: None,
            tools: None,
            tool_choice: None,
            parallel_tool_calls: None,
            instructions: Some("You are Codex.".to_string()),
            modalities: None,
            user: None,
            store: Some(false),
            reasoning_effort: None,
            include: None,
            audio: None,
            text: None,
            service_tier: None,
            top_p: None,
            top_logprobs: None,
            stream_options: None,
            truncation: None,
            conversation: None,
            previous_response_id: None,
            max_tool_calls: None,
            background: None,
        }
    }

    // ---------------------------------------------------------------
    // max_output_tokens → maxTokens rename
    // ---------------------------------------------------------------

    #[test]
    fn chatgpt_renames_max_output_tokens_to_max_tokens_on_wire() {
        let req = make_responses_request(InputParam::Text("Hello".to_string()), Some(8192));
        let request = ProviderRequestType::ResponsesAPIRequest(req);

        let bytes = serialize_for_upstream(&request, ProviderId::ChatGPT).unwrap();
        let wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert!(
            wire.get("max_output_tokens").is_none(),
            "max_output_tokens should be absent from wire format"
        );
        assert_eq!(
            wire.get("maxTokens").and_then(|v| v.as_i64()),
            Some(8192),
            "maxTokens should be present with the original value"
        );
    }

    #[test]
    fn chatgpt_omits_max_tokens_when_max_output_tokens_is_none() {
        let req = make_responses_request(InputParam::Text("Hello".to_string()), None);
        let request = ProviderRequestType::ResponsesAPIRequest(req);

        let bytes = serialize_for_upstream(&request, ProviderId::ChatGPT).unwrap();
        let wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert!(wire.get("max_output_tokens").is_none());
        assert!(
            wire.get("maxTokens").is_none(),
            "maxTokens should not appear when original was None"
        );
    }

    #[test]
    fn chatgpt_gpt55_omits_max_tokens_on_wire() {
        let mut req = make_responses_request(InputParam::Text("Hello".to_string()), Some(8192));
        req.model = "gpt-5.5".to_string();
        let request = ProviderRequestType::ResponsesAPIRequest(req);

        let bytes = serialize_for_upstream(&request, ProviderId::ChatGPT).unwrap();
        let wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert!(wire.get("max_output_tokens").is_none());
        assert!(
            wire.get("maxTokens").is_none(),
            "GPT-5.5 ChatGPT Codex rejects maxTokens, so it must be omitted"
        );
    }

    #[test]
    fn chatgpt_omits_truncation_on_wire() {
        let mut req = make_responses_request(InputParam::Text("Hello".to_string()), None);
        req.truncation = Some("disabled".to_string());
        let request = ProviderRequestType::ResponsesAPIRequest(req);

        let bytes = serialize_for_upstream(&request, ProviderId::ChatGPT).unwrap();
        let wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert!(
            wire.get("truncation").is_none(),
            "ChatGPT Codex rejects truncation, so it must be omitted"
        );
    }

    #[test]
    fn non_chatgpt_preserves_max_output_tokens_field_name() {
        let req = make_responses_request(InputParam::Text("Hello".to_string()), Some(4096));
        let request = ProviderRequestType::ResponsesAPIRequest(req);

        let bytes = serialize_for_upstream(&request, ProviderId::OpenAI).unwrap();
        let wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(
            wire.get("max_output_tokens").and_then(|v| v.as_i64()),
            Some(4096)
        );
        assert!(wire.get("maxTokens").is_none());
    }

    // ---------------------------------------------------------------
    // input_text / output_text content flattening
    // ---------------------------------------------------------------

    #[test]
    fn chatgpt_flattens_input_text_content_parts_to_plain_string() {
        let input = InputParam::Items(vec![InputItem::Message(InputMessage {
            role: MessageRole::User,
            content: MessageContent::Items(vec![
                InputContent::InputText {
                    text: "first line".to_string(),
                },
                InputContent::InputText {
                    text: "second line".to_string(),
                },
            ]),
        })]);

        let req = make_responses_request(input, None);
        let request = ProviderRequestType::ResponsesAPIRequest(req);

        let bytes = serialize_for_upstream(&request, ProviderId::ChatGPT).unwrap();
        let wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        let content = &wire["input"][0]["content"];
        assert!(
            content.is_string(),
            "content should be flattened to a string, got: {}",
            content
        );
        assert_eq!(content.as_str().unwrap(), "first line\nsecond line");
    }

    #[test]
    fn chatgpt_flattens_output_text_content_parts() {
        let input = InputParam::Items(vec![InputItem::Message(InputMessage {
            role: MessageRole::Assistant,
            content: MessageContent::Items(vec![InputContent::InputText {
                text: "assistant reply".to_string(),
            }]),
        })]);

        let req = make_responses_request(input, None);
        let request = ProviderRequestType::ResponsesAPIRequest(req);

        let bytes = serialize_for_upstream(&request, ProviderId::ChatGPT).unwrap();
        let wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        let content = &wire["input"][0]["content"];
        assert!(content.is_string());
        assert_eq!(content.as_str().unwrap(), "assistant reply");
    }

    #[test]
    fn chatgpt_flattens_empty_input_text_content_parts() {
        let input = InputParam::Items(vec![InputItem::Message(InputMessage {
            role: MessageRole::Assistant,
            content: MessageContent::Items(vec![InputContent::InputText {
                text: "".to_string(),
            }]),
        })]);

        let req = make_responses_request(input, None);
        let request = ProviderRequestType::ResponsesAPIRequest(req);

        let bytes = serialize_for_upstream(&request, ProviderId::ChatGPT).unwrap();
        let wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        let content = &wire["input"][0]["content"];
        assert!(
            content.is_string(),
            "content should be flattened to a string, got: {}",
            content
        );
        assert_eq!(content.as_str().unwrap(), "");
    }

    #[test]
    fn chatgpt_preserves_plain_text_content_unchanged() {
        let input = InputParam::Items(vec![InputItem::Message(InputMessage {
            role: MessageRole::User,
            content: MessageContent::Text("plain text message".to_string()),
        })]);

        let req = make_responses_request(input, None);
        let request = ProviderRequestType::ResponsesAPIRequest(req);

        let bytes = serialize_for_upstream(&request, ProviderId::ChatGPT).unwrap();
        let wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        let content = &wire["input"][0]["content"];
        assert_eq!(content.as_str().unwrap(), "plain text message");
    }

    #[test]
    fn non_chatgpt_does_not_flatten_content_parts() {
        let input = InputParam::Items(vec![InputItem::Message(InputMessage {
            role: MessageRole::User,
            content: MessageContent::Items(vec![
                InputContent::InputText {
                    text: "part one".to_string(),
                },
                InputContent::InputText {
                    text: "part two".to_string(),
                },
            ]),
        })]);

        let req = make_responses_request(input, None);
        let request = ProviderRequestType::ResponsesAPIRequest(req);

        let bytes = serialize_for_upstream(&request, ProviderId::OpenAI).unwrap();
        let wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        let content = &wire["input"][0]["content"];
        assert!(
            content.is_array(),
            "OpenAI should preserve array content, got: {}",
            content
        );
    }

    // ---------------------------------------------------------------
    // Reasoning item compatibility
    // ---------------------------------------------------------------

    #[test]
    fn chatgpt_adds_empty_summary_to_stored_reasoning_input_items() {
        let req: ResponsesAPIRequest = serde_json::from_value(serde_json::json!({
            "model": "gpt-5.5",
            "input": [
                {"type": "reasoning", "id": "rs_123"},
                {"role": "user", "content": "Are you there?"}
            ],
            "store": true,
            "stream": true
        }))
        .unwrap();
        let request = ProviderRequestType::ResponsesAPIRequest(req);

        let bytes = serialize_for_upstream(&request, ProviderId::ChatGPT).unwrap();
        let wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(
            wire["input"][0]["summary"],
            serde_json::Value::Array(Vec::new()),
            "GPT-5.5 ChatGPT rejects reasoning input items without summary"
        );
    }

    #[test]
    fn chatgpt_drops_reasoning_item_references_when_store_false() {
        let req: ResponsesAPIRequest = serde_json::from_value(serde_json::json!({
            "model": "gpt-5.5",
            "input": [
                {"type": "reasoning", "id": "rs_123"},
                {"role": "user", "content": "Are you there?"}
            ],
            "store": false,
            "stream": true
        }))
        .unwrap();
        let request = ProviderRequestType::ResponsesAPIRequest(req);

        let bytes = serialize_for_upstream(&request, ProviderId::ChatGPT).unwrap();
        let wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        let input = wire["input"].as_array().unwrap();
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["content"], "Are you there?");
    }

    #[test]
    fn chatgpt_preserves_function_call_fields_with_id() {
        let req: ResponsesAPIRequest = serde_json::from_value(serde_json::json!({
            "model": "gpt-5.5",
            "input": [
                {
                    "type": "function_call",
                    "id": "fc_123",
                    "call_id": "call_123",
                    "name": "exec",
                    "arguments": "{}"
                },
                {"type": "function_call_output", "call_id": "call_123", "output": "ok"},
                {"role": "user", "content": "continue"}
            ],
            "store": false,
            "stream": true
        }))
        .unwrap();
        let request = ProviderRequestType::ResponsesAPIRequest(req);

        let bytes = serialize_for_upstream(&request, ProviderId::ChatGPT).unwrap();
        let wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(wire["input"][0]["type"], "function_call");
        assert_eq!(wire["input"][0]["id"], "fc_123");
        assert_eq!(wire["input"][0]["call_id"], "call_123");
        assert_eq!(wire["input"][0]["name"], "exec");
        assert_eq!(wire["input"][0]["arguments"], "{}");
    }

    #[test]
    fn non_chatgpt_preserves_reasoning_item_reference_without_summary() {
        let req: ResponsesAPIRequest = serde_json::from_value(serde_json::json!({
            "model": "gpt-5.5",
            "input": [
                {"type": "reasoning", "id": "rs_123"},
                {"role": "user", "content": "Are you there?"}
            ],
            "stream": true
        }))
        .unwrap();
        let request = ProviderRequestType::ResponsesAPIRequest(req);

        let bytes = serialize_for_upstream(&request, ProviderId::OpenAI).unwrap();
        let wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert!(wire["input"][0].get("summary").is_none());
    }

    // ---------------------------------------------------------------
    // Both fixes together (realistic ChatGPT payload)
    // ---------------------------------------------------------------

    #[test]
    fn chatgpt_applies_both_fixes_together() {
        let input = InputParam::Items(vec![
            InputItem::Message(InputMessage {
                role: MessageRole::User,
                content: MessageContent::Items(vec![InputContent::InputText {
                    text: "Write a function".to_string(),
                }]),
            }),
            InputItem::Message(InputMessage {
                role: MessageRole::Assistant,
                content: MessageContent::Items(vec![InputContent::InputText {
                    text: "def hello(): pass".to_string(),
                }]),
            }),
            InputItem::Message(InputMessage {
                role: MessageRole::User,
                content: MessageContent::Items(vec![InputContent::InputText {
                    text: "Add a docstring".to_string(),
                }]),
            }),
        ]);

        let req = make_responses_request(input, Some(16384));
        let request = ProviderRequestType::ResponsesAPIRequest(req);

        let bytes = serialize_for_upstream(&request, ProviderId::ChatGPT).unwrap();
        let wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        // max_output_tokens renamed
        assert!(wire.get("max_output_tokens").is_none());
        assert_eq!(wire.get("maxTokens").and_then(|v| v.as_i64()), Some(16384));

        // All content arrays flattened
        for (i, item) in wire["input"].as_array().unwrap().iter().enumerate() {
            let content = &item["content"];
            assert!(
                content.is_string(),
                "input[{}].content should be a string, got: {}",
                i,
                content
            );
        }
    }

    // ---------------------------------------------------------------
    // Non-ResponsesAPI requests pass through unchanged
    // ---------------------------------------------------------------

    #[test]
    fn chatgpt_chat_completions_request_passes_through() {
        use crate::apis::openai::{ChatCompletionsRequest, Message, MessageContent as MC, Role};

        let chat_req = ChatCompletionsRequest {
            model: "gpt-5.4".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: Some(MC::Text("Hello".to_string())),
                name: None,
                tool_calls: None,
                tool_call_id: None,
            }],
            max_completion_tokens: Some(1024),
            ..Default::default()
        };
        let request = ProviderRequestType::ChatCompletionsRequest(chat_req);

        let bytes = serialize_for_upstream(&request, ProviderId::ChatGPT).unwrap();
        let wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(
            wire.get("max_completion_tokens").and_then(|v| v.as_i64()),
            Some(1024)
        );
    }

    // ---------------------------------------------------------------
    // Normalize + serialize round-trip (full pipeline test)
    // ---------------------------------------------------------------

    #[test]
    fn chatgpt_full_pipeline_normalize_then_serialize() {
        let input = InputParam::Text("Hello, Codex!".to_string());
        let req = make_responses_request(input, Some(8192));

        let upstream_api = crate::clients::endpoints::SupportedUpstreamAPIs::OpenAIResponsesAPI(
            OpenAIApi::Responses,
        );
        let mut request = ProviderRequestType::ResponsesAPIRequest(req);

        // normalize_for_upstream sets store=false, stream=true, wraps input in Items
        request
            .normalize_for_upstream(ProviderId::ChatGPT, &upstream_api)
            .expect("ChatGPT responses request should normalize");

        // serialize_for_upstream then renames max_output_tokens and flattens content
        let bytes = serialize_for_upstream(&request, ProviderId::ChatGPT).unwrap();
        let wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert!(wire.get("max_output_tokens").is_none());
        assert_eq!(wire.get("maxTokens").and_then(|v| v.as_i64()), Some(8192));
        assert_eq!(wire.get("store"), Some(&serde_json::Value::Bool(false)));
        assert_eq!(wire.get("stream"), Some(&serde_json::Value::Bool(true)));
        assert!(
            wire["input"].is_array(),
            "input should be an array after normalize"
        );
    }
}
