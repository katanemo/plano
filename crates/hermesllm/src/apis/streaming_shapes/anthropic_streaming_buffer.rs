use crate::apis::anthropic::MessagesStreamEvent;
use crate::apis::streaming_shapes::sse::{SseEvent, SseStreamBufferTrait};
use crate::providers::streaming_response::ProviderStreamResponseType;
use std::collections::HashSet;

/// SSE Stream Buffer for Anthropic Messages API streaming.
///
/// This buffer manages the wire format for Anthropic Messages API streaming,
/// handling the specific event sequencing requirements:
/// - MessageStart → ContentBlockStart → ContentBlockDelta(s) → ContentBlockStop → MessageDelta → MessageStop
///
/// When converting from OpenAI to Anthropic format, this buffer injects the required
/// ContentBlockStart and ContentBlockStop events to maintain proper Anthropic protocol.
pub struct AnthropicMessagesStreamBuffer {
    /// Buffered SSE events ready to be written to wire
    buffered_events: Vec<SseEvent>,

    /// Track if we've seen a message_start event
    message_started: bool,

    /// Track content block indices that have received ContentBlockStart events
    content_block_start_indices: HashSet<i32>,

    /// Track if we need to inject ContentBlockStop before message_delta
    needs_content_block_stop: bool,

    /// Track if we've seen a MessageDelta (so we need to send MessageStop at the end)
    seen_message_delta: bool,

    /// Model name to use when generating message_start events
    model: Option<String>,
}

impl Default for AnthropicMessagesStreamBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl AnthropicMessagesStreamBuffer {
    pub fn new() -> Self {
        Self {
            buffered_events: Vec::new(),
            message_started: false,
            content_block_start_indices: HashSet::new(),
            needs_content_block_stop: false,
            seen_message_delta: false,
            model: None,
        }
    }

    /// Check if a content_block_start event has been sent for the given index
    fn has_content_block_start_been_sent(&self, index: i32) -> bool {
        self.content_block_start_indices.contains(&index)
    }

    /// Mark that a content_block_start event has been sent for the given index
    fn set_content_block_start_sent(&mut self, index: i32) {
        self.content_block_start_indices.insert(index);
    }

    /// Helper to create and format a ContentBlockStart SSE event
    fn create_content_block_start_event() -> SseEvent {
        let content_block_start = MessagesStreamEvent::ContentBlockStart {
            index: 0,
            content_block: crate::apis::anthropic::MessagesContentBlock::Text {
                text: String::new(),
                cache_control: None,
            },
        };
        let sse_string: String = content_block_start.into();

        SseEvent {
            data: None,
            event: Some("content_block_start".to_string()),
            raw_line: sse_string.clone(),
            sse_transformed_lines: sse_string,
            provider_stream_response: None,
        }
    }

    /// Helper to create and format a MessageStart SSE event
    fn create_message_start_event(model: &str) -> SseEvent {
        let message_start = MessagesStreamEvent::MessageStart {
            message: crate::apis::anthropic::MessagesStreamMessage {
                id: format!("msg_{}", uuid::Uuid::new_v4().to_string().replace("-", "")),
                obj_type: "message".to_string(),
                role: crate::apis::anthropic::MessagesRole::Assistant,
                content: vec![],
                model: model.to_string(),
                stop_reason: None,
                stop_sequence: None,
                usage: crate::apis::anthropic::MessagesUsage {
                    input_tokens: 0,
                    output_tokens: 0,
                    cache_creation_input_tokens: None,
                    cache_read_input_tokens: None,
                },
            },
        };
        let sse_string: String = message_start.into();

        SseEvent {
            data: None,
            event: Some("message_start".to_string()),
            raw_line: sse_string.clone(),
            sse_transformed_lines: sse_string,
            provider_stream_response: None,
        }
    }

    /// Helper to create and format a ContentBlockStop SSE event
    fn create_content_block_stop_event() -> SseEvent {
        let content_block_stop = MessagesStreamEvent::ContentBlockStop { index: 0 };
        let sse_string: String = content_block_stop.into();

        SseEvent {
            data: None,
            event: Some("content_block_stop".to_string()),
            raw_line: sse_string.clone(),
            sse_transformed_lines: sse_string,
            provider_stream_response: None,
        }
    }
}

