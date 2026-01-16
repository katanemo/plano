use crate::apis::anthropic::{MessagesContentBlock, MessagesImageSource};
use crate::apis::openai::{ContentPart, FunctionCall, ImageUrl, Message, MessageContent, ToolCall};
use crate::clients::TransformError;
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

pub trait ExtractText {
    fn extract_text(&self) -> String;
}

/// Trait for utility functions on content collections
pub trait ContentUtils<T> {
    fn extract_tool_calls(&self) -> Result<Option<Vec<ToolCall>>, TransformError>;
    fn split_for_openai(&self) -> Result<SplitForOpenAIResult, TransformError>;
}

pub type SplitForOpenAIResult = (Vec<ContentPart>, Vec<ToolCall>, Vec<(String, String, bool)>);

/// Helper to create a current unix timestamp
pub fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

// Content Utilities
impl ContentUtils<ToolCall> for Vec<MessagesContentBlock> {
    fn extract_tool_calls(&self) -> Result<Option<Vec<ToolCall>>, TransformError> {
        let mut tool_calls = Vec::new();

        for block in self {
            match block {
                MessagesContentBlock::ToolUse {
                    id, name, input, ..
                }
                | MessagesContentBlock::ServerToolUse { id, name, input }
                | MessagesContentBlock::McpToolUse { id, name, input } => {
                    let arguments = serde_json::to_string(&input)?;
                    tool_calls.push(ToolCall {
                        id: id.clone(),
                        call_type: "function".to_string(),
                        function: FunctionCall {
                            name: name.clone(),
                            arguments,
                        },
                    });
                }
                _ => continue,
            }
        }

        Ok(if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        })
    }

    fn split_for_openai(
        &self,
    ) -> Result<(Vec<ContentPart>, Vec<ToolCall>, Vec<(String, String, bool)>), TransformError>
    {
        let mut content_parts = Vec::new();
        let mut tool_calls = Vec::new();
        let mut tool_results = Vec::new();

        for block in self {
            match block {
                MessagesContentBlock::Text { text, .. } => {
                    content_parts.push(ContentPart::Text { text: text.clone() });
                }
                MessagesContentBlock::Image { source } => {
                    let url = convert_image_source_to_url(source);
                    content_parts.push(ContentPart::ImageUrl {
                        image_url: ImageUrl {
                            url,
                            detail: Some("auto".to_string()),
                        },
                    });
                }
                MessagesContentBlock::ToolUse {
                    id, name, input, ..
                }
                | MessagesContentBlock::ServerToolUse { id, name, input }
                | MessagesContentBlock::McpToolUse { id, name, input } => {
                    let arguments = serde_json::to_string(&input)?;
                    tool_calls.push(ToolCall {
                        id: id.clone(),
                        call_type: "function".to_string(),
                        function: FunctionCall {
                            name: name.clone(),
                            arguments,
                        },
                    });
                }
                MessagesContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                    ..
                } => {
                    let result_text = content.extract_text();
                    tool_results.push((
                        tool_use_id.clone(),
                        result_text,
                        is_error.unwrap_or(false),
                    ));
                }
                MessagesContentBlock::WebSearchToolResult {
                    tool_use_id,
                    content,
                    is_error,
                }
                | MessagesContentBlock::CodeExecutionToolResult {
                    tool_use_id,
                    content,
                    is_error,
                }
                | MessagesContentBlock::McpToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } => {
                    let result_text = content.extract_text();
                    tool_results.push((
                        tool_use_id.clone(),
                        result_text,
                        is_error.unwrap_or(false),
                    ));
                }
                _ => {
                    // Skip unsupported content types
                    continue;
                }
            }
        }

        Ok((content_parts, tool_calls, tool_results))
    }
}

/// Convert image source to URL
pub fn convert_image_source_to_url(source: &MessagesImageSource) -> String {
    match source {
        MessagesImageSource::Base64 { media_type, data } => {
            format!("data:{};base64,{}", media_type, data)
        }
        MessagesImageSource::Url { url } => url.clone(),
    }
}

/// Convert image URL to Anthropic image source
fn convert_image_url_to_source(image_url: &ImageUrl) -> MessagesImageSource {
    if image_url.url.starts_with("data:") {
        // Parse data URL
        let parts: Vec<&str> = image_url.url.splitn(2, ',').collect();
        if parts.len() == 2 {
            let header = parts[0];
            let data = parts[1];
            let media_type = header
                .strip_prefix("data:")
                .and_then(|s| s.split(';').next())
                .unwrap_or("image/jpeg")
                .to_string();

            MessagesImageSource::Base64 {
                media_type,
                data: data.to_string(),
            }
        } else {
            MessagesImageSource::Url {
                url: image_url.url.clone(),
            }
        }
    } else {
        MessagesImageSource::Url {
            url: image_url.url.clone(),
        }
    }
}

/// Convert OpenAI message to Anthropic content blocks
pub fn convert_openai_message_to_anthropic_content(
    message: &Message,
) -> Result<Vec<MessagesContentBlock>, TransformError> {
    let mut blocks = Vec::new();

    // Handle regular content
    match &message.content {
        MessageContent::Text(text) => {
            if !text.is_empty() {
                blocks.push(MessagesContentBlock::Text {
                    text: text.clone(),
                    cache_control: None,
                });
            }
        }
        MessageContent::Parts(parts) => {
            for part in parts {
                match part {
                    ContentPart::Text { text } => {
                        blocks.push(MessagesContentBlock::Text {
                            text: text.clone(),
                            cache_control: None,
                        });
                    }
                    ContentPart::ImageUrl { image_url } => {
                        let source = convert_image_url_to_source(image_url);
                        blocks.push(MessagesContentBlock::Image { source });
                    }
                }
            }
        }
    }

    // Handle tool calls
    if let Some(tool_calls) = &message.tool_calls {
        for tool_call in tool_calls {
            let input: Value = serde_json::from_str(&tool_call.function.arguments)?;
            blocks.push(MessagesContentBlock::ToolUse {
                id: tool_call.id.clone(),
                name: tool_call.function.name.clone(),
                input,
                cache_control: None,
            });
        }
    }

    Ok(blocks)
}
