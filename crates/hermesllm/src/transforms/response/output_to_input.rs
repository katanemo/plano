//! Conversions from response outputs to request inputs for conversation continuation
//!
//! This module provides utilities for converting OutputItem types from API responses
//! into InputItem types that can be used in subsequent requests. This is primarily used
//! for maintaining conversation history in the v1/responses API.

use crate::apis::openai_responses::{
    InputContent, InputItem, InputMessage, MessageContent, MessageRole, OutputContent, OutputItem,
    OutputItemStatus,
};

/// Converts an OutputItem from a response into an InputItem for the next request
/// This is used to build conversation history from previous responses
pub fn convert_responses_output_to_input_items(output: &OutputItem) -> Option<InputItem> {
    match output {
        // Convert output messages to input messages
        OutputItem::Message { role, content, .. } => {
            let input_content: Vec<InputContent> = content
                .iter()
                .filter_map(|c| match c {
                    OutputContent::OutputText { text, .. } => {
                        // Assistant (output-role) content must round-trip as
                        // output_text; the Responses API rejects input_text here.
                        Some(InputContent::OutputText { text: text.clone() })
                    }
                    OutputContent::OutputAudio { data, .. } => Some(InputContent::InputAudio {
                        data: data.clone(),
                        format: None, // Format not preserved in output
                    }),
                    OutputContent::Refusal { .. } => None, // Skip refusals
                })
                .collect();

            if input_content.is_empty() {
                return None;
            }

            // Map role string to MessageRole enum
            let message_role = match role.as_str() {
                "user" => MessageRole::User,
                "assistant" => MessageRole::Assistant,
                "system" => MessageRole::System,
                "developer" => MessageRole::Developer,
                _ => MessageRole::Assistant, // Default to assistant
            };

            Some(InputItem::Message(InputMessage {
                role: message_role,
                content: MessageContent::Items(input_content),
            }))
        }
        // Keep tool calls structured so stored history pairs with the client's
        // follow-up function_call_output by call_id; flattening to text would
        // orphan that tool result on the next turn.
        OutputItem::FunctionCall {
            id,
            status,
            call_id,
            name,
            arguments,
        } => {
            // Preserve the status as its wire string.
            let status_str = match status {
                OutputItemStatus::InProgress => "in_progress",
                OutputItemStatus::Completed => "completed",
                OutputItemStatus::Incomplete => "incomplete",
            };

            Some(InputItem::FunctionCall {
                // Name/arguments are optional on the output wire but required on
                // the input side. The pair is keyed by call_id, so an empty
                // string stands in for an upstream that omitted them.
                name: name.clone().unwrap_or_default(),
                arguments: arguments.clone().unwrap_or_default(),
                call_id: call_id.clone(),
                id: Some(id.clone()),
                status: Some(status_str.to_string()),
            })
        }
        // Remaining output types do not convert to input. CustomToolCall in
        // particular is dropped because the output type has no name field, so
        // the required `name` on InputItem::CustomToolCall cannot be
        // reconstructed without widening the output schema.
        _ => None,
    }
}

/// Converts a Vec of OutputItems into InputItems for conversation continuation
pub fn outputs_to_inputs(outputs: &[OutputItem]) -> Vec<InputItem> {
    outputs
        .iter()
        .filter_map(convert_responses_output_to_input_items)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apis::openai_responses::OutputItemStatus;

    #[test]
    fn test_output_message_to_input() {
        let output = OutputItem::Message {
            id: "msg_123".to_string(),
            status: OutputItemStatus::Completed,
            role: "assistant".to_string(),
            content: vec![OutputContent::OutputText {
                text: "Hello!".to_string(),
                annotations: vec![],
                logprobs: None,
            }],
        };

        let input = convert_responses_output_to_input_items(&output).unwrap();

        match input {
            InputItem::Message(msg) => {
                assert!(matches!(msg.role, MessageRole::Assistant));
                match &msg.content {
                    MessageContent::Items(items) => {
                        assert_eq!(items.len(), 1);
                        match &items[0] {
                            InputContent::OutputText { text } => assert_eq!(text, "Hello!"),
                            _ => panic!("Expected OutputText"),
                        }
                    }
                    _ => panic!("Expected MessageContent::Items"),
                }
            }
            _ => panic!("Expected Message variant"),
        }
    }

    #[test]
    fn test_function_call_to_input() {
        let output = OutputItem::FunctionCall {
            id: "fc_123".to_string(),
            status: OutputItemStatus::Completed,
            call_id: "call_123".to_string(),
            name: Some("get_weather".to_string()),
            arguments: Some(r#"{"location":"SF"}"#.to_string()),
        };

        let input = convert_responses_output_to_input_items(&output).unwrap();

        match input {
            InputItem::FunctionCall {
                name,
                arguments,
                call_id,
                id,
                status,
            } => {
                assert_eq!(name, "get_weather");
                assert_eq!(arguments, r#"{"location":"SF"}"#);
                assert_eq!(call_id, "call_123");
                assert_eq!(id.as_deref(), Some("fc_123"));
                assert_eq!(status.as_deref(), Some("completed"));
            }
            _ => panic!("Expected FunctionCall variant"),
        }
    }

    #[test]
    fn function_call_round_trips_with_call_id() {
        let output = OutputItem::FunctionCall {
            id: "fc_abc".to_string(),
            status: OutputItemStatus::Completed,
            call_id: "call_abc".to_string(),
            name: Some("get_weather".to_string()),
            arguments: Some(r#"{"location":"SF"}"#.to_string()),
        };

        let stored = outputs_to_inputs(std::slice::from_ref(&output));

        // The stored history must keep the call structured instead of
        // flattening it into assistant text, so the client's follow-up
        // `function_call_output` pairs with it by `call_id` rather than
        // arriving as an orphaned tool result on the next turn.
        assert_eq!(stored.len(), 1);
        match &stored[0] {
            InputItem::FunctionCall {
                name,
                arguments,
                call_id,
                id,
                status,
            } => {
                assert_eq!(name, "get_weather");
                assert_eq!(arguments, r#"{"location":"SF"}"#);
                assert_eq!(call_id, "call_abc");
                assert_eq!(id.as_deref(), Some("fc_abc"));
                assert_eq!(status.as_deref(), Some("completed"));
            }
            other => panic!("Expected structured FunctionCall, got {other:?}"),
        }

        // Downstream pairing of the stored call with the client's follow-up
        // `function_call_output` is covered by
        // `transforms::request::from_openai::tests::test_responses_function_call_and_output_preserve_call_id_link`;
        // this test only pins the storage-side shape that makes it possible.
    }

    #[test]
    fn test_outputs_to_inputs() {
        let outputs = vec![
            OutputItem::Message {
                id: "msg_1".to_string(),
                status: OutputItemStatus::Completed,
                role: "assistant".to_string(),
                content: vec![OutputContent::OutputText {
                    text: "Hello".to_string(),
                    annotations: vec![],
                    logprobs: None,
                }],
            },
            OutputItem::FunctionCall {
                id: "fc_1".to_string(),
                status: OutputItemStatus::Completed,
                call_id: "call_1".to_string(),
                name: Some("test".to_string()),
                arguments: Some("{}".to_string()),
            },
        ];

        let inputs = outputs_to_inputs(&outputs);
        assert_eq!(inputs.len(), 2);
    }
}
