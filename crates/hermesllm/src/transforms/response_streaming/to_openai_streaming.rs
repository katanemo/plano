use crate::apis::amazon_bedrock::{ConverseStreamEvent, StopReason};
use crate::apis::anthropic::{
    MessagesContentBlock, MessagesContentDelta, MessagesStopReason, MessagesStreamEvent,
};
use crate::apis::openai::{
    ChatCompletionsStreamResponse, FinishReason, FunctionCallDelta, MessageDelta, Role,
    StreamChoice, ToolCallDelta, Usage,
};
use crate::apis::openai_responses::ResponsesAPIStreamEvent;

use crate::clients::TransformError;
use crate::transforms::lib::*;

// ============================================================================
// PROVIDER STREAMING TRANSFORMATIONS TO OPENAI FORMAT
// ============================================================================
//
// This module handles business logic for converting streaming events from
// various providers (Anthropic, Bedrock, etc.) into OpenAI's ChatCompletions format.
//
// # Architecture Separation
//
// **Provider Transformations** (this module):
// - Business logic for converting between provider formats
// - Uses Rust traits (TryFrom, Into) for type-safe conversions
// - Stateless event-by-event transformation
// - Example: MessagesStreamEvent → ChatCompletionsStreamResponse
//
// **Wire Format Buffering** (`apis/streaming_shapes/`):
// - SSE protocol handling (data:, event: lines)
// - State accumulation and lifecycle management
// - Buffering for stateful APIs (v1/responses)
// - Example: ChatCompletionsToResponsesTransformer
//
// # Flow
//
// ```text
// Anthropic Event → [Provider Transform] → OpenAI Event → [Wire Buffer] → SSE Wire Format
//    (business)          (this module)        (protocol)    (streaming_shapes)    (network)
// ```
//
// ============================================================================

impl TryFrom<MessagesStreamEvent> for ChatCompletionsStreamResponse {
    type Error = TransformError;

    fn try_from(event: MessagesStreamEvent) -> Result<Self, Self::Error> {
        match event {
            MessagesStreamEvent::MessageStart { message } => Ok(create_openai_chunk(
                &message.id,
                &message.model,
                MessageDelta {
                    role: Some(Role::Assistant),
                    content: None,
                    refusal: None,
                    function_call: None,
                    tool_calls: None,
                },
                None,
                None,
            )),

            MessagesStreamEvent::ContentBlockStart {
                content_block,
                index,
            } => convert_content_block_start(content_block, index),

            MessagesStreamEvent::ContentBlockDelta { delta, index } => {
                convert_content_delta(delta, index)
            }

            MessagesStreamEvent::ContentBlockStop { .. } => Ok(create_empty_openai_chunk()),

            MessagesStreamEvent::MessageDelta { delta, usage } => {
                let finish_reason: Option<FinishReason> = Some(delta.stop_reason.into());
                let openai_usage: Option<Usage> = Some(usage.into());

                Ok(create_openai_chunk(
                    "stream",
                    "unknown",
                    MessageDelta {
                        role: None,
                        content: None,
                        refusal: None,
                        function_call: None,
                        tool_calls: None,
                    },
                    finish_reason,
                    openai_usage,
                ))
            }

            MessagesStreamEvent::MessageStop => Ok(create_openai_chunk(
                "stream",
                "unknown",
                MessageDelta {
                    role: None,
                    content: None,
                    refusal: None,
                    function_call: None,
                    tool_calls: None,
                },
                Some(FinishReason::Stop),
                None,
            )),

            MessagesStreamEvent::Ping => Ok(ChatCompletionsStreamResponse {
                id: "stream".to_string(),
                object: Some("chat.completion.chunk".to_string()),
                created: current_timestamp(),
                model: "unknown".to_string(),
                choices: vec![],
                usage: None,
                system_fingerprint: None,
                service_tier: None,
            }),
        }
    }
}

impl TryFrom<ConverseStreamEvent> for ChatCompletionsStreamResponse {
    type Error = TransformError;

