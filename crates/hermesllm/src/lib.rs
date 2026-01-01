//! hermesllm: A library for translating LLM API requests and responses
//! between Mistral, Grok, Gemini, and OpenAI-compliant formats.

pub mod apis;
pub mod clients;
pub mod providers;
pub mod transforms;
// Re-export important types and traits
pub use apis::streaming_shapes::amazon_bedrock_binary_frame::BedrockBinaryFrameDecoder;
pub use apis::streaming_shapes::sse::{SseEvent, SseStreamIter};
pub use aws_smithy_eventstream::frame::DecodedFrame;
pub use providers::id::ProviderId;
pub use providers::request::{ProviderRequest, ProviderRequestError, ProviderRequestType};
pub use providers::response::{
    ProviderResponse, ProviderResponseError, ProviderResponseType, TokenUsage,
};
pub use providers::streaming_response::{ProviderStreamResponse, ProviderStreamResponseType};

//TODO: Refactor such that commons doesn't depend on Hermes. For now this will clean up strings
pub const CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";
pub const OPENAI_RESPONSES_API_PATH: &str = "/v1/responses";
pub const MESSAGES_PATH: &str = "/v1/messages";

#[cfg(test)]
mod tests {
    use crate::clients::endpoints::SupportedUpstreamAPIs;

    use super::*;

    #[test]
    fn test_provider_id_conversion() {
        assert_eq!(ProviderId::from("openai"), ProviderId::OpenAI);
        assert_eq!(ProviderId::from("mistral"), ProviderId::Mistral);
        assert_eq!(ProviderId::from("groq"), ProviderId::Groq);
        assert_eq!(ProviderId::from("arch"), ProviderId::Arch);
    }

    #[test]
    fn test_provider_streaming_response() {
        // Test streaming response parsing with sample SSE data
        let sse_data = r#"data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1694268190,"model":"gpt-4","choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"},"finish_reason":null}]}

    data: [DONE]
    "#;

        use crate::clients::endpoints::SupportedAPIsFromClient;
        let client_api =
            SupportedAPIsFromClient::OpenAIChatCompletions(crate::apis::OpenAIApi::ChatCompletions);
        let upstream_api =
            SupportedUpstreamAPIs::OpenAIChatCompletions(crate::apis::OpenAIApi::ChatCompletions);

        // Test the new simplified architecture - create SseStreamIter directly
        let sse_iter = SseStreamIter::try_from(sse_data.as_bytes());
        assert!(sse_iter.is_ok());

        let mut streaming_iter = sse_iter.unwrap();

        // Test that we can iterate over SseEvents
        let first_event = streaming_iter.next();
        assert!(first_event.is_some());

        let sse_event = first_event.unwrap();

        // Test SseEvent properties
        assert!(!sse_event.is_done());
        assert!(sse_event.data.as_ref().unwrap().contains("Hello"));

        // Test that we can parse the event into a provider stream response
        let transformed_event = SseEvent::try_from((sse_event, &client_api, &upstream_api));
        if let Err(e) = &transformed_event {
            println!("Transform error: {:?}", e);
        }
        assert!(transformed_event.is_ok());

        let transformed_event = transformed_event.unwrap();
        let provider_response = transformed_event.provider_response();
        assert!(provider_response.is_ok());

        let stream_response = provider_response.unwrap();
        assert_eq!(stream_response.content_delta(), Some("Hello"));
        assert!(!stream_response.is_final());

        // Test that stream ends properly with [DONE]
        // The iterator should return the [DONE] event, then None
        let done_event = streaming_iter.next();
        assert!(done_event.is_some(), "Should get [DONE] event");
        let done_event = done_event.unwrap();
        assert!(
            done_event.is_done(),
            "[DONE] event should be marked as done"
        );

        // After [DONE], iterator should return None
        let final_event = streaming_iter.next();
        assert!(
            final_event.is_none(),
            "Iterator should return None after [DONE]"
        );
    }

