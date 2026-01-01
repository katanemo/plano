use crate::apis::openai_responses::{
    OutputItem, OutputItemStatus, Reasoning, ResponseStatus, ResponsesAPIResponse,
    ResponsesAPIStreamEvent, TextConfig, TextFormat,
};
use crate::apis::streaming_shapes::sse::{SseEvent, SseStreamBufferTrait};
use log::debug;
use std::collections::HashMap;

/// Helper to convert ResponseAPIStreamEvent to SseEvent
fn event_to_sse(event: ResponsesAPIStreamEvent) -> SseEvent {
    let event_type = match &event {
        ResponsesAPIStreamEvent::ResponseCreated { .. } => "response.created",
        ResponsesAPIStreamEvent::ResponseInProgress { .. } => "response.in_progress",
        ResponsesAPIStreamEvent::ResponseCompleted { .. } => "response.completed",
        ResponsesAPIStreamEvent::ResponseOutputItemAdded { .. } => "response.output_item.added",
        ResponsesAPIStreamEvent::ResponseOutputItemDone { .. } => "response.output_item.done",
        ResponsesAPIStreamEvent::ResponseOutputTextDelta { .. } => "response.output_text.delta",
        ResponsesAPIStreamEvent::ResponseOutputTextDone { .. } => "response.output_text.done",
        ResponsesAPIStreamEvent::ResponseFunctionCallArgumentsDelta { .. } => {
            "response.function_call_arguments.delta"
        }
        ResponsesAPIStreamEvent::ResponseFunctionCallArgumentsDone { .. } => {
            "response.function_call_arguments.done"
        }
        unknown => {
            debug!(
                "Unknown ResponsesAPIStreamEvent type encountered: {:?}",
                unknown
            );
            "unknown"
        }
    };

    let json_data = match serde_json::to_string(&event) {
        Ok(data) => data,
        Err(e) => {
            debug!("Error serializing ResponsesAPIStreamEvent to JSON: {}", e);
            String::new()
        }
    };
    let wire_format: String = event.into();

    SseEvent {
        data: Some(json_data),
        event: Some(event_type.to_string()),
        raw_line: wire_format.clone(),
        sse_transformed_lines: wire_format,
        provider_stream_response: None,
    }
}

/// SSE Stream Buffer for ResponsesAPIStreamEvent with full lifecycle management.
///
/// This buffer manages the wire format for v1/responses streaming, handling
/// delta events and emitting complete lifecycle events.
///
pub struct ResponsesAPIStreamBuffer {
    /// Sequence number for events
    sequence_number: i32,

    /// Track item IDs by output index
    item_ids: HashMap<i32, String>,

    /// Response metadata
    response_id: Option<String>,
    model: Option<String>,
    created_at: Option<i64>,

    /// Full response metadata from upstream (tools, temperature, etc.)
    /// This is extracted from the first upstream event and used to build
    /// complete response.created and response.in_progress events
    upstream_response_metadata: Option<ResponsesAPIResponse>,

    /// Lifecycle state flags
    created_emitted: bool,
    in_progress_emitted: bool,

    /// Track which output items we've added
    output_items_added: HashMap<i32, String>, // output_index -> item_id

    /// Accumulated content by item_id
    text_content: HashMap<String, String>,
    function_arguments: HashMap<String, String>,

    /// Tool call metadata by output_index
    tool_call_metadata: HashMap<i32, (String, String)>, // output_index -> (call_id, name)

    /// Final completed response (for logging/tracing/persistence)
    completed_response: Option<ResponsesAPIResponse>,

    /// Buffered SSE events ready to be written to wire
    buffered_events: Vec<SseEvent>,
}

impl Default for ResponsesAPIStreamBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl ResponsesAPIStreamBuffer {
    pub fn new() -> Self {
        Self {
            sequence_number: 0,
            item_ids: HashMap::new(),
            response_id: None,
            model: None,
            created_at: None,
            upstream_response_metadata: None,
            created_emitted: false,
            in_progress_emitted: false,
            output_items_added: HashMap::new(),
            text_content: HashMap::new(),
            function_arguments: HashMap::new(),
            tool_call_metadata: HashMap::new(),
            completed_response: None,
            buffered_events: Vec::new(),
        }
    }

    fn next_sequence_number(&mut self) -> i32 {
        let seq = self.sequence_number;
        self.sequence_number += 1;
        seq
    }