    fn try_from(event: ConverseStreamEvent) -> Result<Self, Self::Error> {
        match event {
            ConverseStreamEvent::MessageStart(start_event) => {
                let role = match start_event.role {
                    crate::apis::amazon_bedrock::ConversationRole::User => Role::User,
                    crate::apis::amazon_bedrock::ConversationRole::Assistant => Role::Assistant,
                };

                Ok(create_openai_chunk(
                    "stream",
                    "unknown",
                    MessageDelta {
                        role: Some(role),
                        content: None,
                        refusal: None,
                        function_call: None,
                        tool_calls: None,
                    },
                    None,
                    None,
                ))
            }

            ConverseStreamEvent::ContentBlockStart(start_event) => {
                use crate::apis::amazon_bedrock::ContentBlockStart;

                match start_event.start {
                    ContentBlockStart::ToolUse { tool_use } => Ok(create_openai_chunk(
                        "stream",
                        "unknown",
                        MessageDelta {
                            role: None,
                            content: None,
                            refusal: None,
                            function_call: None,
                            tool_calls: Some(vec![ToolCallDelta {
                                index: start_event.content_block_index as u32,
                                id: Some(tool_use.tool_use_id),
                                call_type: Some("function".to_string()),
                                function: Some(FunctionCallDelta {
                                    name: Some(tool_use.name),
                                    arguments: Some("".to_string()),
                                }),
                            }]),
                        },
                        None,
                        None,
                    )),
                }
            }

            ConverseStreamEvent::ContentBlockDelta(delta_event) => {
                use crate::apis::amazon_bedrock::ContentBlockDelta;

                match delta_event.delta {
                    ContentBlockDelta::Text { text } => Ok(create_openai_chunk(
                        "stream",
                        "unknown",
                        MessageDelta {
                            role: None,
                            content: Some(text),
                            refusal: None,
                            function_call: None,
                            tool_calls: None,
                        },
                        None,
                        None,
                    )),
                    ContentBlockDelta::ToolUse { tool_use } => Ok(create_openai_chunk(
                        "stream",
                        "unknown",
                        MessageDelta {
                            role: None,
                            content: None,
                            refusal: None,
                            function_call: None,
                            tool_calls: Some(vec![ToolCallDelta {
                                index: delta_event.content_block_index as u32,
                                id: None,
                                call_type: None,
                                function: Some(FunctionCallDelta {
                                    name: None,
                                    arguments: Some(tool_use.input),
                                }),
                            }]),
                        },
                        None,
                        None,
                    )),
                }
            }

            ConverseStreamEvent::ContentBlockStop(_) => Ok(create_empty_openai_chunk()),

            ConverseStreamEvent::MessageStop(stop_event) => {
                let finish_reason = match stop_event.stop_reason {
                    StopReason::EndTurn => FinishReason::Stop,
                    StopReason::ToolUse => FinishReason::ToolCalls,
                    StopReason::MaxTokens => FinishReason::Length,
                    StopReason::StopSequence => FinishReason::Stop,
                    StopReason::GuardrailIntervened => FinishReason::ContentFilter,
                    StopReason::ContentFiltered => FinishReason::ContentFilter,
                };

                Ok(create_openai_chunk(
                    "stream",
                    "unknown",
                    MessageDelta {
                        role: None,
                        content: None,
                        refusal: None,
                        function_call: None,
                        tool_calls: None,
                    },
                    Some(finish_reason),
                    None,
                ))
            }

            ConverseStreamEvent::Metadata(metadata_event) => {
                let usage = Usage {
                    prompt_tokens: metadata_event.usage.input_tokens,
                    completion_tokens: metadata_event.usage.output_tokens,
                    total_tokens: metadata_event.usage.total_tokens,
                    prompt_tokens_details: None,
                    completion_tokens_details: None,
                };

                Ok(create_openai_chunk(
                    "stream",
                    "unknown",
                    MessageDelta {
                        role: None,
                        content: None,
                        refusal: None,
                        function_call: None,
                        tool_calls: None,
                    },
                    None,
                    Some(usage),
                ))
            }

            // Error events - convert to empty chunks (errors should be handled elsewhere)
            ConverseStreamEvent::InternalServerException(_)
            | ConverseStreamEvent::ModelStreamErrorException(_)
            | ConverseStreamEvent::ServiceUnavailableException(_)
            | ConverseStreamEvent::ThrottlingException(_)
            | ConverseStreamEvent::ValidationException(_) => Ok(create_empty_openai_chunk()),
        }
    }
}

