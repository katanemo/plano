use bytes::Bytes;
use common::errors::BrightStaffError;
use hermesllm::apis::OpenAIApi;
use hermesllm::clients::{SupportedAPIsFromClient, SupportedUpstreamAPIs};
use hermesllm::SseEvent;
use http_body_util::combinators::BoxBody;
use http_body_util::StreamBody;
use hyper::body::Frame;
use hyper::Response;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
use tracing::{info, warn, Instrument};

use super::full;

/// Service for handling HTTP responses and streaming
pub struct ResponseHandler;

impl ResponseHandler {
    pub fn new() -> Self {
        Self
    }

    /// Create a full response body from bytes
    pub fn create_full_body<T: Into<Bytes>>(chunk: T) -> BoxBody<Bytes, hyper::Error> {
        full(chunk)
    }

    /// Create a JSON error response with BAD_REQUEST status
    pub fn create_json_error_response(
        json: &serde_json::Value,
    ) -> Response<BoxBody<Bytes, hyper::Error>> {
        let body = Self::create_full_body(json.to_string());
        let mut response = Response::new(body);
        *response.status_mut() = hyper::StatusCode::BAD_REQUEST;
        response.headers_mut().insert(
            hyper::header::CONTENT_TYPE,
            hyper::header::HeaderValue::from_static("application/json"),
        );
        response
    }

    /// Create a BAD_REQUEST error response with a message
    pub fn create_bad_request(message: &str) -> Response<BoxBody<Bytes, hyper::Error>> {
        let json = serde_json::json!({"error": message});
        Self::create_json_error_response(&json)
    }