    fn generate_item_id(prefix: &str) -> String {
        format!(
            "{}_{}",
            prefix,
            uuid::Uuid::new_v4().to_string().replace("-", "")
        )
    }

    fn get_or_create_item_id(&mut self, output_index: i32, prefix: &str) -> String {
        if let Some(id) = self.item_ids.get(&output_index) {
            return id.clone();
        }
        let id = ResponsesAPIStreamBuffer::generate_item_id(prefix);
        self.item_ids.insert(output_index, id.clone());
        id
    }

    /// Create response.created event
    fn create_response_created_event(&mut self) -> SseEvent {
        let response = self.build_response(ResponseStatus::InProgress);
        let event = ResponsesAPIStreamEvent::ResponseCreated {
            response,
            sequence_number: self.next_sequence_number(),
        };
        event_to_sse(event)
    }

    /// Create response.in_progress event
    fn create_response_in_progress_event(&mut self) -> SseEvent {
        let response = self.build_response(ResponseStatus::InProgress);
        let event = ResponsesAPIStreamEvent::ResponseInProgress {
            response,
            sequence_number: self.next_sequence_number(),
        };
        event_to_sse(event)
    }

    /// Create output_item.added event for text
    fn create_output_item_added_event(&mut self, output_index: i32, item_id: &str) -> SseEvent {
        let event = ResponsesAPIStreamEvent::ResponseOutputItemAdded {
            output_index,
            item: OutputItem::Message {
                id: item_id.to_string(),
                status: OutputItemStatus::InProgress,
                role: "assistant".to_string(),
                content: vec![],
            },
            sequence_number: self.next_sequence_number(),
        };
        event_to_sse(event)
    }

    /// Create output_item.added event for tool call
    fn create_tool_call_added_event(
        &mut self,
        output_index: i32,
        item_id: &str,
        call_id: &str,
        name: &str,
    ) -> SseEvent {
        let event = ResponsesAPIStreamEvent::ResponseOutputItemAdded {
            output_index,
            item: OutputItem::FunctionCall {
                id: item_id.to_string(),
                status: OutputItemStatus::InProgress,
                call_id: call_id.to_string(),
                name: Some(name.to_string()),
                arguments: Some(String::new()),
            },
            sequence_number: self.next_sequence_number(),
        };
        event_to_sse(event)
    }

    /// Build the base response object with current state
    fn build_response(&self, status: ResponseStatus) -> ResponsesAPIResponse {
        // If we have upstream metadata, use it as a base and update status/output
        if let Some(upstream) = &self.upstream_response_metadata {
            let mut response = upstream.clone();
            response.status = status;
            // Don't update output here - will be set in finalize()
            return response;
        }

        // Fallback: build a minimal response from local state
        ResponsesAPIResponse {
            id: self.response_id.clone().unwrap_or_default(),
            object: "response".to_string(),
            created_at: self.created_at.unwrap_or(0),
            status,
            error: None,
            incomplete_details: None,
            instructions: None,
            model: self.model.clone().unwrap_or_else(|| "unknown".to_string()),
            output: vec![],
            usage: None,
            parallel_tool_calls: true,
            conversation: None,
            previous_response_id: None,
            tools: vec![],
            tool_choice: "auto".to_string(),
            temperature: 1.0,
            top_p: 1.0,
            metadata: HashMap::new(),
            truncation: Some("disabled".to_string()),
            max_output_tokens: None,
            reasoning: Some(Reasoning {
                effort: None,
                summary: None,
            }),
            store: Some(true),
            text: Some(TextConfig {
                format: TextFormat::Text,
            }),
            audio: None,
            modalities: None,
            service_tier: Some("auto".to_string()),
            background: Some(false),
            top_logprobs: Some(0),
            max_tool_calls: None,
        }
    }

    /// Get the completed response after finalization (for logging/tracing/persistence)
    pub fn get_completed_response(&self) -> Option<&ResponsesAPIResponse> {
        self.completed_response.as_ref()
    }