/// Convert content block start to OpenAI chunk
fn convert_content_block_start(
    content_block: MessagesContentBlock,
    index: u32,
) -> Result<ChatCompletionsStreamResponse, TransformError> {
    match content_block {
        MessagesContentBlock::Text { .. } => {
            // No immediate output for text block start
            Ok(create_empty_openai_chunk())
        }
        MessagesContentBlock::ToolUse { id, name, .. }
        | MessagesContentBlock::ServerToolUse { id, name, .. }
        | MessagesContentBlock::McpToolUse { id, name, .. } => {
            // Tool use start → OpenAI chunk with tool_calls
            Ok(create_openai_chunk(
                "stream",
                "unknown",
                MessageDelta {
                    role: None,
                    content: None,
                    refusal: None,
                    function_call: None,
                    tool_calls: Some(vec![ToolCallDelta {
                        index,
                        id: Some(id),
                        call_type: Some("function".to_string()),
                        function: Some(FunctionCallDelta {
                            name: Some(name),
                            arguments: Some("".to_string()),
                        }),
                    }]),
                },
                None,
                None,
            ))
        }
        _ => Err(TransformError::UnsupportedContent(
            "Unsupported content block type in stream start".to_string(),
        )),
    }
}

/// Convert content delta to OpenAI chunk
fn convert_content_delta(
    delta: MessagesContentDelta,
    index: u32,
) -> Result<ChatCompletionsStreamResponse, TransformError> {
    match delta {
        MessagesContentDelta::TextDelta { text } => Ok(create_openai_chunk(
            "stream",
            "unknown",
            MessageDelta {
                role: None,
                content: Some(text),
                refusal: None,
                function_call: None,
                tool_calls: None,
            },
            None,
            None,
        )),
        MessagesContentDelta::ThinkingDelta { thinking } => Ok(create_openai_chunk(
            "stream",
            "unknown",
            MessageDelta {
                role: None,
                content: Some(format!("thinking: {}", thinking)),
                refusal: None,
                function_call: None,
                tool_calls: None,
            },
            None,
            None,
        )),
        MessagesContentDelta::InputJsonDelta { partial_json } => Ok(create_openai_chunk(
            "stream",
            "unknown",
            MessageDelta {
                role: None,
                content: None,
                refusal: None,
                function_call: None,
                tool_calls: Some(vec![ToolCallDelta {
                    index,
                    id: None,
                    call_type: None,
                    function: Some(FunctionCallDelta {
                        name: None,
                        arguments: Some(partial_json),
                    }),
                }]),
            },
            None,
            None,
        )),
        MessagesContentDelta::SignatureDelta { signature: _ } => {
            // Signature delta is cryptographic verification metadata, not content
            // Create an empty delta chunk to maintain stream continuity
            Ok(create_openai_chunk(
                "stream",
                "unknown",
                MessageDelta {
                    role: None,
                    content: None,
                    refusal: None,
                    function_call: None,
                    tool_calls: None,
                },
                None,
                None,
            ))
        }
    }
}

/// Helper to create OpenAI streaming chunk
fn create_openai_chunk(
    id: &str,
    model: &str,
    delta: MessageDelta,
    finish_reason: Option<FinishReason>,
    usage: Option<Usage>,
) -> ChatCompletionsStreamResponse {
    ChatCompletionsStreamResponse {
        id: id.to_string(),
        object: Some("chat.completion.chunk".to_string()),
        created: current_timestamp(),
        model: model.to_string(),
        choices: vec![StreamChoice {
            index: 0,
            delta,
            finish_reason,
            logprobs: None,
        }],
        usage,
        system_fingerprint: None,
        service_tier: None,
    }
}

/// Helper to create empty OpenAI streaming chunk
fn create_empty_openai_chunk() -> ChatCompletionsStreamResponse {
    create_openai_chunk(
        "stream",
        "unknown",
        MessageDelta {
            role: None,
            content: None,
            refusal: None,
            function_call: None,
            tool_calls: None,
        },
        None,
        None,
    )
}