    /// Create an INTERNAL_SERVER_ERROR response with a message
    pub fn create_internal_error(message: &str) -> Response<BoxBody<Bytes, hyper::Error>> {
        let json = serde_json::json!({"error": message});
        let body = Self::create_full_body(json.to_string());
        let mut response = Response::new(body);
        *response.status_mut() = hyper::StatusCode::INTERNAL_SERVER_ERROR;
        response.headers_mut().insert(
            hyper::header::CONTENT_TYPE,
            hyper::header::HeaderValue::from_static("application/json"),
        );
        response
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

            let sse_iter = SseStreamIter::try_from(response_bytes.as_ref()).map_err(|e| {
                BrightStaffError::StreamError(format!("Failed to parse SSE stream: {}", e))
            })?;
            let mut accumulated_text = String::new();

            for sse_event in sse_iter {
                // Skip [DONE] markers and event-only lines
                if sse_event.is_done() || sse_event.is_event_only() {
                    continue;
                }

                let transformed_event =
                    match SseEvent::try_from((sse_event, &client_api, &upstream_api)) {
                        Ok(event) => event,
                        Err(e) => {
                            warn!(error = ?e, "failed to transform SSE event, skipping");
                            continue;
                        }
                    };

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
            let response_text = String::from_utf8(response_bytes.to_vec()).map_err(|e| {
                BrightStaffError::StreamError(format!("Failed to decode response: {}", e))
            })?;

            // transfer assistant's text to the next agent rather than the transport
            // envelope.
            match serde_json::from_str::<serde_json::Value>(&response_text) {
                Ok(body) => match body
                    .pointer("/choices/0/message/content")
                    .and_then(serde_json::Value::as_str)
                {
                    Some(content) => Ok(content.to_string()),
                    None => {
                        warn!("no message content in agent response, passing body through");
                        Ok(response_text)
                    }
                },
                Err(err) => {
                    warn!(
                        error = %err,
                        "agent response is not json, passing body through"
                    );
                    Ok(response_text)
                }
            }
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

    /// Serve `body` with `content_type` and hand back the live reqwest response.
    async fn upstream_response(
        server: &mut mockito::Server,
        content_type: &str,
        body: &str,
    ) -> reqwest::Response {
        server
            .mock("GET", "/agent")
            .with_status(200)
            .with_header("content-type", content_type)
            .with_body(body)
            .create_async()
            .await;

        reqwest::Client::new()
            .get(server.url() + "/agent")
            .send()
            .await
            .unwrap()
    }

    /// Intermediate agent replies are injected into the next agent's context, so
    /// they must carry the assistant's text and not the transport envelope.
    #[tokio::test]
    async fn collect_full_response_extracts_content_from_chat_completion() {
        let mut server = mockito::Server::new_async().await;
        let body = serde_json::json!({
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "created": 0,
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "the actual answer"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}
        })
        .to_string();

        let response = upstream_response(&mut server, "application/json", &body).await;
        let collected = ResponseHandler::new()
            .collect_full_response(response)
            .await
            .unwrap();

        assert_eq!(collected, "the actual answer");
    }

    /// Agents are plain HTTP services and many omit `usage`. Extraction must not
    /// depend on the full OpenAI response schema being present.
    #[tokio::test]
    async fn collect_full_response_extracts_content_without_usage_field() {
        let mut server = mockito::Server::new_async().await;
        let body = serde_json::json!({
            "id": "chatcmpl-2",
            "choices": [{"message": {"role": "assistant", "content": "terse agent"}}]
        })
        .to_string();

        let response = upstream_response(&mut server, "application/json", &body).await;
        let collected = ResponseHandler::new()
            .collect_full_response(response)
            .await
            .unwrap();

        assert_eq!(collected, "terse agent");
    }

    /// A body we cannot read is passed through rather than dropped.
    #[tokio::test]
    async fn collect_full_response_passes_through_non_json_body() {
        let mut server = mockito::Server::new_async().await;

        let response = upstream_response(&mut server, "text/plain", "plain text reply").await;
        let collected = ResponseHandler::new()
            .collect_full_response(response)
            .await
            .unwrap();

        assert_eq!(collected, "plain text reply");
    }

    /// A completion with no textual content (for example tool calls only) also
    /// falls back instead of yielding an empty message.
    #[tokio::test]
    async fn collect_full_response_passes_through_completion_without_content() {
        let mut server = mockito::Server::new_async().await;
        let body = serde_json::json!({
            "id": "chatcmpl-3",
            "choices": [{"message": {"role": "assistant", "content": null}}]
        })
        .to_string();

        let response = upstream_response(&mut server, "application/json", &body).await;
        let collected = ResponseHandler::new()
            .collect_full_response(response)
            .await
            .unwrap();

        assert_eq!(collected, body);
    }

    /// The streaming branch accumulates content deltas across chunks.
    #[tokio::test]
    async fn collect_full_response_accumulates_sse_content_deltas() {
        let mut server = mockito::Server::new_async().await;
        let body = concat!(
            "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,",
            "\"delta\":{\"content\":\"hello \"}}]}\n\n",
            "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,",
            "\"delta\":{\"content\":\"world\"}}]}\n\n",
            "data: [DONE]\n\n"
        );

        let response = upstream_response(&mut server, "text/event-stream", body).await;
        let collected = ResponseHandler::new()
            .collect_full_response(response)
            .await
            .unwrap();

        assert_eq!(collected, "hello world");
    }

    #[tokio::test]
    async fn streamed_and_whole_deliveries_of_one_reply_collect_identically() {
        let handler = ResponseHandler::new();

        let mut streaming_server = mockito::Server::new_async().await;
        let sse_body = concat!(
            "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,",
            "\"delta\":{\"content\":\"the same \"}}]}\n\n",
            "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":0,",
            "\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,",
            "\"delta\":{\"content\":\"reply\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let streamed = handler
            .collect_full_response(
                upstream_response(&mut streaming_server, "text/event-stream", sse_body).await,
            )
            .await
            .unwrap();

        let mut whole_server = mockito::Server::new_async().await;
        let whole_body = serde_json::json!({
            "id": "1",
            "object": "chat.completion",
            "created": 0,
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "the same reply"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}
        })
        .to_string();
        let whole = handler
            .collect_full_response(
                upstream_response(&mut whole_server, "application/json", &whole_body).await,
            )
            .await
            .unwrap();

        assert_eq!(
            streamed, whole,
            "streaming and non-streaming delivery of one reply must collect identically"
        );
    }
}