    /// Finalize the response by emitting all *.done events and response.completed.
    /// Call this when the stream is complete (after seeing [DONE] or end_of_stream).
    pub fn finalize(&mut self) {
        let mut events = Vec::new();

        // Emit done events for all accumulated content

        // Text content done events
        let text_items: Vec<_> = self
            .text_content
            .iter()
            .map(|(id, content)| (id.clone(), content.clone()))
            .collect();
        for (item_id, content) in text_items {
            let output_index = self
                .output_items_added
                .iter()
                .find(|(_, id)| **id == item_id)
                .map(|(idx, _)| *idx)
                .unwrap_or(0);

            let seq1 = self.next_sequence_number();
            let text_done_event = ResponsesAPIStreamEvent::ResponseOutputTextDone {
                item_id: item_id.clone(),
                output_index,
                content_index: 0,
                text: content.clone(),
                logprobs: vec![],
                sequence_number: seq1,
            };
            events.push(event_to_sse(text_done_event));

            let seq2 = self.next_sequence_number();
            let item_done_event = ResponsesAPIStreamEvent::ResponseOutputItemDone {
                output_index,
                item: OutputItem::Message {
                    id: item_id.clone(),
                    status: OutputItemStatus::Completed,
                    role: "assistant".to_string(),
                    content: vec![],
                },
                sequence_number: seq2,
            };
            events.push(event_to_sse(item_done_event));
        }

        // Function call done events
        let func_items: Vec<_> = self
            .function_arguments
            .iter()
            .map(|(id, args)| (id.clone(), args.clone()))
            .collect();
        for (item_id, arguments) in func_items {
            let output_index = self
                .output_items_added
                .iter()
                .find(|(_, id)| **id == item_id)
                .map(|(idx, _)| *idx)
                .unwrap_or(0);

            let seq1 = self.next_sequence_number();
            let args_done_event = ResponsesAPIStreamEvent::ResponseFunctionCallArgumentsDone {
                output_index,
                item_id: item_id.clone(),
                arguments: arguments.clone(),
                sequence_number: seq1,
            };
            events.push(event_to_sse(args_done_event));

            let (call_id, name) = self
                .tool_call_metadata
                .get(&output_index)
                .cloned()
                .unwrap_or_else(|| {
                    (
                        format!("call_{}", uuid::Uuid::new_v4()),
                        "unknown".to_string(),
                    )
                });

            let seq2 = self.next_sequence_number();
            let item_done_event = ResponsesAPIStreamEvent::ResponseOutputItemDone {
                output_index,
                item: OutputItem::FunctionCall {
                    id: item_id.clone(),
                    status: OutputItemStatus::Completed,
                    call_id,
                    name: Some(name),
                    arguments: Some(arguments.clone()),
                },
                sequence_number: seq2,
            };
            events.push(event_to_sse(item_done_event));
        }

        // Build final response
        let mut output_items = Vec::new();

        // Build complete output array by iterating through all output indices in order
        let max_output_index = self.output_items_added.keys().max().copied().unwrap_or(-1);

        for output_index in 0..=max_output_index {
            if let Some(item_id) = self.output_items_added.get(&output_index) {
                // Check if this is a function call
                if let Some(arguments) = self.function_arguments.get(item_id) {
                    let (call_id, name) = self
                        .tool_call_metadata
                        .get(&output_index)
                        .cloned()
                        .unwrap_or_else(|| {
                            (
                                format!("call_{}", uuid::Uuid::new_v4()),
                                "unknown".to_string(),
                            )
                        });

                    output_items.push(OutputItem::FunctionCall {
                        id: item_id.clone(),
                        status: OutputItemStatus::Completed,
                        call_id,
                        name: Some(name),
                        arguments: Some(arguments.clone()),
                    });
                }
                // Check if this is a text message
                else if let Some(text) = self.text_content.get(item_id) {
                    use crate::apis::openai_responses::OutputContent;
                    output_items.push(OutputItem::Message {
                        id: item_id.clone(),
                        status: OutputItemStatus::Completed,
                        role: "assistant".to_string(),
                        content: vec![OutputContent::OutputText {
                            text: text.clone(),
                            annotations: vec![],
                            logprobs: None,
                        }],
                    });
                }
            }
        }

        let mut final_response = self.build_response(ResponseStatus::Completed);
        final_response.output = output_items;

        // Store completed response
        self.completed_response = Some(final_response.clone());

        // Emit response.completed
        let seq_final = self.next_sequence_number();
        let completed_event = ResponsesAPIStreamEvent::ResponseCompleted {
            response: final_response,
            sequence_number: seq_final,
        };
        events.push(event_to_sse(completed_event));

        // Add all finalization events to the buffer
        self.buffered_events.extend(events);
    }
}

