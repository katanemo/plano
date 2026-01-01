//! Conversions from response outputs to request inputs for conversation continuation
//!
//! This module provides utilities for converting OutputItem types from API responses
//! into InputItem types that can be used in subsequent requests. This is primarily used
//! for maintaining conversation history in the v1/responses API.

use crate::apis::openai_responses::{
    InputContent, InputItem, InputMessage, MessageContent, MessageRole, OutputContent, OutputItem,
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
                        Some(InputContent::InputText { text: text.clone() })
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
        // For function calls, we'll create an assistant message with the tool call info
        // This matches how conversation history is typically built
        OutputItem::FunctionCall {
            name, arguments, ..
        } => {
            let tool_call_text = if let (Some(n), Some(args)) = (name, arguments) {
                format!("Called function: {} with arguments: {}", n, args)
            } else {
                "Called a function".to_string()
            };

            Some(InputItem::Message(InputMessage {
                role: MessageRole::Assistant,
                content: MessageContent::Items(vec![InputContent::InputText {
                    text: tool_call_text,
                }]),
            }))
        }
        // Skip other output types (tool outputs, etc.) as they don't convert to input
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
                            InputContent::InputText { text } => assert_eq!(text, "Hello!"),
                            _ => panic!("Expected InputText"),
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
            InputItem::Message(msg) => {
                assert!(matches!(msg.role, MessageRole::Assistant));
                match &msg.content {
                    MessageContent::Items(items) => match &items[0] {
                        InputContent::InputText { text } => {
                            assert!(text.contains("get_weather"));
                        }
                        _ => panic!("Expected InputText"),
                    },
                    _ => panic!("Expected MessageContent::Items"),
                }
            }
            _ => panic!("Expected Message variant"),
        }
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
