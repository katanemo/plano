use crate::apis::streaming_shapes::sse::{SseEvent, SseStreamIter};
use crate::clients::endpoints::{SupportedAPIsFromClient, SupportedUpstreamAPIs};

/// Stateful processor for handling SSE chunks that may contain incomplete events.
///
/// This processor buffers incomplete SSE event bytes when transformation fails
/// (e.g., due to incomplete JSON) and prepends them to the next chunk for retry.
pub struct SseChunkProcessor {
    /// Buffered bytes from incomplete SSE events across chunks
    incomplete_event_buffer: Vec<u8>,
}

impl Default for SseChunkProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl SseChunkProcessor {
    pub fn new() -> Self {
        Self {
            incomplete_event_buffer: Vec::new(),
        }
    }

    /// Process a chunk of SSE data, handling incomplete events across chunk boundaries.
    ///
    /// Returns successfully transformed events. Incomplete events are buffered internally
    /// and will be retried when more data arrives in the next chunk.
    ///
    /// # Arguments
    /// * `chunk` - Raw bytes from upstream SSE stream
    /// * `client_api` - The API format the client expects
    /// * `upstream_api` - The API format from the upstream provider
    ///
    /// # Returns
    /// * `Ok(Vec<SseEvent>)` - Successfully transformed events ready for client
    /// * `Err(String)` - Fatal error that cannot be recovered by buffering
    pub fn process_chunk(
        &mut self,
        chunk: &[u8],
        client_api: &SupportedAPIsFromClient,
        upstream_api: &SupportedUpstreamAPIs,
    ) -> Result<Vec<SseEvent>, String> {
        // Combine buffered incomplete event with new chunk
        let mut combined_data = std::mem::take(&mut self.incomplete_event_buffer);
        combined_data.extend_from_slice(chunk);

        // Parse using SseStreamIter
        let sse_iter = match SseStreamIter::try_from(combined_data.as_slice()) {
            Ok(iter) => iter,
            Err(e) => return Err(format!("Failed to create SSE iterator: {}", e)),
        };

        let mut transformed_events = Vec::new();

        // Process each parsed SSE event
        for sse_event in sse_iter {
            // Try to transform the event (this is where incomplete JSON fails)
            match SseEvent::try_from((sse_event.clone(), client_api, upstream_api)) {
                Ok(transformed) => {
                    // Successfully transformed - add to results
                    transformed_events.push(transformed);
                }
                Err(e) => {
                    // Check if this is incomplete JSON (EOF while parsing) vs other errors
                    let error_str = e.to_string().to_lowercase();
                    let is_incomplete_json = error_str.contains("eof while parsing")
                        || error_str.contains("unexpected end of json")
                        || error_str.contains("unexpected eof");

                    if is_incomplete_json {
                        // Incomplete JSON - buffer for retry with next chunk
                        self.incomplete_event_buffer = sse_event.raw_line.as_bytes().to_vec();
                        break;
                    } else {
                        // Other error (unsupported event type, validation error, etc.)
                        // Skip this event and continue processing others
                        continue;
                    }
                }
            }
        }

        Ok(transformed_events)
    }

    /// Check if there are buffered incomplete bytes
    pub fn has_buffered_data(&self) -> bool {
        !self.incomplete_event_buffer.is_empty()
    }

