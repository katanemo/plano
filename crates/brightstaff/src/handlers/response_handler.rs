use bytes::Bytes;
use common::errors::BrightStaffError;
use hermesllm::apis::OpenAIApi;
use hermesllm::clients::{SupportedAPIsFromClient, SupportedUpstreamAPIs};
use hermesllm::SseEvent;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::Frame;
use hyper::Response;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
use tracing::{info, warn, Instrument};

/// Service for handling HTTP responses and streaming
pub struct ResponseHandler;

impl ResponseHandler {
    pub fn new() -> Self {
        Self
    }

    /// Create a full response body from bytes
    pub fn create_full_body<T: Into<Bytes>>(chunk: T) -> BoxBody<Bytes, hyper::Error> {
        Full::new(chunk.into())
            .map_err(|never| match never {})
            .boxed()
    }

    /// Create a streaming response from a reqwest response.
    /// The spawned streaming task is instrumented with both `agent_span` and `orchestrator_span`
    /// so their durations reflect the actual time spent streaming to the client.
    pub async fn create_streaming_response(
        &self,
        llm_response: reqwest::Response,
        agent_span: tracing::Span,
        orchestrator_span: tracing::Span,
    ) -> Result<Response<BoxBody<Bytes, hyper::Error>>, BrightStaffError> {
        // Copy headers from the original response
        let response_headers = llm_response.headers();
        let mut response_builder = Response::builder();

        let headers = response_builder.headers_mut().ok_or_else(|| {
            BrightStaffError::StreamError("Failed to get mutable headers".to_string())
        })?;

        for (header_name, header_value) in response_headers.iter() {
            headers.insert(header_name, header_value.clone());
        }

        // Create channel for async streaming
        let (tx, rx) = mpsc::channel::<Bytes>(16);

        // Spawn streaming task instrumented with both spans (nested) so both
        // remain entered for the full streaming duration.
        tokio::spawn(
            async move {
                let mut byte_stream = llm_response.bytes_stream();

                while let Some(item) = byte_stream.next().await {
                    let chunk = match item {
                        Ok(chunk) => chunk,
                        Err(err) => {
                            warn!(error = ?err, "error receiving chunk");
                            break;
                        }
                    };

                    if tx.send(chunk).await.is_err() {
                        warn!("receiver dropped");
                        break;
                    }
                }
            }
            .instrument(agent_span)
            .instrument(orchestrator_span),
        );

        let stream = ReceiverStream::new(rx).map(|chunk| Ok::<_, hyper::Error>(Frame::data(chunk)));
        let stream_body = BoxBody::new(StreamBody::new(stream));

        response_builder
            .body(stream_body)
            .map_err(BrightStaffError::from)
    }

    /// Collect the full response body as a string
    /// This is used for intermediate agents where we need to capture the full response
    /// before passing it to the next agent.
    ///
    /// This method handles both streaming and non-streaming responses:
    /// - For streaming SSE responses: parses chunks and extracts text deltas
    /// - For non-streaming responses: returns the full text
    pub async fn collect_full_response(
        &self,
        llm_response: reqwest::Response,
    ) -> Result<String, BrightStaffError> {
        use hermesllm::apis::streaming_shapes::sse::SseStreamIter;

        let response_headers = llm_response.headers();
        let is_sse_streaming = response_headers
            .get(hyper::header::CONTENT_TYPE)
            .is_some_and(|v| v.to_str().unwrap_or("").contains("text/event-stream"));

        let response_bytes = llm_response.bytes().await.map_err(|e| {
            BrightStaffError::StreamError(format!("Failed to read response: {}", e))
        })?;

        if is_sse_streaming {
            let client_api =
                SupportedAPIsFromClient::OpenAIChatCompletions(OpenAIApi::ChatCompletions);
            let upstream_api =
                SupportedUpstreamAPIs::OpenAIChatCompletions(OpenAIApi::ChatCompletions);

            let sse_iter = SseStreamIter::try_from(response_bytes.as_ref()).unwrap();
            let mut accumulated_text = String::new();

            for sse_event in sse_iter {
                // Skip [DONE] markers and event-only lines
                if sse_event.is_done() || sse_event.is_event_only() {
                    continue;
                }

                let transformed_event =
                    SseEvent::try_from((sse_event, &client_api, &upstream_api)).unwrap();

                // Try to get provider response and extract content delta
                match transformed_event.provider_response() {
                    Ok(provider_response) => {
                        if let Some(content) = provider_response.content_delta() {
                            accumulated_text.push_str(content);
                        } else {
                            info!("no content delta in provider response");
                        }
                    }
                    Err(e) => {
                        warn!(error = ?e, "failed to parse provider response");
                    }
                }
            }
            Ok(accumulated_text)
        } else {
            // If not SSE, treat as regular text response
            let response_text = String::from_utf8(response_bytes.to_vec()).map_err(|e| {
                BrightStaffError::StreamError(format!("Failed to decode response: {}", e))
            })?;

            Ok(response_text)
        }
    }
}

impl Default for ResponseHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::StatusCode;

    #[tokio::test]
    async fn test_create_streaming_response_with_mock() {
        use mockito::Server;

        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/test")
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body("streaming response")
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let llm_response = client.get(&(server.url() + "/test")).send().await.unwrap();

        let handler = ResponseHandler::new();
        let result = handler
            .create_streaming_response(
                llm_response,
                tracing::Span::current(),
                tracing::Span::current(),
            )
            .await;

        mock.assert_async().await;
        assert!(result.is_ok());

        let response = result.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().contains_key("content-type"));
    }
}