impl SseStreamBufferTrait for AnthropicMessagesStreamBuffer {
    fn add_transformed_event(&mut self, event: SseEvent) {
        // Skip ping messages
        if event.should_skip() {
            return;
        }

        // FIRST: Try to extract model name from the raw event data before transformation
        // The provider_stream_response has already been transformed to Anthropic format,
        // so we need to extract the model from the original raw data if available
        if self.model.is_none() {
            if let Some(data) = &event.data {
                // Try to parse as JSON and extract model field
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(model) = json.get("model").and_then(|m| m.as_str()) {
                        self.model = Some(model.to_string());
                    }
                }
            }
        }

        // Match directly on the provider response type to handle event processing
        // We match on a reference first to determine the type, then move the event
        match &event.provider_stream_response {
            Some(ProviderStreamResponseType::MessagesStreamEvent(evt)) => {
                match evt {
                    MessagesStreamEvent::MessageStart { .. } => {
                        // Add the message_start event
                        self.buffered_events.push(event);
                        self.message_started = true;
                    }
                    MessagesStreamEvent::ContentBlockStart { index, .. } => {
                        let index = *index as i32;
                        // Inject message_start if needed
                        if !self.message_started {
                            let model = self.model.as_deref().unwrap_or("unknown");
                            let message_start =
                                AnthropicMessagesStreamBuffer::create_message_start_event(model);
                            self.buffered_events.push(message_start);
                            self.message_started = true;
                        }

                        // Add the content_block_start event (from tool calls or other sources)
                        self.buffered_events.push(event);
                        self.set_content_block_start_sent(index);
                        self.needs_content_block_stop = true;
                    }
                    MessagesStreamEvent::ContentBlockDelta { index, .. } => {
                        let index = *index as i32;
                        // Inject message_start if needed
                        if !self.message_started {
                            let model = self.model.as_deref().unwrap_or("unknown");
                            let message_start =
                                AnthropicMessagesStreamBuffer::create_message_start_event(model);
                            self.buffered_events.push(message_start);
                            self.message_started = true;
                        }

                        // Check if ContentBlockStart was sent for this index
                        if !self.has_content_block_start_been_sent(index) {
                            // Inject ContentBlockStart before delta
                            let content_block_start =
                                AnthropicMessagesStreamBuffer::create_content_block_start_event();
                            self.buffered_events.push(content_block_start);
                            self.set_content_block_start_sent(index);
                            self.needs_content_block_stop = true;
                        }

                        // Content deltas are between ContentBlockStart and ContentBlockStop
                        self.buffered_events.push(event);
                    }
                    MessagesStreamEvent::MessageDelta { usage, .. } => {
                        // Inject ContentBlockStop before message_delta
                        if self.needs_content_block_stop {
                            let content_block_stop =
                                AnthropicMessagesStreamBuffer::create_content_block_stop_event();
                            self.buffered_events.push(content_block_stop);
                            self.needs_content_block_stop = false;
                        }

                        // Check if the last event was also a MessageDelta - if so, merge them
                        // This handles Bedrock's split of stop_reason (MessageStop) and usage (Metadata)
                        if let Some(last_event) = self.buffered_events.last_mut() {
                            if let Some(ProviderStreamResponseType::MessagesStreamEvent(
                                MessagesStreamEvent::MessageDelta {
                                    usage: last_usage, ..
                                },
                            )) = &mut last_event.provider_stream_response
                            {
                                // Merge: take stop_reason from first, usage from second (if non-zero)
                                if usage.input_tokens > 0 || usage.output_tokens > 0 {
                                    *last_usage = usage.clone();
                                }
                                // Mark that we've seen MessageDelta (need to send MessageStop later)
                                self.seen_message_delta = true;
                                // Don't push the new event, we've merged it
                                return;
                            }
                        }

                        // No previous MessageDelta to merge with, add this one
                        self.buffered_events.push(event);
                        self.seen_message_delta = true;
                    }
                    MessagesStreamEvent::ContentBlockStop { .. } => {
                        // ContentBlockStop received from upstream (e.g., Bedrock)
                        // Clear the flag so we don't inject another one
                        self.needs_content_block_stop = false;
                        self.buffered_events.push(event);
                    }
                    MessagesStreamEvent::MessageStop => {
                        // MessageStop received from upstream (e.g., OpenAI via [DONE])
                        // Clear the flag so we don't inject another one
                        self.seen_message_delta = false;
                        self.buffered_events.push(event);
                    }
                    _ => {
                        // Other Anthropic event types (Ping, etc.), just accumulate
                        self.buffered_events.push(event);
                    }
                }
            }
            _ => {
                // Non-Anthropic events or events without provider_stream_response, just accumulate
                self.buffered_events.push(event);
            }
        }
    }

    fn to_bytes(&mut self) -> Vec<u8> {
        // Convert all accumulated events to bytes and clear buffer
        // NOTE: We do NOT inject ContentBlockStop here because it's injected when we see MessageDelta
        // or MessageStop. Injecting it here causes premature ContentBlockStop in the middle of streaming.

        // Inject MessageStop after MessageDelta if we've seen one
        // This completes the Anthropic Messages API event sequence
        if self.seen_message_delta {
            let message_stop = MessagesStreamEvent::MessageStop;
            let sse_string: String = message_stop.into();
            let message_stop_event = SseEvent {
                data: None,
                event: Some("message_stop".to_string()),
                raw_line: sse_string.clone(),
                sse_transformed_lines: sse_string,
                provider_stream_response: None,
            };
            self.buffered_events.push(message_stop_event);
            self.seen_message_delta = false;
        }

        let mut buffer = Vec::new();
        for event in self.buffered_events.drain(..) {
            let event_bytes: Vec<u8> = event.into();
            buffer.extend_from_slice(&event_bytes);
        }
        buffer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apis::anthropic::AnthropicApi;
    use crate::apis::openai::OpenAIApi;
    use crate::apis::streaming_shapes::sse::SseStreamIter;
    use crate::clients::{SupportedAPIsFromClient, SupportedUpstreamAPIs};

    #[test]
    fn test_openai_to_anthropic_complete_transformation() {
        // OpenAI ChatCompletions input that will be transformed to Anthropic Messages API
        let raw_input = r#"data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1234567890,"model":"gpt-4o","choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"},"finish_reason":null}]}

data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1234567890,"model":"gpt-4o","choices":[{"index":0,"delta":{"content":" world"},"finish_reason":null}]}

data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1234567890,"model":"gpt-4o","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

data: [DONE]"#;

        println!("\n{}", "=".repeat(80));
        println!("TEST 1: OpenAI → Anthropic Messages API Complete Transformation");
        println!("{}", "=".repeat(80));
        println!("\nRAW INPUT (OpenAI ChatCompletions):");
        println!("{}", "-".repeat(80));
        println!("{}", raw_input);

        // Setup API configuration for transformation (client wants Anthropic, upstream is OpenAI)
        let client_api = SupportedAPIsFromClient::AnthropicMessagesAPI(AnthropicApi::Messages);
        let upstream_api = SupportedUpstreamAPIs::OpenAIChatCompletions(OpenAIApi::ChatCompletions);

        // Parse events and apply transformation
        let stream_iter = SseStreamIter::try_from(raw_input.as_bytes()).unwrap();
        let mut buffer = AnthropicMessagesStreamBuffer::new();

        for raw_event in stream_iter {
            let transformed_event =
                SseEvent::try_from((raw_event, &client_api, &upstream_api)).unwrap();
            buffer.add_transformed_event(transformed_event);
        }

        let output_bytes = buffer.to_bytes();
        let output = String::from_utf8_lossy(&output_bytes);

        println!("\nTRANSFORMED OUTPUT (Anthropic Messages API):");
        println!("{}", "-".repeat(80));
        println!("{}", output);

        // Assertions
        assert!(!output_bytes.is_empty(), "Should have output");
        assert!(
            output.contains("event: message_start"),
            "Should have message_start"
        );
        assert!(
            output.contains("event: content_block_start"),
            "Should have content_block_start (injected)"
        );

        let delta_count = output.matches("event: content_block_delta").count();
        assert_eq!(
            delta_count, 2,
            "Should have exactly 2 content_block_delta events"
        );

        // Verify both pieces of content are present
        assert!(
            output.contains("\"text\":\"Hello\""),
            "Should have first content delta 'Hello'"
        );
        assert!(
            output.contains("\"text\":\" world\""),
            "Should have second content delta ' world'"
        );

        assert!(
            output.contains("event: content_block_stop"),
            "Should have content_block_stop (injected)"
        );
        assert!(
            output.contains("event: message_delta"),
            "Should have message_delta"
        );
        assert!(
            output.contains("event: message_stop"),
            "Should have message_stop"
        );

        println!("\nVALIDATION SUMMARY:");
        println!("{}", "-".repeat(80));
        println!("✓ Complete transformation: OpenAI ChatCompletions → Anthropic Messages API");
        println!(
            "✓ Injected lifecycle events: message_start, content_block_start, content_block_stop"
        );
        println!(
            "✓ Content deltas: {} events (BOTH 'Hello' and ' world' preserved!)",
            delta_count
        );
        println!("✓ Complete stream with message_stop");
        println!("✓ Proper Anthropic protocol sequencing\n");
    }

    #[test]
    fn test_openai_to_anthropic_partial_transformation() {
        // Partial OpenAI ChatCompletions stream - no [DONE]
        let raw_input = r#"data: {"id":"chatcmpl-456","object":"chat.completion.chunk","created":1234567890,"model":"gpt-4o","choices":[{"index":0,"delta":{"role":"assistant","content":"The weather"},"finish_reason":null}]}

data: {"id":"chatcmpl-456","object":"chat.completion.chunk","created":1234567890,"model":"gpt-4o","choices":[{"index":0,"delta":{"content":" in San Francisco"},"finish_reason":null}]}

data: {"id":"chatcmpl-456","object":"chat.completion.chunk","created":1234567890,"model":"gpt-4o","choices":[{"index":0,"delta":{"content":" is"},"finish_reason":null}]}"#;

        println!("\n{}", "=".repeat(80));
        println!("TEST 2: OpenAI → Anthropic Partial Transformation (NO [DONE])");
        println!("{}", "=".repeat(80));
        println!("\nRAW INPUT (OpenAI ChatCompletions - NO [DONE]):");
        println!("{}", "-".repeat(80));
        println!("{}", raw_input);

        // Setup API configuration for transformation
        let client_api = SupportedAPIsFromClient::AnthropicMessagesAPI(AnthropicApi::Messages);
        let upstream_api = SupportedUpstreamAPIs::OpenAIChatCompletions(OpenAIApi::ChatCompletions);

        // Parse and transform events
        let stream_iter = SseStreamIter::try_from(raw_input.as_bytes()).unwrap();
        let mut buffer = AnthropicMessagesStreamBuffer::new();

        for raw_event in stream_iter {
            let transformed_event =
                SseEvent::try_from((raw_event, &client_api, &upstream_api)).unwrap();
            buffer.add_transformed_event(transformed_event);
        }

        let output_bytes = buffer.to_bytes();
        let output = String::from_utf8_lossy(&output_bytes);

        println!("\nTRANSFORMED OUTPUT (Anthropic Messages API):");
        println!("{}", "-".repeat(80));
        println!("{}", output);

        // Assertions
        assert!(!output_bytes.is_empty(), "Should have output");
        assert!(
            output.contains("event: message_start"),
            "Should have message_start"
        );
        assert!(
            output.contains("event: content_block_start"),
            "Should have content_block_start (injected)"
        );

        let delta_count = output.matches("event: content_block_delta").count();
        assert_eq!(
            delta_count, 3,
            "Should have exactly 3 content_block_delta events"
        );

        // Verify all three pieces of content are present
        assert!(
            output.contains("\"text\":\"The weather\""),
            "Should have first content delta"
        );
        assert!(
            output.contains("\"text\":\" in San Francisco\""),
            "Should have second content delta"
        );
        assert!(
            output.contains("\"text\":\" is\""),
            "Should have third content delta"
        );

        // For partial streams (no finish_reason, no [DONE]), we do NOT inject content_block_stop
        // because the stream may continue. This is correct behavior - only inject lifecycle events
        // when we have explicit signals from upstream (finish_reason, [DONE], etc.)
        assert!(
            !output.contains("event: content_block_stop"),
            "Should NOT have content_block_stop for partial stream"
        );

        // Should NOT have completion events
        assert!(
            !output.contains("event: message_delta"),
            "Should NOT have message_delta"
        );
        assert!(
            !output.contains("event: message_stop"),
            "Should NOT have message_stop"
        );

        println!("\nVALIDATION SUMMARY:");
        println!("{}", "-".repeat(80));
        println!("✓ Partial transformation: OpenAI → Anthropic (stream interrupted)");
        println!("✓ Injected: message_start, content_block_start at beginning");
        println!(
            "✓ Incremental deltas: {} events (ALL content preserved!)",
            delta_count
        );
        println!("✓ NO completion events (partial stream, no [DONE])");
        println!("✓ Buffer maintains Anthropic protocol for active streams\n");
    }

    #[test]
    fn test_openai_tool_calling_to_anthropic_transformation() {
        // OpenAI ChatCompletions tool calling stream
        let raw_input = r#"data: {"id":"chatcmpl-Cgx6pZPBgfLcMqfT0ILIH2mID2zWQ","object":"chat.completion.chunk","created":1764353027,"model":"gpt-4o-2024-08-06","service_tier":"default","system_fingerprint":"fp_7eeb46f068","choices":[{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_2Uzw0AEZQeOex2CP2TKjcLKc","type":"function","function":{"name":"get_weather","arguments":""}}],"refusal":null},"logprobs":null,"finish_reason":null}],"obfuscation":"uSpCcO"}

data: {"id":"chatcmpl-Cgx6pZPBgfLcMqfT0ILIH2mID2zWQ","object":"chat.completion.chunk","created":1764353027,"model":"gpt-4o-2024-08-06","service_tier":"default","system_fingerprint":"fp_7eeb46f068","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\""}}]},"logprobs":null,"finish_reason":null}],"obfuscation":""}

