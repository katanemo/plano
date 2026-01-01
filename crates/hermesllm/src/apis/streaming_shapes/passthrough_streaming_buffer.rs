use crate::apis::streaming_shapes::sse::{SseEvent, SseStreamBufferTrait};

/// Passthrough SSE Stream Buffer for when client and upstream APIs match.
pub struct PassthroughStreamBuffer {
    /// Buffered SSE events ready to be written to wire
    buffered_events: Vec<SseEvent>,
}

impl Default for PassthroughStreamBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl PassthroughStreamBuffer {
    pub fn new() -> Self {
        Self {
            buffered_events: Vec::new(),
        }
    }
}

impl SseStreamBufferTrait for PassthroughStreamBuffer {
    fn add_transformed_event(&mut self, event: SseEvent) {
        // Skip ping messages
        if event.should_skip() {
            return;
        }

        // Skip events with empty transformed lines (e.g., suppressed event-only lines)
        if event.sse_transformed_lines.is_empty() {
            return;
        }

        // Just accumulate events as-is
        self.buffered_events.push(event);
    }

    fn to_bytes(&mut self) -> Vec<u8> {
        // No finalization needed for passthrough - just convert accumulated events to bytes
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
    use crate::apis::streaming_shapes::passthrough_streaming_buffer::PassthroughStreamBuffer;
    use crate::apis::streaming_shapes::sse::{SseStreamBufferTrait, SseStreamIter};

    #[test]
    fn test_chat_completions_passthrough_buffer() {
        let raw_input = r#"data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1234567890,"model":"gpt-4o","choices":[{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_abc","type":"function","function":{"name":"get_weather","arguments":""}}],"refusal":null},"logprobs":null,"finish_reason":null}]}

    data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1234567890,"model":"gpt-4o","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\""}}]},"logprobs":null,"finish_reason":null}]}

    data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1234567890,"model":"gpt-4o","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"location"}}]},"logprobs":null,"finish_reason":null}]}

    data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1234567890,"model":"gpt-4o","choices":[{"index":0,"delta":{},"logprobs":null,"finish_reason":"tool_calls"}]}

    data: [DONE]"#;

        println!("\n{}", "=".repeat(80));
        println!("TEST 1: ChatCompletions Passthrough Buffer");
        println!("{}", "=".repeat(80));
        println!("\nRAW INPUT (ChatCompletions):");
        println!("{}", "-".repeat(80));
        println!("{}", raw_input);

        // Parse and process through buffer
        let stream_iter = SseStreamIter::try_from(raw_input.as_bytes()).unwrap();
        let mut buffer = PassthroughStreamBuffer::new();

        for event in stream_iter {
            buffer.add_transformed_event(event);
        }

        let output_bytes = buffer.to_bytes();
        let output = String::from_utf8_lossy(&output_bytes);

        println!("\nTRANSFORMED OUTPUT (ChatCompletions - Passthrough):");
        println!("{}", "-".repeat(80));
        println!("{}", output);

        // Assertions
        assert!(!output_bytes.is_empty());
        assert!(output.contains("chatcmpl-123"));
        assert!(output.contains("[DONE]"));
        assert_eq!(
            raw_input.trim(),
            output.trim(),
            "Passthrough should preserve input"
        );

        println!("\nVALIDATION SUMMARY:");
        println!("{}", "-".repeat(80));
        println!("✓ Passthrough buffer: input = output (no transformation)");
        println!("✓ All events preserved including [DONE]");
        println!("✓ Function calling events preserved\n");
    }
}
