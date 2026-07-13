use crate::apis::streaming_shapes::sse::{is_incomplete_json_error, SseEvent, SseStreamIter};
use crate::clients::endpoints::{SupportedAPIsFromClient, SupportedUpstreamAPIs};

/// Determine whether a trailing partial line (the bytes after the final `\n` in
/// the working buffer) already forms a *complete* SSE event.
///
/// SSE lines are newline-delimited, so bytes after the last `\n` are normally an
/// incomplete line to be held back until the next chunk arrives. The one
/// exception is a final line that is genuinely complete but was sent without a
/// trailing newline (e.g. a terminal `data: [DONE]`). Nothing flushes the
/// processor at end-of-stream, so such a line must be parsed now rather than
/// buffered forever.
///
/// A line is "complete" when it parses as an SSE event and, for `data:` lines,
/// its JSON payload is itself fully parseable (or is the `[DONE]` sentinel). A
/// mid-prefix split (`da`), a mid-JSON split (`data: {"a":`), or a split inside
/// a multi-byte UTF-8 character all fail this check and are held back.
///
/// Soundness note: this relies on SSE data payloads being JSON *objects* (or
/// `[DONE]`), where no strict prefix of a longer payload can itself fully
/// parse. Bare scalar payloads (`data: 12` split from `data: 123`) would defeat
/// the check, but no supported provider emits scalar data lines.
fn trailing_line_is_complete(bytes: &[u8]) -> bool {
    let s = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        // Split inside a multi-byte UTF-8 char -> definitely incomplete.
        Err(_) => return false,
    };
    match s.parse::<SseEvent>() {
        Ok(event) => match &event.data {
            Some(data) => {
                data.trim() == "[DONE]" || serde_json::from_str::<serde_json::Value>(data).is_ok()
            }
            // event-only line (e.g. `event: foo`) with no data payload
            None => true,
        },
        Err(_) => false,
    }
}