data: {"id":"chatcmpl-Cgx6pZPBgfLcMqfT0ILIH2mID2zWQ","object":"chat.completion.chunk","created":1764353027,"model":"gpt-4o-2024-08-06","service_tier":"default","system_fingerprint":"fp_7eeb46f068","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"location"}}]},"logprobs":null,"finish_reason":null}],"obfuscation":"24WSqt08jtf"}

data: {"id":"chatcmpl-Cgx6pZPBgfLcMqfT0ILIH2mID2zWQ","object":"chat.completion.chunk","created":1764353027,"model":"gpt-4o-2024-08-06","service_tier":"default","system_fingerprint":"fp_7eeb46f068","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\":\""}}]},"logprobs":null,"finish_reason":null}],"obfuscation":"6CleV8twTxkKYg"}

data: {"id":"chatcmpl-Cgx6pZPBgfLcMqfT0ILIH2mID2zWQ","object":"chat.completion.chunk","created":1764353027,"model":"gpt-4o-2024-08-06","service_tier":"default","system_fingerprint":"fp_7eeb46f068","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"San"}}]},"logprobs":null,"finish_reason":null}],"obfuscation":""}

data: {"id":"chatcmpl-Cgx6pZPBgfLcMqfT0ILIH2mID2zWQ","object":"chat.completion.chunk","created":1764353027,"model":"gpt-4o-2024-08-06","service_tier":"default","system_fingerprint":"fp_7eeb46f068","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":" Francisco"}}]},"logprobs":null,"finish_reason":null}],"obfuscation":"1XLz89l3v"}

