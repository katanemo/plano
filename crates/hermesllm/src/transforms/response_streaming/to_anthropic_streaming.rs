use crate::apis::amazon_bedrock::{ContentBlockDelta, ConverseStreamEvent};
use crate::apis::anthropic::{
    MessagesContentBlock, MessagesContentDelta, MessagesMessageDelta, MessagesRole,
    MessagesStopReason, MessagesStreamEvent, MessagesStreamMessage, MessagesUsage,
};
use crate::apis::openai::{ChatCompletionsStreamResponse, ToolCallDelta};
use crate::clients::TransformError;
use serde_json::Value;

impl TryFrom<ChatCompletionsStreamResponse> for MessagesStreamEvent {
    type Error = TransformError;

    fn try_from(resp: ChatCompletionsStreamResponse) -> Result<Self, Self::Error> {
        if resp.choices.is_empty() {
            return Ok(MessagesStreamEvent::Ping);
        }

        let choice = &resp.choices[0];

        // Handle final chunk with usage
        let has_usage = resp.usage.is_some();
        if let Some(usage) = resp.usage {
            if let Some(finish_reason) = &choice.finish_reason {
                let anthropic_stop_reason: MessagesStopReason = finish_reason.clone().into();
                return Ok(MessagesStreamEvent::MessageDelta {
                    delta: MessagesMessageDelta {
                        stop_reason: anthropic_stop_reason,
                        stop_sequence: None,
                    },
                    usage: usage.into(),
                });
            }
        }

        // NOTE: We do NOT emit MessageStart here anymore!
        // The AnthropicMessagesStreamBuffer will inject message_start and content_block_start
        // when it sees the first content_block_delta. This solves the problem where OpenAI
        // sends both role and content in the same chunk - we can only return one event here,
        // so we prioritize the content and let the buffer handle lifecycle events.

        // Handle content delta (even if role is present in the same chunk)
        if let Some(content) = &choice.delta.content {
            if !content.is_empty() {
                return Ok(MessagesStreamEvent::ContentBlockDelta {
                    index: 0,
                    delta: MessagesContentDelta::TextDelta {
                        text: content.clone(),
                    },
                });
            }
        }

        // Handle tool calls
        if let Some(tool_calls) = &choice.delta.tool_calls {
            return convert_tool_call_deltas(tool_calls.clone());
        }

        // Handle finish reason - generate MessageDelta only (MessageStop comes later)
        if let Some(finish_reason) = &choice.finish_reason {
            // If we have usage data, it was already handled above
            // If not, we need to generate MessageDelta with default usage
            if !has_usage {
                let anthropic_stop_reason: MessagesStopReason = finish_reason.clone().into();
                return Ok(MessagesStreamEvent::MessageDelta {
                    delta: MessagesMessageDelta {
                        stop_reason: anthropic_stop_reason,
                        stop_sequence: None,
                    },
                    usage: MessagesUsage {
                        input_tokens: 0,
                        output_tokens: 0,
                        cache_creation_input_tokens: None,
                        cache_read_input_tokens: None,
                    },
                });
            }
            // If usage was already handled above, we don't need to do anything more here
            // MessageStop will be handled when [DONE] is encountered
        }

        // Default to ping for unhandled cases
        Ok(MessagesStreamEvent::Ping)
    }
}

impl From<MessagesStreamEvent> for String {
    fn from(val: MessagesStreamEvent) -> Self {
        let transformed_json = serde_json::to_string(&val).unwrap_or_default();
        let event_type = match &val {
            MessagesStreamEvent::MessageStart { .. } => "message_start",
            MessagesStreamEvent::ContentBlockStart { .. } => "content_block_start",
            MessagesStreamEvent::ContentBlockDelta { .. } => "content_block_delta",
            MessagesStreamEvent::ContentBlockStop { .. } => "content_block_stop",
            MessagesStreamEvent::MessageDelta { .. } => "message_delta",
            MessagesStreamEvent::MessageStop => "message_stop",
            MessagesStreamEvent::Ping => "ping",
        };

        let event = format!("event: {}\n", event_type);
        let data = format!("data: {}\n\n", transformed_json);
        event + &data
    }
}

impl TryFrom<ConverseStreamEvent> for MessagesStreamEvent {
    type Error = TransformError;