impl SseStreamBufferTrait for ResponsesAPIStreamBuffer {
    fn add_transformed_event(&mut self, event: SseEvent) {
        // Skip ping messages
        if event.should_skip() {
            return;
        }

        // Handle [DONE] marker - trigger finalization
        if event.is_done() {
            self.finalize();
            return;
        }

        // Extract the ResponseAPIStreamEvent from the SseEvent's provider_stream_response
        let provider_response = match event.provider_stream_response.as_ref() {
            Some(response) => response,
            None => {
                eprintln!("Warning: Event missing provider_stream_response");
                return;
            }
        };

        // Extract ResponseAPIStreamEvent from the enum
        let stream_event = match provider_response {
            crate::providers::streaming_response::ProviderStreamResponseType::ResponseAPIStreamEvent(evt) => evt,
            _ => {
                eprintln!("Warning: Expected ResponseAPIStreamEvent in provider_stream_response");
                return;
            }
        };

        let mut events = Vec::new();

        // Capture upstream metadata from ResponseCreated or ResponseInProgress if present
        match stream_event.as_ref() {
            ResponsesAPIStreamEvent::ResponseCreated { response, .. }
            | ResponsesAPIStreamEvent::ResponseInProgress { response, .. } => {
                if self.upstream_response_metadata.is_none() {
                    // Store the full upstream response as our metadata template
                    self.upstream_response_metadata = Some(response.clone());
                    // Also extract basic fields
                    self.response_id = Some(response.id.clone());
                    self.model = Some(response.model.clone());
                    self.created_at = Some(response.created_at);
                }
                // Don't emit these - we'll generate our own lifecycle events
                return;
            }
            _ => {}
        }

        // Emit lifecycle events if not yet emitted
        if !self.created_emitted {
            // Initialize metadata from first event if needed
            if self.response_id.is_none() {
                self.response_id = Some(format!(
                    "resp_{}",
                    uuid::Uuid::new_v4().to_string().replace("-", "")
                ));
                self.created_at = Some(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs() as i64,
                );
                self.model = Some("unknown".to_string()); // Will be set by caller if available
            }

            events.push(self.create_response_created_event());
            self.created_emitted = true;
        }

        if !self.in_progress_emitted {
            events.push(self.create_response_in_progress_event());
            self.in_progress_emitted = true;
        }

        // Process the delta event
        match stream_event.as_ref() {
            ResponsesAPIStreamEvent::ResponseOutputTextDelta {
                output_index,
                delta,
                ..
            } => {
                let item_id = self.get_or_create_item_id(*output_index, "msg");

                // Emit output_item.added if this is the first time we see this output index
                if !self.output_items_added.contains_key(output_index) {
                    self.output_items_added
                        .insert(*output_index, item_id.clone());
                    events.push(self.create_output_item_added_event(*output_index, &item_id));
                }

                // Accumulate text content
                self.text_content
                    .entry(item_id.clone())
                    .and_modify(|content| content.push_str(delta))
                    .or_insert_with(|| delta.clone());

                // Emit text delta with filled-in item_id and sequence_number
                let mut delta_event = stream_event.as_ref().clone();
                if let ResponsesAPIStreamEvent::ResponseOutputTextDelta {
                    item_id: ref mut id,
                    sequence_number: ref mut seq,
                    ..
                } = &mut delta_event
                {
                    *id = item_id;
                    *seq = self.next_sequence_number();
                }
                events.push(event_to_sse(delta_event));
            }
            ResponsesAPIStreamEvent::ResponseFunctionCallArgumentsDelta {
                output_index,
                delta,
                call_id,
                name,
                ..
            } => {
                let item_id = self.get_or_create_item_id(*output_index, "fc");

                // Store metadata if provided (from initial tool call event)
                if let (Some(cid), Some(n)) = (call_id, name) {
                    self.tool_call_metadata
                        .insert(*output_index, (cid.clone(), n.clone()));
                }

                // Emit output_item.added if this is the first time we see this tool call
                if !self.output_items_added.contains_key(output_index) {
                    self.output_items_added
                        .insert(*output_index, item_id.clone());

                    // For tool calls, we need call_id and name from metadata
                    // These should now be populated from the event itself
                    let (call_id, name) = self
                        .tool_call_metadata
                        .get(output_index)
                        .cloned()
                        .unwrap_or_else(|| {
                            (
                                format!("call_{}", uuid::Uuid::new_v4()),
                                "unknown".to_string(),
                            )
                        });

                    events.push(self.create_tool_call_added_event(
                        *output_index,
                        &item_id,
                        &call_id,
                        &name,
                    ));
                }

                // Accumulate function arguments
                self.function_arguments
                    .entry(item_id.clone())
                    .and_modify(|args| args.push_str(delta))
                    .or_insert_with(|| delta.clone());

                // Emit function call arguments delta with filled-in item_id and sequence_number
                let mut delta_event = stream_event.as_ref().clone();
                if let ResponsesAPIStreamEvent::ResponseFunctionCallArgumentsDelta {
                    item_id: ref mut id,
                    sequence_number: ref mut seq,
                    ..
                } = &mut delta_event
                {
                    *id = item_id;
                    *seq = self.next_sequence_number();
                }
                events.push(event_to_sse(delta_event));
            }
            _ => {
                // For other event types, just pass through with sequence number
                let other_event = stream_event.as_ref().clone();
                // TODO: Add sequence number to other event types if needed
                events.push(event_to_sse(other_event));
            }
        }

        // Store all generated events in the buffer
        self.buffered_events.extend(events);
    }

