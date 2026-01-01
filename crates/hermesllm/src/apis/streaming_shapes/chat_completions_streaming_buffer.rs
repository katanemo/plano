use crate::apis::streaming_shapes::sse::{SseEvent, SseStreamBufferTrait};

///  OpenAI Chat Completions SSE Stream Buffer for when client and upstream APIs match.
pub struct OpenAIChatCompletionsStreamBuffer {
    /// Buffered SSE events ready to be written to wire
    buffered_events: Vec<SseEvent>,
}

impl Default for OpenAIChatCompletionsStreamBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenAIChatCompletionsStreamBuffer {
    pub fn new() -> Self {
        Self {
            buffered_events: Vec::new(),
        }
    }
}

impl SseStreamBufferTrait for OpenAIChatCompletionsStreamBuffer {
    fn add_transformed_event(&mut self, event: SseEvent) {
        // Skip ping messages
        if event.should_skip() {
            return;
        }

        // For OpenAI Chat Completions, events are already properly transformed
        // Just accumulate them for later wire transmission
        self.buffered_events.push(event);
    }

    fn to_bytes(&mut self) -> Vec<u8> {
        // No finalization needed for OpenAI Chat Completions
        // The [DONE] marker is already handled by the transformation layer
        let mut buffer = Vec::new();
        for event in self.buffered_events.drain(..) {
            let event_bytes: Vec<u8> = event.into();
            buffer.extend_from_slice(&event_bytes);
        }
        buffer
    }
}