    /// Test AWS Event Stream decoding for Bedrock ConverseStream responses.
    ///
    /// This test demonstrates how to:
    /// 1. Use MessageFrameDecoder to decode AWS Event Stream frames
    /// 2. Handle chunked network arrivals with buffering
    /// 3. Extract event types from message headers
    /// 4. Parse JSON payloads from decoded messages
    /// 5. Reconstruct streaming content from contentBlockDelta events
    ///
    /// The decoder handles frame boundaries automatically - you just keep calling
    /// decode_frame() until it returns Incomplete, which means you've processed
    /// all complete frames in the buffer.
    #[test]
    fn test_amazon_bedrock_streaming_response() {
        use aws_smithy_eventstream::frame::{DecodedFrame, MessageFrameDecoder};
        use bytes::{Buf, BytesMut};
        use std::fs;
        use std::path::PathBuf;

        // Read the response.hex file from tests/e2e directory
        // Use absolute path to avoid cargo test working directory issues
        let test_file =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/e2e/response.hex");
        let response_data = fs::read(&test_file)
            .unwrap_or_else(|e| panic!("Failed to read {:?}: {}", test_file, e));

        println!("📊 Response data size: {} bytes\n", response_data.len());

        // Create decoder and buffer that implements Buf trait
        // BytesMut automatically tracks position as decoder advances it!
        let mut decoder = MessageFrameDecoder::new();
        let mut simulated_network_buffer = BytesMut::new();
        let mut frame_count = 0;
        let mut content_chunks = Vec::new();

        // Simulate chunked network arrivals - process as data comes in
        let chunk_sizes = [50, 100, 75, 200, 150, 300, 500, 1000];
        let mut offset = 0;
        let mut chunk_num = 0;

        println!("🔄 Simulating chunked network arrivals...\n");

        // Process chunks as they "arrive" from the network
        while offset < response_data.len() {
            // Receive next chunk from network
            let chunk_size = chunk_sizes[chunk_num % chunk_sizes.len()];
            let end = (offset + chunk_size).min(response_data.len());
            let chunk = &response_data[offset..end];

            chunk_num += 1;
            simulated_network_buffer.extend_from_slice(chunk);
            offset = end;

            println!(
                "📦 Chunk {}: Received {} bytes (buffer: {} bytes total, {} bytes remaining)",
                chunk_num,
                chunk.len(),
                simulated_network_buffer.len(),
                simulated_network_buffer.remaining()
            );

            // Try to decode all complete frames from buffer
            // The Buf trait tracks position automatically!
            loop {
                let bytes_before = simulated_network_buffer.remaining();
                match decoder.decode_frame(&mut simulated_network_buffer) {
                    Ok(DecodedFrame::Complete(message)) => {
                        frame_count += 1;
                        let consumed = bytes_before - simulated_network_buffer.remaining();

                        println!(
                            "  ✅ Frame {}: decoded ({} bytes, {} bytes remaining)",
                            frame_count,
                            consumed,
                            simulated_network_buffer.remaining()
                        );

                        // Get event type from headers
                        let event_type = message
                            .headers()
                            .iter()
                            .find(|h| h.name().as_str() == ":event-type")
                            .and_then(|h| {
                                h.value().as_string().ok().map(|s| s.as_str().to_string())
                            });

                        if let Some(ref evt) = event_type {
                            println!("  Event: {}", evt);
                        }

                        // Parse payload and extract content
                        let payload = message.payload();
                        if !payload.is_empty() {
                            if let Ok(json) = serde_json::from_slice::<serde_json::Value>(payload) {
                                if event_type.as_deref() == Some("contentBlockDelta") {
                                    if let Some(delta) = json.get("delta") {
                                        if let Some(text) =
                                            delta.get("text").and_then(|t| t.as_str())
                                        {
                                            println!("     📝 Content: \"{}\"", text);
                                            content_chunks.push(text.to_string());
                                        }
                                    }
                                }
                            }
                        } // Continue loop to check for more complete frames in buffer
                    }
                    Ok(DecodedFrame::Incomplete) => {
                        // Not enough data for a complete frame - need more chunks
                        println!(
                            "  ⏳ Incomplete frame ({} bytes remaining) - waiting for more data\n",
                            simulated_network_buffer.remaining()
                        );
                        break; // Wait for next chunk
                    }
                    Err(e) => {
                        panic!("❌ Frame decode error: {}", e);
                    }
                }
            }
        }

        println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!("📋 Summary:");
        println!("  Total chunks received: {}", chunk_num);
        println!("  Total frames decoded: {}", frame_count);
        println!("  Total content chunks: {}", content_chunks.len());
        println!(
            "  Final buffer remaining: {} bytes",
            simulated_network_buffer.remaining()
        );

        if !content_chunks.is_empty() {
            let full_text = content_chunks.join("");
            println!("\n📄 Full reconstructed content:");
            println!("{}", full_text);
            println!("\n  Characters: {}", full_text.len());
            println!("  Estimated tokens: ~{}", full_text.len() / 4);
        }

        // Ensure we decoded at least one frame
        assert!(frame_count > 0, "Should decode at least one frame");

        // Ensure all data was consumed - if buffer has remaining bytes, it's a partial frame
        assert_eq!(
            simulated_network_buffer.remaining(),
            0,
            "All bytes should be consumed, {} bytes remain",
            simulated_network_buffer.remaining()
        );
    }
}