    fn to_bytes(&mut self) -> Vec<u8> {
        // For Responses API, we need special handling:
        // - Most events are already in buffered_events from add_transformed_event
        // - We should NOT finalize here - finalization happens when we detect [DONE] or end of stream
        // - Just flush the accumulated events and clear the buffer

        // Convert all accumulated events to bytes and clear buffer
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
    use crate::apis::openai::OpenAIApi;
    use crate::apis::streaming_shapes::sse::SseStreamIter;
    use crate::clients::{SupportedAPIsFromClient, SupportedUpstreamAPIs};

    #[test]
    fn test_chat_completions_to_responses_api_transformation() {
        // ChatCompletions input that will be transformed to ResponsesAPI
        let raw_input = r#"data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1234567890,"model":"gpt-4o","choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"},"finish_reason":null}]}

    data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1234567890,"model":"gpt-4o","choices":[{"index":0,"delta":{"content":" world"},"finish_reason":null}]}

    data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1234567890,"model":"gpt-4o","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

    data: [DONE]"#;

        println!("\n{}", "=".repeat(80));
        println!("TEST 2: ChatCompletions → ResponsesAPI Transformation (with [DONE])");
        println!("{}", "=".repeat(80));
        println!("\nRAW INPUT (ChatCompletions):");
        println!("{}", "-".repeat(80));
        println!("{}", raw_input);

        // Setup API configuration for transformation
        let client_api = SupportedAPIsFromClient::OpenAIResponsesAPI(OpenAIApi::Responses);
        let upstream_api = SupportedUpstreamAPIs::OpenAIChatCompletions(OpenAIApi::ChatCompletions);

        // Parse events and apply transformation
        let stream_iter = SseStreamIter::try_from(raw_input.as_bytes()).unwrap();
        let mut buffer = ResponsesAPIStreamBuffer::new();

        for raw_event in stream_iter {
            // Transform the event using the client/upstream APIs
            let transformed_event =
                SseEvent::try_from((raw_event, &client_api, &upstream_api)).unwrap();
            buffer.add_transformed_event(transformed_event);
        }

        let output_bytes = buffer.to_bytes();
        let output = String::from_utf8_lossy(&output_bytes);

        println!("\nTRANSFORMED OUTPUT (ResponsesAPI):");
        println!("{}", "-".repeat(80));
        println!("{}", output);

        // Assertions
        assert!(!output_bytes.is_empty(), "Should have output");
        assert!(
            output.contains("response.created"),
            "Should have response.created"
        );
        assert!(
            output.contains("response.in_progress"),
            "Should have response.in_progress"
        );
        assert!(
            output.contains("response.output_item.added"),
            "Should have output_item.added"
        );
        assert!(
            output.contains("response.output_text.delta"),
            "Should have text deltas"
        );
        assert!(
            output.contains("response.output_text.done"),
            "Should have text.done"
        );
        assert!(
            output.contains("response.output_item.done"),
            "Should have output_item.done"
        );
        assert!(
            output.contains("response.completed"),
            "Should have response.completed"
        );

        println!("\nVALIDATION SUMMARY:");
        println!("{}", "-".repeat(80));
        println!("✓ Lifecycle events: response.created, response.in_progress, response.completed");
        println!("✓ Output item lifecycle: output_item.added, output_item.done");
        println!("✓ Text streaming: output_text.delta (2 deltas), output_text.done");
        println!("✓ Complete transformation with finalization ([DONE] processed)\n");
    }

