use bytes::Bytes;
use flate2::read::GzDecoder;
use hermesllm::apis::openai_responses::{InputItem, OutputItem, ResponsesAPIStreamEvent};
use hermesllm::apis::streaming_shapes::sse::SseStreamIter;
use hermesllm::transforms::response::output_to_input::outputs_to_inputs;
use std::io::Read;
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::handlers::utils::StreamProcessor;
use crate::state::{OpenAIConversationState, StateStorage};

/// Processor that wraps another processor and handles v1/responses state management
/// Captures response_id and output from streaming responses, stores state after completion
pub struct ResponsesStateProcessor<P: StreamProcessor> {
    /// The underlying processor (e.g., ObservableStreamProcessor for metrics)
    inner: P,

    /// State storage backend
    storage: Arc<dyn StateStorage>,

    /// Original input items from the request
    original_input: Vec<InputItem>,

    /// Model name
    model: String,

    /// Provider name
    provider: String,

    /// Whether this is a streaming request
    is_streaming: bool,

    /// Whether upstream is OpenAI (skip storage if true)
    is_openai_upstream: bool,

    /// Content-Encoding header value (e.g., "gzip", "br", None)
    content_encoding: Option<String>,

    /// Request ID for logging
    request_id: String,

    /// Buffer for accumulating chunks (needed for non-streaming compressed responses)
    chunk_buffer: Vec<u8>,

    /// Captured response_id from response.completed event
    response_id: Option<String>,

    /// Captured output items from response.completed event
    output_items: Option<Vec<OutputItem>>,
}

impl<P: StreamProcessor> ResponsesStateProcessor<P> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        inner: P,
        storage: Arc<dyn StateStorage>,
        original_input: Vec<InputItem>,
        model: String,
        provider: String,
        is_streaming: bool,
        is_openai_upstream: bool,
        content_encoding: Option<String>,
        request_id: String,
    ) -> Self {
        Self {
            inner,
            storage,
            original_input,
            model,
            provider,
            is_streaming,
            is_openai_upstream,
            content_encoding,
            request_id,
            chunk_buffer: Vec::new(),
            response_id: None,
            output_items: None,
        }
    }

    /// Decompress accumulated buffer based on Content-Encoding header
    fn decompress_buffer(&self) -> Vec<u8> {
        if self.chunk_buffer.is_empty() {
            return Vec::new();
        }

        match self.content_encoding.as_deref() {
            Some("gzip") => {
                let mut decoder = GzDecoder::new(self.chunk_buffer.as_slice());
                let mut decompressed = Vec::new();
                match decoder.read_to_end(&mut decompressed) {
                    Ok(_) => {
                        debug!(
                            "[PLANO_REQ_ID:{}] | STATE_PROCESSOR | Successfully decompressed {} bytes to {} bytes",
                            self.request_id,
                            self.chunk_buffer.len(),
                            decompressed.len()
                        );
                        decompressed
                    }
                    Err(e) => {
                        warn!(
                            "[PLANO_REQ_ID:{}] | STATE_PROCESSOR | Failed to decompress gzip buffer: {}",
                            self.request_id,
                            e
                        );
                        self.chunk_buffer.clone()
                    }
                }
            }
            Some(encoding) => {
                warn!(
                    "[PLANO_REQ_ID:{}] | STATE_PROCESSOR | Unsupported Content-Encoding: {}. Only gzip is currently supported.",
                    self.request_id,
                    encoding
                );
                self.chunk_buffer.clone()
            }
            None => self.chunk_buffer.clone(),
        }
    }

    /// Parse response to extract response_id and output
    /// For streaming: parse SSE events looking for response.completed (per chunk)
    /// For non-streaming: buffer all chunks, then decompress and parse on completion
    fn try_parse_response_chunk(&mut self, chunk: &[u8]) {
        if self.is_streaming {
            // Streaming: Try to parse SSE events from this chunk
            // Note: For compressed streaming, we'd need to buffer and decompress first
            // but most streaming responses aren't compressed since SSE needs to be readable
            let sse_iter = match SseStreamIter::try_from(chunk) {
                Ok(iter) => iter,
                Err(_) => return, // Not valid SSE format, skip
            };

            // Process each SSE event in the chunk, looking for data lines with response.completed
            for event in sse_iter {
                // Only process data lines (skip event-only lines)
                if let Some(data_str) = &event.data {
                    // Try to parse as ResponsesAPIStreamEvent and check if it's a ResponseCompleted event
                    if let Ok(ResponsesAPIStreamEvent::ResponseCompleted { response, .. }) =
                        serde_json::from_str::<ResponsesAPIStreamEvent>(data_str)
                    {
                        info!(
                            "[PLANO_REQ_ID:{}] | STATE_PROCESSOR | Captured streaming response.completed: response_id={}, output_items={}",
                            self.request_id,
                            response.id,
                            response.output.len()
                        );
                        self.response_id = Some(response.id.clone());
                        self.output_items = Some(response.output.clone());
                        return; // Found what we need, exit early
                    }
                }
            }
        } else {
            // Non-streaming: Buffer chunks, will decompress and parse on completion
            self.chunk_buffer.extend_from_slice(chunk);
        }
    }

    /// Parse buffered non-streaming response (called on completion)
    fn try_parse_buffered_response(&mut self) {
        if self.is_streaming || self.chunk_buffer.is_empty() {
            return;
        }

        // Decompress if needed
        let decompressed = self.decompress_buffer();

        // Parse complete JSON response
        match serde_json::from_slice::<hermesllm::apis::openai_responses::ResponsesAPIResponse>(
            &decompressed,
        ) {
            Ok(response) => {
                info!(
                    "[PLANO_REQ_ID:{}] | STATE_PROCESSOR | Captured non-streaming response: response_id={}, output_items={}",
                    self.request_id,
                    response.id,
                    response.output.len()
                );
                self.response_id = Some(response.id.clone());
                self.output_items = Some(response.output.clone());
            }
            Err(e) => {
                // Log parse error with chunk preview for debugging
                let chunk_preview = String::from_utf8_lossy(&decompressed);
                let preview_len = chunk_preview.len().min(200);
                warn!(
                    "[PLANO_REQ_ID:{}] | STATE_PROCESSOR | Failed to parse non-streaming ResponsesAPIResponse: {}. Decompressed preview (first {} bytes): {}",
                    self.request_id,
                    e,
                    preview_len,
                    &chunk_preview[..preview_len]
                );
            }
        }
    }
}