    /// Get the size of buffered incomplete data (for debugging/logging)
    pub fn buffered_size(&self) -> usize {
        self.incomplete_event_buffer.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apis::openai::OpenAIApi;
    use crate::clients::endpoints::{SupportedAPIsFromClient, SupportedUpstreamAPIs};

    #[test]
    fn test_complete_events_process_immediately() {
        let mut processor = SseChunkProcessor::new();
        let client_api = SupportedAPIsFromClient::OpenAIChatCompletions(OpenAIApi::ChatCompletions);
        let upstream_api = SupportedUpstreamAPIs::OpenAIChatCompletions(OpenAIApi::ChatCompletions);

        let chunk1 = b"data: {\"id\":\"chatcmpl-123\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n\n";

        let events = processor
            .process_chunk(chunk1, &client_api, &upstream_api)
            .unwrap();

        assert_eq!(events.len(), 1);
        assert!(!processor.has_buffered_data());
    }

    #[test]
    fn test_incomplete_json_buffered_and_completed() {
        let mut processor = SseChunkProcessor::new();
        let client_api = SupportedAPIsFromClient::OpenAIChatCompletions(OpenAIApi::ChatCompletions);
        let upstream_api = SupportedUpstreamAPIs::OpenAIChatCompletions(OpenAIApi::ChatCompletions);

        // First chunk with incomplete JSON
        let chunk1 = b"data: {\"id\":\"chatcmpl-123\",\"object\":\"chat.completion.chu";

        let events1 = processor
            .process_chunk(chunk1, &client_api, &upstream_api)
            .unwrap();

        assert_eq!(events1.len(), 0, "Incomplete event should not be processed");
        assert!(
            processor.has_buffered_data(),
            "Incomplete data should be buffered"
        );

        // Second chunk completes the JSON
        let chunk2 = b"nk\",\"created\":1234567890,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n\n";

        let events2 = processor
            .process_chunk(chunk2, &client_api, &upstream_api)
            .unwrap();

        assert_eq!(events2.len(), 1, "Complete event should be processed");
        assert!(
            !processor.has_buffered_data(),
            "Buffer should be cleared after completion"
        );
    }

    #[test]
    fn test_multiple_events_with_one_incomplete() {
        let mut processor = SseChunkProcessor::new();
        let client_api = SupportedAPIsFromClient::OpenAIChatCompletions(OpenAIApi::ChatCompletions);
        let upstream_api = SupportedUpstreamAPIs::OpenAIChatCompletions(OpenAIApi::ChatCompletions);

        // Chunk with 2 complete events and 1 incomplete
        let chunk = b"data: {\"id\":\"chatcmpl-123\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"A\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"chatcmpl-124\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"B\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"chatcmpl-125\",\"object\":\"chat.completion.chu";

        let events = processor
            .process_chunk(chunk, &client_api, &upstream_api)
            .unwrap();

        assert_eq!(events.len(), 2, "Two complete events should be processed");
        assert!(
            processor.has_buffered_data(),
            "Incomplete third event should be buffered"
        );
    }

    #[test]
    fn test_anthropic_signature_delta_from_production_logs() {
        use crate::apis::anthropic::AnthropicApi;

        let mut processor = SseChunkProcessor::new();
        let client_api = SupportedAPIsFromClient::AnthropicMessagesAPI(AnthropicApi::Messages);
        let upstream_api = SupportedUpstreamAPIs::AnthropicMessagesAPI(AnthropicApi::Messages);

        // Exact chunk from production logs - signature_delta event followed by content_block_stop
        let chunk = br#"event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"ErECCkYIChgCKkC7lAf/BOatd0I4NnANYNEDKl5/WSsjNK44AETnLoy3i5FfdYMAb0m4qMLJD6A04QnM4Hf3VpGqq/snA/9vvNxCEgw3CYcHcj0aTdqOisQaDOhlVBtAUKkoh3WopSIwAbJp4jG/41vVWBj63eaR7KFJ37OdY1byjlPkaGDUJRcWc/YfUWIDSAToomq2fB4VKpgBk+swVYxLZ709gQvyTCT+3vO/I+yexZpkx6eBl/+YCgQXTeviZ+hTxSoPVayf5vEQoc19ZA4MEkZ7yBInRgk8vUxAJITSf+vOvDIBsElpgkLfSjARCasjh78wONg39AkAoIbKzU+Q2l1htUwXcqQ2b+b5DrY9+Oxae4pBVGQlWU36XAHsa/KG+ejfdwhWJM7FNL3uphwAf0oYAQ=="}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

"#;

        let result = processor.process_chunk(chunk, &client_api, &upstream_api);

        match result {
            Ok(events) => {
                println!("Successfully processed {} events", events.len());
                for (i, event) in events.iter().enumerate() {
                    println!(
                        "Event {}: event={:?}, has_data={}",
                        i,
                        event.event,
                        event.data.is_some()
                    );
                }
                // Should successfully process both events (signature_delta + content_block_stop)
                assert!(
                    events.len() >= 2,
                    "Should process at least 2 complete events (signature_delta + stop), got {}",
                    events.len()
                );
                assert!(
                    !processor.has_buffered_data(),
                    "Complete events should not be buffered"
                );
            }
            Err(e) => {
                panic!("Failed to process signature_delta chunk - this means SignatureDelta is not properly handled: {}", e);
            }
        }
    }

    #[test]
    fn test_unsupported_event_does_not_block_subsequent_events() {
        let mut processor = SseChunkProcessor::new();
        let client_api = SupportedAPIsFromClient::OpenAIChatCompletions(OpenAIApi::ChatCompletions);
        let upstream_api = SupportedUpstreamAPIs::OpenAIChatCompletions(OpenAIApi::ChatCompletions);

        // Chunk with an unsupported/invalid event followed by a valid event
        // First event has invalid JSON structure that will fail validation (not incomplete)
        // Second event is valid and should be processed
        let chunk = b"data: {\"id\":\"chatcmpl-123\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"unsupported_field_causing_validation_error\":true},\"finish_reason\":null}]}\n\ndata: {\"id\":\"chatcmpl-124\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n\n";

        let events = processor
            .process_chunk(chunk, &client_api, &upstream_api)
            .unwrap();

        // Should skip the invalid event and process the valid one
        // (If we were buffering all errors, we'd get 0 events and have buffered data)
        assert!(
            !events.is_empty(),
            "Should process at least the valid event, got {} events",
            events.len()
        );
        assert!(
            !processor.has_buffered_data(),
            "Invalid (non-incomplete) events should not be buffered"
        );
    }

    #[test]
    fn test_unknown_delta_type_skipped_others_processed() {
        use crate::apis::anthropic::AnthropicApi;

        let mut processor = SseChunkProcessor::new();
        let client_api = SupportedAPIsFromClient::AnthropicMessagesAPI(AnthropicApi::Messages);
        let upstream_api = SupportedUpstreamAPIs::AnthropicMessagesAPI(AnthropicApi::Messages);

        // Chunk with valid event, unsupported delta type, then another valid event
        // This simulates a future API change where Anthropic adds a new delta type we don't support yet
        let chunk = br#"event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"future_unsupported_delta","future_field":"some_value"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" World"}}

"#;

        let result = processor.process_chunk(chunk, &client_api, &upstream_api);

        match result {
            Ok(events) => {
                println!(
                    "Processed {} events (unsupported event should be skipped)",
                    events.len()
                );
                // Should process the 2 valid text_delta events and skip the unsupported one
                // We expect at least 2 events (the valid ones), unsupported should be skipped
                assert!(
                    events.len() >= 2,
                    "Should process at least 2 valid events, got {}",
                    events.len()
                );
                assert!(
                    !processor.has_buffered_data(),
                    "Unsupported events should be skipped, not buffered"
                );
            }
            Err(e) => {
                panic!(
                    "Should not fail on unsupported delta type, should skip it: {}",
                    e
                );
            }
        }
    }
}