    #[test]
    fn test_partial_streaming_incremental_output() {
        let raw_input = r#"data: {"id":"chatcmpl-CfpqklihniLRuuQfP7inMb2ghtGmT","object":"chat.completion.chunk","created":1764086794,"model":"gpt-4o-2024-08-06","service_tier":"default","system_fingerprint":"fp_7eeb46f068","choices":[{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_mD5ggLKk3SMKGPFqFdcpKg6q","type":"function","function":{"name":"get_weather","arguments":""}}],"refusal":null},"logprobs":null,"finish_reason":null}],"obfuscation":"PCFrpy"}

    data: {"id":"chatcmpl-CfpqklihniLRuuQfP7inMb2ghtGmT","object":"chat.completion.chunk","created":1764086794,"model":"gpt-4o-2024-08-06","service_tier":"default","system_fingerprint":"fp_7eeb46f068","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\""}}]},"logprobs":null,"finish_reason":null}],"obfuscation":""}

    data: {"id":"chatcmpl-CfpqklihniLRuuQfP7inMb2ghtGmT","object":"chat.completion.chunk","created":1764086794,"model":"gpt-4o-2024-08-06","service_tier":"default","system_fingerprint":"fp_7eeb46f068","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"location"}}]},"logprobs":null,"finish_reason":null}],"obfuscation":"TC58A3QEIx8"}

    data: {"id":"chatcmpl-CfpqklihniLRuuQfP7inMb2ghtGmT","object":"chat.completion.chunk","created":1764086794,"model":"gpt-4o-2024-08-06","service_tier":"default","system_fingerprint":"fp_7eeb46f068","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\":\""}}]},"logprobs":null,"finish_reason":null}],"obfuscation":"PK4oFzlVlGTUP5"}"#;

        println!("\n{}", "=".repeat(80));
        println!("TEST 3: Partial Streaming - Function Calling (NO [DONE])");
        println!("{}", "=".repeat(80));
        println!("\nRAW INPUT (ChatCompletions - NO [DONE]):");
        println!("{}", "-".repeat(80));
        println!("{}", raw_input);

        // Setup API configuration for transformation
        let client_api = SupportedAPIsFromClient::OpenAIResponsesAPI(OpenAIApi::Responses);
        let upstream_api = SupportedUpstreamAPIs::OpenAIChatCompletions(OpenAIApi::ChatCompletions);

        // Transform all events
        let stream_iter = SseStreamIter::try_from(raw_input.as_bytes()).unwrap();
        let mut buffer = ResponsesAPIStreamBuffer::new();

        for raw_event in stream_iter {
            let transformed = SseEvent::try_from((raw_event, &client_api, &upstream_api)).unwrap();
            buffer.add_transformed_event(transformed);
        }

        let output_bytes = buffer.to_bytes();
        let output = String::from_utf8_lossy(&output_bytes);

        println!("\nTRANSFORMED OUTPUT (ResponsesAPI):");
        println!("{}", "-".repeat(80));
        println!("{}", output);

        // Assertions
        assert!(
            output.contains("response.created"),
            "Should have response.created"
        );
        assert!(
            output.contains("response.in_progress"),
            "Should have response.in_progress"
        );
        assert!(
            output.contains("response.output_item.added"),
            "Should have output_item.added"
        );
        assert!(
            output.contains("\"type\":\"function_call\""),
            "Should be function_call type"
        );
        assert!(
            output.contains("\"name\":\"get_weather\""),
            "Should have function name"
        );
        assert!(
            output.contains("\"call_id\":\"call_mD5ggLKk3SMKGPFqFdcpKg6q\""),
            "Should have correct call_id"
        );

        let delta_count = output
            .matches("event: response.function_call_arguments.delta")
            .count();
        assert_eq!(delta_count, 4, "Should have 4 delta events");

        assert!(
            !output.contains("response.function_call_arguments.done"),
            "Should NOT have arguments.done"
        );
        assert!(
            !output.contains("response.output_item.done"),
            "Should NOT have output_item.done"
        );
        assert!(
            !output.contains("response.completed"),
            "Should NOT have response.completed"
        );

        println!("\nVALIDATION SUMMARY:");
        println!("{}", "-".repeat(80));
        println!("✓ Lifecycle events: response.created, response.in_progress");
        println!(
            "✓ Function call metadata: name='get_weather', call_id='call_mD5ggLKk3SMKGPFqFdcpKg6q'"
        );
        println!("✓ Incremental deltas: 4 events (1 initial + 3 argument chunks)");
        println!("✓ NO completion events (partial stream, no [DONE])");
        println!("✓ Arguments accumulated: '{{\"location\":\"'\n");
    }
}