    fn try_from(event: ConverseStreamEvent) -> Result<Self, Self::Error> {
        match event {
            // MessageStart - convert to Anthropic MessageStart
            ConverseStreamEvent::MessageStart(start_event) => {
                let role = match start_event.role {
                    crate::apis::amazon_bedrock::ConversationRole::User => MessagesRole::User,
                    crate::apis::amazon_bedrock::ConversationRole::Assistant => {
                        MessagesRole::Assistant
                    }
                };

                Ok(MessagesStreamEvent::MessageStart {
                    message: MessagesStreamMessage {
                        id: format!(
                            "bedrock-stream-{}",
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_nanos()
                        ),
                        obj_type: "message".to_string(),
                        role,
                        content: vec![],
                        model: "bedrock-model".to_string(),
                        stop_reason: None,
                        stop_sequence: None,
                        usage: MessagesUsage {
                            input_tokens: 0,
                            output_tokens: 0,
                            cache_creation_input_tokens: None,
                            cache_read_input_tokens: None,
                        },
                    },
                })
            }

            // ContentBlockStart - convert to Anthropic ContentBlockStart
            ConverseStreamEvent::ContentBlockStart(start_event) => {
                // Note: Bedrock sends tool_use_id and name at start, with input coming in subsequent deltas
                // Anthropic expects the same pattern, so we initialize with an empty input object
                match start_event.start {
                    crate::apis::amazon_bedrock::ContentBlockStart::ToolUse { tool_use } => {
                        Ok(MessagesStreamEvent::ContentBlockStart {
                            index: start_event.content_block_index as u32,
                            content_block: MessagesContentBlock::ToolUse {
                                id: tool_use.tool_use_id,
                                name: tool_use.name,
                                input: Value::Object(serde_json::Map::new()), // Empty - will be filled by deltas
                                cache_control: None,
                            },
                        })
                    }
                }
            }

            // ContentBlockDelta - convert to Anthropic ContentBlockDelta
            ConverseStreamEvent::ContentBlockDelta(delta_event) => {
                let delta = match delta_event.delta {
                    ContentBlockDelta::Text { text } => MessagesContentDelta::TextDelta { text },
                    ContentBlockDelta::ToolUse { tool_use } => {
                        MessagesContentDelta::InputJsonDelta {
                            partial_json: tool_use.input,
                        }
                    }
                };

                Ok(MessagesStreamEvent::ContentBlockDelta {
                    index: delta_event.content_block_index as u32,
                    delta,
                })
            }

            // ContentBlockStop - convert to Anthropic ContentBlockStop
            ConverseStreamEvent::ContentBlockStop(stop_event) => {
                Ok(MessagesStreamEvent::ContentBlockStop {
                    index: stop_event.content_block_index as u32,
                })
            }

            // MessageStop - convert to Anthropic MessageDelta with stop reason
            // Note: Bedrock sends Metadata separately with usage info, creating a second MessageDelta
            // The client should merge these or use the final one with complete usage
            ConverseStreamEvent::MessageStop(stop_event) => {
                let anthropic_stop_reason = match stop_event.stop_reason {
                    crate::apis::amazon_bedrock::StopReason::EndTurn => MessagesStopReason::EndTurn,
                    crate::apis::amazon_bedrock::StopReason::ToolUse => MessagesStopReason::ToolUse,
                    crate::apis::amazon_bedrock::StopReason::MaxTokens => {
                        MessagesStopReason::MaxTokens
                    }
                    crate::apis::amazon_bedrock::StopReason::StopSequence => {
                        MessagesStopReason::EndTurn
                    }
                    crate::apis::amazon_bedrock::StopReason::GuardrailIntervened => {
                        MessagesStopReason::Refusal
                    }
                    crate::apis::amazon_bedrock::StopReason::ContentFiltered => {
                        MessagesStopReason::Refusal
                    }
                };

                Ok(MessagesStreamEvent::MessageDelta {
                    delta: MessagesMessageDelta {
                        stop_reason: anthropic_stop_reason,
                        stop_sequence: None,
                    },
                    usage: MessagesUsage {
                        input_tokens: 0,
                        output_tokens: 0,
                        cache_creation_input_tokens: None,
                        cache_read_input_tokens: None,
                    },
                })
            }

            // Metadata - convert usage information to MessageDelta
            ConverseStreamEvent::Metadata(metadata_event) => {
                Ok(MessagesStreamEvent::MessageDelta {
                    delta: MessagesMessageDelta {
                        stop_reason: MessagesStopReason::EndTurn,
                        stop_sequence: None,
                    },
                    usage: MessagesUsage {
                        input_tokens: metadata_event.usage.input_tokens,
                        output_tokens: metadata_event.usage.output_tokens,
                        cache_creation_input_tokens: metadata_event.usage.cache_write_input_tokens,
                        cache_read_input_tokens: metadata_event.usage.cache_read_input_tokens,
                    },
                })
            }

            // Exception events - convert to Ping (could be enhanced to return error events)
            ConverseStreamEvent::InternalServerException(_)
            | ConverseStreamEvent::ModelStreamErrorException(_)
            | ConverseStreamEvent::ServiceUnavailableException(_)
            | ConverseStreamEvent::ThrottlingException(_)
            | ConverseStreamEvent::ValidationException(_) => {
                // TODO: Consider adding proper error handling/events
                Ok(MessagesStreamEvent::Ping)
            }
        }
    }
}

/// Convert tool call deltas to Anthropic stream events
fn convert_tool_call_deltas(
    tool_calls: Vec<ToolCallDelta>,
) -> Result<MessagesStreamEvent, TransformError> {
    for tool_call in tool_calls {
        if let Some(id) = &tool_call.id {
            // Tool call start
            if let Some(function) = &tool_call.function {
                if let Some(name) = &function.name {
                    return Ok(MessagesStreamEvent::ContentBlockStart {
                        index: tool_call.index,
                        content_block: MessagesContentBlock::ToolUse {
                            id: id.clone(),
                            name: name.clone(),
                            input: Value::Object(serde_json::Map::new()),
                            cache_control: None,
                        },
                    });
                }
            }
        } else if let Some(function) = &tool_call.function {
            if let Some(arguments) = &function.arguments {
                // Tool arguments delta
                return Ok(MessagesStreamEvent::ContentBlockDelta {
                    index: tool_call.index,
                    delta: MessagesContentDelta::InputJsonDelta {
                        partial_json: arguments.clone(),
                    },
                });
            }
        }
    }

    // Fallback to ping if no valid tool call found
    Ok(MessagesStreamEvent::Ping)
}