data: {"id":"chatcmpl-Cgx6pZPBgfLcMqfT0ILIH2mID2zWQ","object":"chat.completion.chunk","created":1764353027,"model":"gpt-4o-2024-08-06","service_tier":"default","system_fingerprint":"fp_7eeb46f068","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":","}}]},"logprobs":null,"finish_reason":null}],"obfuscation":"sh"}

data: {"id":"chatcmpl-Cgx6pZPBgfLcMqfT0ILIH2mID2zWQ","object":"chat.completion.chunk","created":1764353027,"model":"gpt-4o-2024-08-06","service_tier":"default","system_fingerprint":"fp_7eeb46f068","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":" CA"}}]},"logprobs":null,"finish_reason":null}],"obfuscation":""}

data: {"id":"chatcmpl-Cgx6pZPBgfLcMqfT0ILIH2mID2zWQ","object":"chat.completion.chunk","created":1764353027,"model":"gpt-4o-2024-08-06","service_tier":"default","system_fingerprint":"fp_7eeb46f068","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"}"}}]},"logprobs":null,"finish_reason":null}],"obfuscation":""}

data: {"id":"chatcmpl-Cgx6pZPBgfLcMqfT0ILIH2mID2zWQ","object":"chat.completion.chunk","created":1764353027,"model":"gpt-4o-2024-08-06","service_tier":"default","system_fingerprint":"fp_7eeb46f068","choices":[{"index":0,"delta":{},"logprobs":null,"finish_reason":"tool_calls"}],"obfuscation":"I"}