impl<P: StreamProcessor> StreamProcessor for ResponsesStateProcessor<P> {
    fn process_chunk(&mut self, chunk: Bytes) -> Result<Option<Bytes>, String> {
        // Buffer/parse chunk for response extraction
        self.try_parse_response_chunk(&chunk);

        // Forward to inner processor
        self.inner.process_chunk(chunk)
    }

    fn on_first_bytes(&mut self) {
        self.inner.on_first_bytes();
    }

    fn on_complete(&mut self) {
        // For non-streaming, decompress and parse buffered response
        self.try_parse_buffered_response();

        // First, let the inner processor complete
        self.inner.on_complete();

        // Skip storage for OpenAI upstream
        if self.is_openai_upstream {
            debug!(
                "[PLANO_REQ_ID:{}] | STATE_PROCESSOR | Skipping state storage for OpenAI upstream provider",
                self.request_id
            );
            return;
        }

        // Store state if we captured response_id and output
        if let (Some(response_id), Some(output_items)) = (&self.response_id, &self.output_items) {
            // Convert output items to input items for next request
            let output_as_inputs = outputs_to_inputs(output_items);

            debug!(
                "[PLANO_REQ_ID:{}] | STATE_PROCESSOR | Converting outputs to inputs: output_items_count={}, converted_input_items_count={}",
                self.request_id, output_items.len(), output_as_inputs.len()
            );

            // Combine original input + output as new input history
            let mut combined_input = self.original_input.clone();
            combined_input.extend(output_as_inputs);

            debug!(
                "[PLANO_REQ_ID:{}] | STATE_PROCESSOR | Storing state: original_input_count={}, combined_input_count={}, combined_json={}",
                self.request_id,
                self.original_input.len(),
                combined_input.len(),
                serde_json::to_string(&combined_input).unwrap_or_else(|_| "serialization_error".to_string())
            );

            let state = OpenAIConversationState {
                response_id: response_id.clone(),
                input_items: combined_input,
                created_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64,
                model: self.model.clone(),
                provider: self.provider.clone(),
            };

            // Store asynchronously (fire and forget with logging)
            let storage = self.storage.clone();
            let response_id_clone = response_id.clone();
            let request_id = self.request_id.clone();
            let items_count = state.input_items.len();
            tokio::spawn(async move {
                match storage.put(state).await {
                    Ok(()) => {
                        info!(
                            "[PLANO_REQ_ID:{}] | STATE_PROCESSOR | Successfully stored conversation state for response_id: {}, items_count={}",
                            request_id,
                            response_id_clone,
                            items_count
                        );
                    }
                    Err(e) => {
                        warn!(
                            "[PLANO_REQ_ID:{}] | STATE_PROCESSOR | Failed to store conversation state for response_id {}: {}",
                            request_id,
                            response_id_clone,
                            e
                        );
                    }
                }
            });
        } else {
            warn!(
                "[PLANO_REQ_ID:{}] | STATE_PROCESSOR | No response_id captured from upstream response - cannot store conversation state. response_id present: {}, output present: {}",
                self.request_id,
                self.response_id.is_some(),
                self.output_items.is_some()
            );
        }
    }

    fn on_error(&mut self, error: &str) {
        self.inner.on_error(error);
    }
}