/// Upper bound for bytes held back across chunks while waiting for the rest of
/// a partial SSE line. A compliant upstream terminates every event with `\n\n`
/// well before this size; the cap only exists so a broken or hostile upstream
/// streaming a newline-less blob cannot grow the buffer without bound.
///
/// Note: a single legitimate `data:` line larger than this (e.g. an inline
/// base64 image payload) would be dropped. Text/reasoning deltas are orders of
/// magnitude smaller; raise the cap if such payloads ever become a norm.
const MAX_INCOMPLETE_EVENT_BUFFER_BYTES: usize = 1024 * 1024;

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

        // Byte-level SSE framing. Only parse up to the last line terminator; any
        // bytes after the final `\n` form a partial line that may be split across
        // the chunk boundary (mid-prefix like `da`, mid-JSON, or even mid-UTF-8).
        // Hold those bytes back and prepend them to the next chunk. This fixes
        // both the mid-prefix split (which used to be silently dropped by
        // `SseStreamIter`) and the mid-JSON split generically, and it guarantees
        // the parsed prefix always ends on a `\n` boundary so `str::from_utf8`
        // never splits a multi-byte character.
        //
        // The single exception is a trailing line that is already a complete
        // event but arrived without its terminating newline (e.g. a final
        // `data: [DONE]`): parse it now, because nothing flushes the buffer at
        // end-of-stream.
        let parse_end = match combined_data.iter().rposition(|&b| b == b'\n') {
            Some(idx) => {
                let trailing = &combined_data[idx + 1..];
                if trailing.is_empty() || trailing_line_is_complete(trailing) {
                    combined_data.len()
                } else {
                    idx + 1
                }
            }
            None => {
                // No line terminator yet in the whole buffer.
                if trailing_line_is_complete(&combined_data) {
                    combined_data.len()
                } else {
                    0
                }
            }
        };

        let (parse_bytes, remainder) = combined_data.split_at(parse_end);
        let remainder = remainder.to_vec();

        if remainder.len() > MAX_INCOMPLETE_EVENT_BUFFER_BYTES {
            // Give up on ever completing this line rather than buffering
            // without bound; the caller falls back to forwarding raw bytes.
            self.incomplete_event_buffer.clear();
            return Err(format!(
                "incomplete SSE line exceeded {} byte buffer limit",
                MAX_INCOMPLETE_EVENT_BUFFER_BYTES
            ));
        }

        // Parse using SseStreamIter (only complete lines reach here)
        let sse_iter = match SseStreamIter::try_from(parse_bytes) {
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
                    if is_incomplete_json_error(&e) {
                        // Conservative fallback: with the framing above complete
                        // lines always carry complete JSON, so this should not
                        // trigger. If it ever does, buffer this line (plus the
                        // held-back remainder) for retry with the next chunk.
                        self.incomplete_event_buffer = sse_event.raw_line.as_bytes().to_vec();
                        self.incomplete_event_buffer.extend_from_slice(&remainder);
                        return Ok(transformed_events);
                    } else {
                        // Other error (unsupported event type, validation error, etc.)
                        // Skip this event and continue processing others
                        continue;
                    }
                }
            }
        }

        // Hold back the trailing partial line (if any) for the next chunk.
        self.incomplete_event_buffer = remainder;

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
    fn test_incomplete_buffer_growth_is_bounded() {
        let mut processor = SseChunkProcessor::new();
        let client_api = SupportedAPIsFromClient::OpenAIChatCompletions(OpenAIApi::ChatCompletions);
        let upstream_api = SupportedUpstreamAPIs::OpenAIChatCompletions(OpenAIApi::ChatCompletions);

        // Newline-less garbage that never completes a line: the processor must
        // give up with an error instead of buffering it forever.
        let garbage = vec![b'x'; 256 * 1024];
        let mut errored = false;
        for _ in 0..8 {
            match processor.process_chunk(&garbage, &client_api, &upstream_api) {
                Ok(events) => assert!(events.is_empty()),
                Err(_) => {
                    errored = true;
                    break;
                }
            }
        }
        assert!(errored, "unbounded garbage should eventually error");
        assert!(
            !processor.has_buffered_data(),
            "buffer should be cleared after exceeding the limit"
        );
    }

    // Captured GPT-5.6 Responses-API SSE stream (direct from backend, streams fine).
    // ALL ids/tokens are obfuscated to fake same-format values. JSON shapes are byte-faithful.
    const GPT56_RESPONSES_STREAM: &str = "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_0123456789abcdef0123456789abcdef0123456789abcdef01\",\"object\":\"response\",\"created_at\":1783941440,\"status\":\"in_progress\",\"background\":false,\"completed_at\":null,\"error\":null,\"frequency_penalty\":0.0,\"incomplete_details\":null,\"instructions\":\"Answer in one word.\",\"max_output_tokens\":null,\"max_tool_calls\":null,\"model\":\"gpt-5.6-sol\",\"moderation\":null,\"output\":[],\"parallel_tool_calls\":true,\"presence_penalty\":0.0,\"previous_response_id\":null,\"prompt_cache_key\":\"00000000-0000-4000-8000-000000000000\",\"prompt_cache_retention\":\"24h\",\"reasoning\":{\"context\":\"all_turns\",\"effort\":\"medium\",\"mode\":\"standard\",\"summary\":null},\"safety_identifier\":\"user-000000000000000000000000\",\"service_tier\":\"auto\",\"store\":false,\"temperature\":1.0,\"text\":{\"format\":{\"type\":\"text\"},\"verbosity\":\"medium\"},\"tool_choice\":\"auto\",\"tool_usage\":{\"image_gen\":{\"input_tokens\":0,\"input_tokens_details\":{\"image_tokens\":0,\"text_tokens\":0},\"output_tokens\":0,\"output_tokens_details\":{\"image_tokens\":0,\"text_tokens\":0},\"total_tokens\":0},\"web_search\":{\"num_requests\":0}},\"tools\":[],\"top_logprobs\":0,\"top_p\":0.98,\"truncation\":\"disabled\",\"usage\":null,\"user\":null,\"metadata\":{}},\"sequence_number\":0}\n\nevent: response.in_progress\ndata: {\"type\":\"response.in_progress\",\"response\":{\"id\":\"resp_0123456789abcdef0123456789abcdef0123456789abcdef01\",\"object\":\"response\",\"created_at\":1783941440,\"status\":\"in_progress\",\"background\":false,\"completed_at\":null,\"error\":null,\"frequency_penalty\":0.0,\"incomplete_details\":null,\"instructions\":\"Answer in one word.\",\"max_output_tokens\":null,\"max_tool_calls\":null,\"model\":\"gpt-5.6-sol\",\"moderation\":null,\"output\":[],\"parallel_tool_calls\":true,\"presence_penalty\":0.0,\"previous_response_id\":null,\"prompt_cache_key\":\"00000000-0000-4000-8000-000000000000\",\"prompt_cache_retention\":\"24h\",\"reasoning\":{\"context\":\"all_turns\",\"effort\":\"medium\",\"mode\":\"standard\",\"summary\":null},\"safety_identifier\":\"user-000000000000000000000000\",\"service_tier\":\"auto\",\"store\":false,\"temperature\":1.0,\"text\":{\"format\":{\"type\":\"text\"},\"verbosity\":\"medium\"},\"tool_choice\":\"auto\",\"tool_usage\":{\"image_gen\":{\"input_tokens\":0,\"input_tokens_details\":{\"image_tokens\":0,\"text_tokens\":0},\"output_tokens\":0,\"output_tokens_details\":{\"image_tokens\":0,\"text_tokens\":0},\"total_tokens\":0},\"web_search\":{\"num_requests\":0}},\"tools\":[],\"top_logprobs\":0,\"top_p\":0.98,\"truncation\":\"disabled\",\"usage\":null,\"user\":null,\"metadata\":{}},\"sequence_number\":1}\n\nevent: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"item\":{\"id\":\"rs_fedcba9876543210fedcba9876543210fedcba9876543210fe\",\"type\":\"reasoning\",\"content\":[],\"summary\":[]},\"output_index\":0,\"sequence_number\":2}\n\nevent: response.output_item.done\ndata: {\"type\":\"response.output_item.done\",\"item\":{\"id\":\"rs_fedcba9876543210fedcba9876543210fedcba9876543210fe\",\"type\":\"reasoning\",\"content\":[],\"summary\":[]},\"output_index\":0,\"sequence_number\":3}\n\nevent: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"item\":{\"id\":\"msg_1a2b3c4d5e6f78901a2b3c4d5e6f78901a2b3c4d5e6f789012\",\"type\":\"message\",\"status\":\"in_progress\",\"content\":[],\"phase\":\"final_answer\",\"role\":\"assistant\"},\"output_index\":1,\"sequence_number\":4}\n\nevent: response.content_part.added\ndata: {\"type\":\"response.content_part.added\",\"content_index\":0,\"item_id\":\"msg_1a2b3c4d5e6f78901a2b3c4d5e6f78901a2b3c4d5e6f789012\",\"output_index\":1,\"part\":{\"type\":\"output_text\",\"annotations\":[],\"logprobs\":[],\"text\":\"\"},\"sequence_number\":5}\n\nevent: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"content_index\":0,\"delta\":\"Hi\",\"item_id\":\"msg_1a2b3c4d5e6f78901a2b3c4d5e6f78901a2b3c4d5e6f789012\",\"logprobs\":[],\"obfuscation\":\"0000000000abcd\",\"output_index\":1,\"sequence_number\":6}\n\nevent: response.output_text.done\ndata: {\"type\":\"response.output_text.done\",\"content_index\":0,\"item_id\":\"msg_1a2b3c4d5e6f78901a2b3c4d5e6f78901a2b3c4d5e6f789012\",\"logprobs\":[],\"output_index\":1,\"sequence_number\":7,\"text\":\"Hi\"}\n\nevent: response.content_part.done\ndata: {\"type\":\"response.content_part.done\",\"content_index\":0,\"item_id\":\"msg_1a2b3c4d5e6f78901a2b3c4d5e6f78901a2b3c4d5e6f789012\",\"output_index\":1,\"part\":{\"type\":\"output_text\",\"annotations\":[],\"logprobs\":[],\"text\":\"Hi\"},\"sequence_number\":8}\n\nevent: response.output_item.done\ndata: {\"type\":\"response.output_item.done\",\"item\":{\"id\":\"msg_1a2b3c4d5e6f78901a2b3c4d5e6f78901a2b3c4d5e6f789012\",\"type\":\"message\",\"status\":\"completed\",\"content\":[{\"type\":\"output_text\",\"annotations\":[],\"logprobs\":[],\"text\":\"Hi\"}],\"phase\":\"final_answer\",\"role\":\"assistant\"},\"output_index\":1,\"sequence_number\":9}\n\nevent: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_0123456789abcdef0123456789abcdef0123456789abcdef01\",\"object\":\"response\",\"created_at\":1783941440,\"status\":\"completed\",\"background\":false,\"completed_at\":1783941441,\"error\":null,\"frequency_penalty\":0.0,\"incomplete_details\":null,\"instructions\":\"Answer in one word.\",\"max_output_tokens\":null,\"max_tool_calls\":null,\"model\":\"gpt-5.6-sol\",\"moderation\":null,\"output\":[],\"parallel_tool_calls\":true,\"presence_penalty\":0.0,\"previous_response_id\":null,\"prompt_cache_key\":\"00000000-0000-4000-8000-000000000000\",\"prompt_cache_retention\":\"24h\",\"reasoning\":{\"context\":\"all_turns\",\"effort\":\"medium\",\"mode\":\"standard\",\"summary\":null},\"safety_identifier\":\"user-000000000000000000000000\",\"service_tier\":\"default\",\"store\":false,\"temperature\":1.0,\"text\":{\"format\":{\"type\":\"text\"},\"verbosity\":\"medium\"},\"tool_choice\":\"auto\",\"tool_usage\":{\"image_gen\":{\"input_tokens\":0,\"input_tokens_details\":{\"image_tokens\":0,\"text_tokens\":0},\"output_tokens\":0,\"output_tokens_details\":{\"image_tokens\":0,\"text_tokens\":0},\"total_tokens\":0},\"web_search\":{\"num_requests\":0}},\"tools\":[],\"top_logprobs\":0,\"top_p\":0.98,\"truncation\":\"disabled\",\"usage\":{\"input_tokens\":17,\"input_tokens_details\":{\"cache_write_tokens\":0,\"cached_tokens\":0},\"output_tokens\":16,\"output_tokens_details\":{\"reasoning_tokens\":9},\"total_tokens\":33},\"user\":null,\"metadata\":{}},\"sequence_number\":10}\n\n";

    /// Regression test pinning the GPT-5.6 Responses-API identity passthrough
    /// (OpenAIResponsesAPI client <- OpenAIResponsesAPI upstream) streaming bug.
    ///
    /// The captured backend stream (`GPT56_RESPONSES_STREAM`) streams fine and, when
    /// processed as a single chunk, all 11 events emerge. But real SSE arrives in
    /// arbitrarily-fragmented chunks. This test replays the stream while shifting the
    /// 64-byte chunk boundaries across every possible byte position and asserts that
    /// every event survives regardless of where the chunk boundaries fall.
    ///
    /// It FAILS today: when a chunk boundary lands inside a `data: ` / `event: ` line
    /// prefix, `SseEvent::from_str` fails with a non-JSON error, `SseStreamIter`
    /// silently drops the partial line (it is never buffered as "incomplete JSON"),
    /// and the continuation on the next chunk is also unparseable -> the event is lost.
    #[test]
    fn test_gpt56_responses_identity_passthrough_full_stream() {
        use crate::apis::streaming_shapes::passthrough_streaming_buffer::PassthroughStreamBuffer;
        use crate::apis::streaming_shapes::sse::SseStreamBufferTrait;

        let client = SupportedAPIsFromClient::OpenAIResponsesAPI(OpenAIApi::Responses);
        let upstream = SupportedUpstreamAPIs::OpenAIResponsesAPI(OpenAIApi::Responses);

        // The 9 distinct SSE event types the client must observe.
        let expected_event_types = [
            "response.created",
            "response.in_progress",
            "response.output_item.added",
            "response.output_item.done",
            "response.content_part.added",
            "response.output_text.delta",
            "response.output_text.done",
            "response.content_part.done",
            "response.completed",
        ];

        let input = GPT56_RESPONSES_STREAM.as_bytes();

        // Baseline: processed as a single chunk, everything must survive (sanity).
        let baseline_len = {
            let mut proc = SseChunkProcessor::new();
            let mut buf = PassthroughStreamBuffer::new();
            for ev in proc.process_chunk(input, &client, &upstream).unwrap() {
                buf.add_transformed_event(ev);
            }
            buf.to_bytes().len()
        };
        assert!(
            baseline_len > 0,
            "baseline single-chunk output must be non-empty"
        );

        // Replay the stream for every boundary phase: start the fixed 64-byte chunking
        // at each `start_offset` in 0..64 so that, across all runs, every byte position
        // is exercised as a chunk boundary.
        for start_offset in 0..64usize {
            let mut proc = SseChunkProcessor::new();
            let mut buf = PassthroughStreamBuffer::new();

            let mut pos = 0usize;
            while pos < input.len() {
                let end = if pos == 0 {
                    start_offset.clamp(1, input.len())
                } else {
                    (pos + 64).min(input.len())
                };
                let chunk = &input[pos..end];
                for ev in proc.process_chunk(chunk, &client, &upstream).unwrap() {
                    buf.add_transformed_event(ev);
                }
                pos = end;
            }

            let out = buf.to_bytes();
            let out_str = String::from_utf8_lossy(&out);

            // Every event type must be present regardless of chunk boundary placement.
            for et in expected_event_types {
                assert!(
                    out_str.contains(et),
                    "start_offset={}: missing event type `{}` after chunked replay \
                     (a chunk boundary landed inside a `data:`/`event:` line and the event was dropped)\n\
                     output was:\n{}",
                    start_offset,
                    et,
                    out_str
                );
            }

            // Every sequence number 0..=10 must be present (no event silently dropped).
            for seq in 0..=10 {
                let needle = format!("\"sequence_number\":{}", seq);
                assert!(
                    out_str.contains(&needle),
                    "start_offset={}: missing sequence_number {} after chunked replay",
                    start_offset,
                    seq
                );
            }

            // No incomplete data should be left dangling once the stream ends.
            assert!(
                !proc.has_buffered_data(),
                "start_offset={}: processor still has {} buffered bytes at end of stream",
                start_offset,
                proc.buffered_size()
            );

            // Output must match the single-chunk baseline byte length (no loss / no dup).
            assert_eq!(
                out.len(),
                baseline_len,
                "start_offset={}: chunked output length {} != single-chunk baseline {}",
                start_offset,
                out.len(),
                baseline_len
            );
        }
    }

    /// Task A guard: a chunk boundary landing INSIDE the `data: ` line prefix
    /// (e.g. right after `da`) must not drop the event. The old code parsed each
    /// `str::lines()` line independently, `SseEvent::from_str` failed on the
    /// partial prefix with a non-JSON error, and the line was silently skipped.
    #[test]
    fn test_mid_prefix_split_event_survives() {
        let client = SupportedAPIsFromClient::OpenAIResponsesAPI(OpenAIApi::Responses);
        let upstream = SupportedUpstreamAPIs::OpenAIResponsesAPI(OpenAIApi::Responses);

        let full = "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"content_index\":0,\"delta\":\"Hi\",\"item_id\":\"msg_abc\",\"logprobs\":[],\"output_index\":1,\"sequence_number\":6}\n\n";

        // Split inside the `data: ` prefix, right after `da`.
        let split_at = full.find("data:").unwrap() + 2; // after "da"
        let (a, b) = full.as_bytes().split_at(split_at);

        let mut proc = SseChunkProcessor::new();
        let mut out = String::new();
        for ev in proc.process_chunk(a, &client, &upstream).unwrap() {
            out.push_str(&ev.sse_transformed_lines);
        }
        for ev in proc.process_chunk(b, &client, &upstream).unwrap() {
            out.push_str(&ev.sse_transformed_lines);
        }

        assert!(
            out.contains("\"sequence_number\":6"),
            "mid-prefix split dropped the event; output: {}",
            out
        );
        assert!(
            out.contains("event: response.output_text.delta"),
            "mid-prefix split lost the event line; output: {}",
            out
        );
        assert!(!proc.has_buffered_data(), "no data should remain buffered");
    }

    /// Task A guard: a chunk boundary landing inside the JSON payload must
    /// recombine across chunks and emit the event exactly once, intact.
    #[test]
    fn test_mid_json_split_recombines_once() {
        let client = SupportedAPIsFromClient::OpenAIResponsesAPI(OpenAIApi::Responses);
        let upstream = SupportedUpstreamAPIs::OpenAIResponsesAPI(OpenAIApi::Responses);

        let full = "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"content_index\":0,\"delta\":\"Hi\",\"item_id\":\"msg_abc\",\"logprobs\":[],\"output_index\":1,\"sequence_number\":6}\n\n";

        // Split inside the JSON object.
        let split_at = full.find("\"item_id\"").unwrap() + 4;
        let (a, b) = full.as_bytes().split_at(split_at);

        let mut proc = SseChunkProcessor::new();
        let mut out = String::new();
        let first = proc.process_chunk(a, &client, &upstream).unwrap();
        // The complete `event:` line may be emitted (it is suppressed to an empty
        // string on the identity path); the incomplete data line must be held
        // back for recombination.
        assert!(
            first.iter().all(|e| e.sse_transformed_lines.is_empty()),
            "incomplete data line must not be emitted yet"
        );
        assert!(
            proc.has_buffered_data(),
            "incomplete JSON data line must be buffered"
        );
        for ev in first {
            out.push_str(&ev.sse_transformed_lines);
        }
        for ev in proc.process_chunk(b, &client, &upstream).unwrap() {
            out.push_str(&ev.sse_transformed_lines);
        }

        assert_eq!(
            out.matches("\"sequence_number\":6").count(),
            1,
            "event must appear exactly once; output: {}",
            out
        );
        assert_eq!(
            out.matches("event: response.output_text.delta").count(),
            1,
            "event line must appear exactly once; output: {}",
            out
        );
        assert!(!proc.has_buffered_data());
    }

    /// Task B guard: an unmodeled Responses event type + unmodeled JSON fields
    /// must be forwarded verbatim on the identity Responses->Responses path,
    /// not dropped.
    #[test]
    fn test_unknown_event_forwarded_verbatim() {
        let client = SupportedAPIsFromClient::OpenAIResponsesAPI(OpenAIApi::Responses);
        let upstream = SupportedUpstreamAPIs::OpenAIResponsesAPI(OpenAIApi::Responses);

        // `response.reasoning_text.delta` is NOT a modeled ResponsesAPIStreamEvent
        // variant, and `mystery_field` is not modeled either.
        let data = "{\"type\":\"response.reasoning_text.delta\",\"delta\":\"pondering\",\"mystery_field\":123,\"sequence_number\":7}";
        let chunk = format!("event: response.reasoning_text.delta\ndata: {}\n\n", data);

        let mut proc = SseChunkProcessor::new();
        let mut out = String::new();
        for ev in proc
            .process_chunk(chunk.as_bytes(), &client, &upstream)
            .unwrap()
        {
            out.push_str(&ev.sse_transformed_lines);
        }

        assert!(
            out.contains("event: response.reasoning_text.delta"),
            "unknown event type must pass through; output: {}",
            out
        );
        assert!(
            out.contains("\"mystery_field\":123"),
            "unmodeled fields must be forwarded verbatim; output: {}",
            out
        );
        // The exact original data payload survives byte-for-byte.
        assert!(
            out.contains(data),
            "raw JSON must be verbatim; output: {}",
            out
        );
    }

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