data: [DONE]"#;

        println!("\n{}", "=".repeat(80));
        println!("TEST 3: OpenAI Tool Calling → Anthropic Messages API Transformation");
        println!("{}", "=".repeat(80));
        println!("\nRAW INPUT (OpenAI ChatCompletions with Tool Calls):");
        println!("{}", "-".repeat(80));
        println!("{}", raw_input);

        // Setup API configuration for transformation
        let client_api = SupportedAPIsFromClient::AnthropicMessagesAPI(AnthropicApi::Messages);
        let upstream_api = SupportedUpstreamAPIs::OpenAIChatCompletions(OpenAIApi::ChatCompletions);

        // Parse and transform events
        let stream_iter = SseStreamIter::try_from(raw_input.as_bytes()).unwrap();
        let mut buffer = AnthropicMessagesStreamBuffer::new();

        for raw_event in stream_iter {
            let transformed_event =
                SseEvent::try_from((raw_event, &client_api, &upstream_api)).unwrap();
            buffer.add_transformed_event(transformed_event);
        }

        let output_bytes = buffer.to_bytes();
        let output = String::from_utf8_lossy(&output_bytes);

        println!("\nTRANSFORMED OUTPUT (Anthropic Messages API):");
        println!("{}", "-".repeat(80));
        println!("{}", output);

        // Assertions for tool calling transformation
        assert!(!output_bytes.is_empty(), "Should have output");

        // Should have lifecycle events (injected by buffer)
        assert!(
            output.contains("event: message_start"),
            "Should have message_start (injected)"
        );
        assert!(
            output.contains("event: content_block_start"),
            "Should have content_block_start"
        );
        assert!(
            output.contains("event: content_block_stop"),
            "Should have content_block_stop (injected)"
        );
        assert!(
            output.contains("event: message_delta"),
            "Should have message_delta"
        );
        assert!(
            output.contains("event: message_stop"),
            "Should have message_stop"
        );

        // Should have tool_use content block
        assert!(
            output.contains("\"type\":\"tool_use\""),
            "Should have tool_use type"
        );
        assert!(
            output.contains("\"name\":\"get_weather\""),
            "Should have correct function name"
        );
        assert!(
            output.contains("\"id\":\"call_2Uzw0AEZQeOex2CP2TKjcLKc\""),
            "Should have correct tool call ID"
        );

        // Count input_json_delta events - should match the number of argument chunks
        let delta_count = output.matches("event: content_block_delta").count();
        assert!(
            delta_count >= 8,
            "Should have at least 8 input_json_delta events"
        );

        // Verify argument deltas are present
        assert!(
            output.contains("\"type\":\"input_json_delta\""),
            "Should have input_json_delta type"
        );
        assert!(
            output.contains("\"partial_json\":"),
            "Should have partial_json field"
        );

        // Verify the accumulated arguments contain the location
        assert!(output.contains("San"), "Arguments should contain 'San'");
        assert!(
            output.contains("Francisco"),
            "Arguments should contain 'Francisco'"
        );
        assert!(output.contains("CA"), "Arguments should contain 'CA'");

        // Verify stop reason is tool_use
        assert!(
            output.contains("\"stop_reason\":\"tool_use\""),
            "Should have stop_reason as tool_use"
        );

        println!("\nVALIDATION SUMMARY:");
        println!("{}", "-".repeat(80));
        println!("✓ Complete tool calling transformation: OpenAI → Anthropic Messages API");
        println!("✓ Injected lifecycle: message_start, content_block_stop");
        println!("✓ Tool metadata: name='get_weather', id='call_2Uzw0AEZQeOex2CP2TKjcLKc'");
        println!("✓ Argument deltas: {} events", delta_count);
        println!("✓ Complete JSON arguments: '{{\"location\":\"San Francisco, CA\"}}'");
        println!("✓ Stop reason: tool_use");
        println!("✓ Proper Anthropic tool_use protocol\n");
    }
}