// Stop Reason Conversions
impl From<MessagesStopReason> for FinishReason {
    fn from(val: MessagesStopReason) -> Self {
        match val {
            MessagesStopReason::EndTurn => FinishReason::Stop,
            MessagesStopReason::MaxTokens => FinishReason::Length,
            MessagesStopReason::StopSequence => FinishReason::Stop,
            MessagesStopReason::ToolUse => FinishReason::ToolCalls,
            MessagesStopReason::PauseTurn => FinishReason::Stop,
            MessagesStopReason::Refusal => FinishReason::ContentFilter,
        }
    }
}

impl TryFrom<ChatCompletionsStreamResponse> for ResponsesAPIStreamEvent {
    type Error = TransformError;

    fn try_from(chunk: ChatCompletionsStreamResponse) -> Result<Self, TransformError> {
        // Stateless conversion - just extract the delta information
        // The buffer will manage state, item IDs, and sequence numbers

        // Extract first choice if available
        if let Some(choice) = chunk.choices.first() {
            let delta = &choice.delta;

            // Tool call with function name and/or arguments
            if let Some(tool_calls) = &delta.tool_calls {
                if let Some(tool_call) = tool_calls.first() {
                    // Extract call_id and name if available (metadata from initial event)
                    let call_id = tool_call.id.clone();
                    let function_name = tool_call.function.as_ref().and_then(|f| f.name.clone());

                    // Check if we have function metadata (name, id)
                    if let Some(function) = &tool_call.function {
                        // If we have arguments delta, return that
                        if let Some(args) = &function.arguments {
                            return Ok(
                                ResponsesAPIStreamEvent::ResponseFunctionCallArgumentsDelta {
                                    output_index: choice.index as i32,
                                    item_id: "".to_string(), // Buffer will fill this
                                    delta: args.clone(),
                                    sequence_number: 0, // Buffer will fill this
                                    call_id,
                                    name: function_name,
                                },
                            );
                        }

                        // If we have function name but no arguments yet (initial tool call event)
                        // Return an empty arguments delta so the buffer knows to create the item
                        if function.name.is_some() {
                            return Ok(
                                ResponsesAPIStreamEvent::ResponseFunctionCallArgumentsDelta {
                                    output_index: choice.index as i32,
                                    item_id: "".to_string(), // Buffer will fill this
                                    delta: "".to_string(), // Empty delta signals this is the initial event
                                    sequence_number: 0,    // Buffer will fill this
                                    call_id,
                                    name: function_name,
                                },
                            );
                        }
                    }
                }
            }

            // Text content delta
            if let Some(content) = &delta.content {
                if !content.is_empty() {
                    return Ok(ResponsesAPIStreamEvent::ResponseOutputTextDelta {
                        item_id: "".to_string(), // Buffer will fill this
                        output_index: choice.index as i32,
                        content_index: 0,
                        delta: content.clone(),
                        logprobs: vec![],
                        obfuscation: None,
                        sequence_number: 0, // Buffer will fill this
                    });
                }
            }

            // Handle finish_reason - this is a completion signal
            // Return an empty delta that the buffer can use to detect completion
            if choice.finish_reason.is_some() {
                // Return a minimal text delta to signal completion
                // The buffer will handle the finish_reason and generate response.completed
                return Ok(ResponsesAPIStreamEvent::ResponseOutputTextDelta {
                    item_id: "".to_string(), // Buffer will fill this
                    output_index: choice.index as i32,
                    content_index: 0,
                    delta: "".to_string(), // Empty delta signals completion
                    logprobs: vec![],
                    obfuscation: None,
                    sequence_number: 0, // Buffer will fill this
                });
            }

            // Empty delta with role only (common at stream start)
            if delta.role.is_some() {
                // This is typically the first chunk establishing the assistant role
                // Return an empty text delta that the buffer can use to initialize state
                return Ok(ResponsesAPIStreamEvent::ResponseOutputTextDelta {
                    item_id: "".to_string(),
                    output_index: choice.index as i32,
                    content_index: 0,
                    delta: "".to_string(),
                    logprobs: vec![],
                    obfuscation: None,
                    sequence_number: 0,
                });
            }
        }

        // Empty chunk or no convertible content (e.g., keep-alive chunks with delta: {})
        // These are valid in OpenAI streaming and should be silently ignored
        // Return error so the caller can skip these chunks without warnings
        Err(TransformError::UnsupportedConversion(
            "Empty or keep-alive chunk with no convertible content".to_string(),
        ))
    }
}
