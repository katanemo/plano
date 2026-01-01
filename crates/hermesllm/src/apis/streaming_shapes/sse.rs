use crate::apis::streaming_shapes::anthropic_streaming_buffer::AnthropicMessagesStreamBuffer;
use crate::apis::streaming_shapes::chat_completions_streaming_buffer::OpenAIChatCompletionsStreamBuffer;
use crate::apis::streaming_shapes::passthrough_streaming_buffer::PassthroughStreamBuffer;
use crate::apis::streaming_shapes::responses_api_streaming_buffer::ResponsesAPIStreamBuffer;
use crate::providers::streaming_response::ProviderStreamResponse;
use crate::providers::streaming_response::ProviderStreamResponseType;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;
use std::str::FromStr;

/// Trait defining the interface for SSE stream buffers.
///
/// This trait is implemented by both the enum `SseStreamBuffer` (for zero-cost dispatch)
/// and individual buffer implementations (for direct use).
///
pub trait SseStreamBufferTrait: Send + Sync {
    /// Add a transformed SSE event to the buffer.
    ///
    /// The buffer may inject additional events as needed based on internal state.
    /// For example, Anthropic buffers inject ContentBlockStart before the first ContentBlockDelta.
    ///
    /// All events (original + injected) are accumulated internally for the next `into_bytes()` call.
    ///
    /// # Arguments
    /// * `event` - A transformed SSE event to accumulate
    fn add_transformed_event(&mut self, event: SseEvent);

    /// Get bytes for all accumulated events since the last call.
    ///
    /// This method:
    /// - Converts all buffered events to wire format bytes
    /// - Clears the internal event buffer
    /// - Preserves state for subsequent `add_transformed_event()` calls
    ///
    /// Call this after processing each chunk of upstream events to get bytes for immediate transmission.
    ///
    /// # Returns
    /// Bytes ready for wire transmission (may be empty if no events were accumulated)
    fn to_bytes(&mut self) -> Vec<u8>;
}

/// Unified SSE Stream Buffer enum that provides a zero-cost abstraction
pub enum SseStreamBuffer {
    Passthrough(PassthroughStreamBuffer),
    OpenAIChatCompletions(OpenAIChatCompletionsStreamBuffer),
    AnthropicMessages(AnthropicMessagesStreamBuffer),
    OpenAIResponses(Box<ResponsesAPIStreamBuffer>),
}

impl SseStreamBufferTrait for SseStreamBuffer {
    fn add_transformed_event(&mut self, event: SseEvent) {
        match self {
            Self::Passthrough(buffer) => buffer.add_transformed_event(event),
            Self::OpenAIChatCompletions(buffer) => buffer.add_transformed_event(event),
            Self::AnthropicMessages(buffer) => buffer.add_transformed_event(event),
            Self::OpenAIResponses(buffer) => buffer.add_transformed_event(event),
        }
    }

    fn to_bytes(&mut self) -> Vec<u8> {
        match self {
            Self::Passthrough(buffer) => buffer.to_bytes(),
            Self::OpenAIChatCompletions(buffer) => buffer.to_bytes(),
            Self::AnthropicMessages(buffer) => buffer.to_bytes(),
            Self::OpenAIResponses(buffer) => buffer.to_bytes(),
        }
    }
}

// ============================================================================
// SSE EVENT CONTAINER
// ============================================================================

/// Represents a single Server-Sent Event with the complete wire format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SseEvent {
    #[serde(rename = "data")]
    pub data: Option<String>, // The JSON payload after "data: "

    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<String>, // Optional event type (e.g., "message_start", "content_block_delta")

    #[serde(skip_serializing, skip_deserializing)]
    pub raw_line: String, // The complete line as received including "data: " prefix and "\n\n"

    #[serde(skip_serializing, skip_deserializing)]
    pub sse_transformed_lines: String, // The complete line as received including "data: " prefix and "\n\n"

    #[serde(skip_serializing, skip_deserializing)]
    pub provider_stream_response: Option<ProviderStreamResponseType>, // Parsed provider stream response object
}

impl SseEvent {
    /// Create an SseEvent from a ProviderStreamResponseType
    /// This is useful for binary frame formats (like Bedrock) that need to be converted to SSE
    pub fn from_provider_response(response: ProviderStreamResponseType) -> Self {
        // Convert the provider response to SSE format string
        let sse_string: String = response.clone().into();

        SseEvent {
            data: None,  // Data is embedded in sse_transformed_lines
            event: None, // Event type is embedded in sse_transformed_lines
            raw_line: sse_string.clone(),
            sse_transformed_lines: sse_string,
            provider_stream_response: Some(response),
        }
    }

    /// Check if this event represents the end of the stream
    pub fn is_done(&self) -> bool {
        self.data == Some("[DONE]".into()) || self.event == Some("message_stop".into())
    }

    /// Check if this event should be skipped during processing
    /// This includes ping messages and other provider-specific events that don't contain content
    pub fn should_skip(&self) -> bool {
        // Skip ping messages (commonly used by providers for connection keep-alive)
        self.data == Some(r#"{"type": "ping"}"#.into())
    }

    /// Check if this is an event-only SSE event (no data payload)
    pub fn is_event_only(&self) -> bool {
        self.event.is_some() && self.data.is_none()
    }

    /// Get the parsed provider response if available
    pub fn provider_response(&self) -> Result<&dyn ProviderStreamResponse, std::io::Error> {
        self.provider_stream_response
            .as_ref()
            .map(|resp| resp as &dyn ProviderStreamResponse)
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "Provider response not found")
            })
    }
}

impl FromStr for SseEvent {
    type Err = SseParseError;

    fn from_str(line: &str) -> Result<Self, Self::Err> {
        // Trim leading/trailing whitespace for parsing
        let trimmed_line = line.trim();

        // Skip empty or whitespace-only lines (SSE event separators)
        if trimmed_line.is_empty() {
            return Err(SseParseError {
                message: "Empty line (SSE event separator)".to_string(),
            });
        }

        if let Some(stripped) = trimmed_line.strip_prefix("data: ") {
            let data: String = stripped.to_string();
            if data.trim().is_empty() {
                return Err(SseParseError {
                    message: "Empty data field after 'data: ' prefix".to_string(),
                });
            }
            Ok(SseEvent {
                data: Some(data),
                event: None,
                raw_line: line.to_string(),
                // Preserve original line format for passthrough, use trimmed for transformations
                sse_transformed_lines: line.to_string(),
                provider_stream_response: None,
            })
        } else if let Some(stripped) = trimmed_line.strip_prefix("event: ") {
            let event_type = stripped.to_string();
            if event_type.is_empty() {
                return Err(SseParseError {
                    message: "Empty event field is not a valid SSE event".to_string(),
                });
            }
            Ok(SseEvent {
                data: None,
                event: Some(event_type),
                raw_line: line.to_string(),
                // Preserve original line format for passthrough, use trimmed for transformations
                sse_transformed_lines: line.to_string(),
                provider_stream_response: None,
            })
        } else {
            Err(SseParseError {
                message: format!(
                    "Line does not start with 'data: ' or 'event: ': {}",
                    trimmed_line
                ),
            })
        }
    }
}

impl fmt::Display for SseEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.sse_transformed_lines)
    }
}

// Into implementation to convert SseEvent to bytes for response buffer
impl From<SseEvent> for Vec<u8> {
    fn from(val: SseEvent) -> Self {
        // For generated events (like ResponsesAPI), sse_transformed_lines already includes trailing \n\n
        // For parsed events (like passthrough), we need to add the \n\n separator
        if val.sse_transformed_lines.ends_with("\n\n") {
            // Already properly formatted with trailing newlines
            val.sse_transformed_lines.into_bytes()
        } else {
            // Add SSE event separator
            format!("{}\n\n", val.sse_transformed_lines).into_bytes()
        }
    }
}

#[derive(Debug)]
pub struct SseParseError {
    pub message: String,
}

impl fmt::Display for SseParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SSE parse error: {}", self.message)
    }
}

impl Error for SseParseError {}

/// Generic SSE (Server-Sent Events) streaming iterator container
/// Parses raw SSE lines into SseEvent objects
pub struct SseStreamIter<I>
where
    I: Iterator,
    I::Item: AsRef<str>,
{
    pub lines: I,
    pub done_seen: bool,
}

impl<I> SseStreamIter<I>
where
    I: Iterator,
    I::Item: AsRef<str>,
{
    pub fn new(lines: I) -> Self {
        Self {
            lines,
            done_seen: false,
        }
    }
}

// TryFrom implementation to parse bytes into SseStreamIter
// Handles both text-based SSE and binary AWS Event Stream formats
impl TryFrom<&[u8]> for SseStreamIter<std::vec::IntoIter<String>> {
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        // Parse as text-based SSE format
        let s = std::str::from_utf8(bytes)?;
        let lines: Vec<String> = s.lines().map(|line| line.to_string()).collect();
        Ok(SseStreamIter::new(lines.into_iter()))
    }
}

impl<I> Iterator for SseStreamIter<I>
where
    I: Iterator,
    I::Item: AsRef<str>,
{
    type Item = SseEvent;

    fn next(&mut self) -> Option<Self::Item> {
        // If we already returned [DONE], terminate the stream
        if self.done_seen {
            return None;
        }

        for line in &mut self.lines {
            let line_str = line.as_ref();

            // Try to parse as either data: or event: line
            if let Ok(event) = line_str.parse::<SseEvent>() {
                // For data: lines, check if this is the [DONE] marker
                if event.data.is_some() && event.is_done() {
                    self.done_seen = true;
                    return Some(event); // Return [DONE] event for transformation
                }
                // For data: lines, skip events that should be filtered at the transport layer
                if event.data.is_some() && event.should_skip() {
                    continue;
                }
                return Some(event);
            }
        }
        None
    }
}
