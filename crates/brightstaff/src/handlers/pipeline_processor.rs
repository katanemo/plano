use std::collections::HashMap;

use common::configuration::{Agent, AgentFilterChain};
use common::consts::{
    ARCH_UPSTREAM_HOST_HEADER, BRIGHT_STAFF_SERVICE_NAME, ENVOY_RETRY_HEADER, TRACE_PARENT_HEADER,
};
use hermesllm::apis::openai::Message;
use hermesllm::{ProviderRequest, ProviderRequestType};
use hyper::header::HeaderMap;
use opentelemetry::global;
use opentelemetry_http::HeaderInjector;
use tracing::{debug, info, instrument, warn};

use crate::handlers::jsonrpc::{
    JsonRpcId, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, JSON_RPC_VERSION,
    MCP_INITIALIZE, MCP_INITIALIZE_NOTIFICATION, TOOL_CALL_METHOD,
};
use crate::tracing::{operation_component, set_service_name};
use uuid::Uuid;

/// Errors that can occur during pipeline processing
#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("HTTP request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),
    #[error("Failed to parse response: {0}")]
    ParseError(#[from] serde_json::Error),
    #[error("Agent '{0}' not found in agent map")]
    AgentNotFound(String),
    #[error("No choices in response from agent '{0}'")]
    NoChoicesInResponse(String),
    #[error("No content in response from agent '{0}'")]
    NoContentInResponse(String),
    #[error("No result in response from agent '{0}'")]
    NoResultInResponse(String),
    #[error("No structured content in response from agent '{0}'")]
    NoStructuredContentInResponse(String),
    #[error("No messages in response from agent '{0}'")]
    NoMessagesInResponse(String),
    #[error("Client error from agent '{agent}' (HTTP {status}): {body}")]
    ClientError {
        agent: String,
        status: u16,
        body: String,
    },
    #[error("Server error from agent '{agent}' (HTTP {status}): {body}")]
    ServerError {
        agent: String,
        status: u16,
        body: String,
    },
}

/// Service for processing agent pipelines
pub struct PipelineProcessor {
    client: reqwest::Client,
    url: String,
    agent_id_session_map: HashMap<String, String>,
}

const ENVOY_API_ROUTER_ADDRESS: &str = "http://localhost:11000";

impl Default for PipelineProcessor {
    fn default() -> Self {
        Self {
            client: reqwest::Client::new(),
            url: ENVOY_API_ROUTER_ADDRESS.to_string(),
            agent_id_session_map: HashMap::new(),
        }
    }
}

impl PipelineProcessor {
    pub fn new(url: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            url,
            agent_id_session_map: HashMap::new(),
        }
    }

    // /// Process the filter chain of agents (all except the terminal agent)
    // #[instrument(
    //     skip(self, chat_history, agent_filter_chain, agent_map, request_headers),
    //     fields(
    //         filter_count = agent_filter_chain.filter_chain.as_ref().map(|fc| fc.len()).unwrap_or(0),
    //         message_count = chat_history.len()
    //     )
    // )]
    #[allow(clippy::too_many_arguments)]
    pub async fn process_filter_chain(
        &mut self,
        chat_history: &[Message],
        agent_filter_chain: &AgentFilterChain,
        agent_map: &HashMap<String, Agent>,
        request_headers: &HeaderMap,
    ) -> Result<Vec<Message>, PipelineError> {
        let mut chat_history_updated = chat_history.to_vec();

        // If filter_chain is None or empty, proceed without filtering
        let filter_chain = match agent_filter_chain.filter_chain.as_ref() {
            Some(fc) if !fc.is_empty() => fc,
            _ => return Ok(chat_history_updated),
        };

        for agent_name in filter_chain {
            debug!(agent = %agent_name, "processing filter agent");

            let agent = agent_map
                .get(agent_name)
                .ok_or_else(|| PipelineError::AgentNotFound(agent_name.clone()))?;

            let tool_name = agent.tool.as_deref().unwrap_or(&agent.id);

            info!(
                agent = %agent_name,
                tool = %tool_name,
                url = %agent.url,
                agent_type = %agent.agent_type.as_deref().unwrap_or("mcp"),
                conversation_len = chat_history.len(),
                "executing filter"
            );

            if agent.agent_type.as_deref().unwrap_or("mcp") == "mcp" {
                chat_history_updated = self
                    .execute_mcp_filter(&chat_history_updated, agent, request_headers)
                    .await?;
            } else {
                chat_history_updated = self
                    .execute_http_filter(&chat_history_updated, agent, request_headers)
                    .await?;
            }

            info!(
                agent = %agent_name,
                updated_len = chat_history_updated.len(),
                "filter completed"
            );
        }

        Ok(chat_history_updated)
    }

    /// Build common MCP headers for requests
    fn build_mcp_headers(
        &self,
        request_headers: &HeaderMap,
        agent_id: &str,
        session_id: Option<&str>,
    ) -> Result<HeaderMap, PipelineError> {
        let mut headers = request_headers.clone();
        headers.remove(hyper::header::CONTENT_LENGTH);

        // Inject OpenTelemetry trace context automatically
        headers.remove(TRACE_PARENT_HEADER);
        global::get_text_map_propagator(|propagator| {
            let cx =
                tracing_opentelemetry::OpenTelemetrySpanExt::context(&tracing::Span::current());
            propagator.inject_context(&cx, &mut HeaderInjector(&mut headers));
        });

        headers.insert(
            ARCH_UPSTREAM_HOST_HEADER,
            hyper::header::HeaderValue::from_str(agent_id)
                .map_err(|_| PipelineError::AgentNotFound(agent_id.to_string()))?,
        );

        headers.insert(
            ENVOY_RETRY_HEADER,
            hyper::header::HeaderValue::from_str("3").unwrap(),
        );

        headers.insert(
            "Accept",
            hyper::header::HeaderValue::from_static("application/json, text/event-stream"),
        );

        headers.insert(
            "Content-Type",
            hyper::header::HeaderValue::from_static("application/json"),
        );

        if let Some(sid) = session_id {
            headers.insert(
                "mcp-session-id",
                hyper::header::HeaderValue::from_str(sid).unwrap(),
            );
        }

        Ok(headers)
    }

    /// Parse SSE formatted response and extract JSON-RPC data
    fn parse_sse_response(
        &self,
        response_bytes: &[u8],
        agent_id: &str,
    ) -> Result<String, PipelineError> {
        let response_str = String::from_utf8_lossy(response_bytes);
        let lines: Vec<&str> = response_str.lines().collect();

        // Validate SSE format: first line should be "event: message"
        if lines.is_empty() || lines[0] != "event: message" {
            warn!(
                agent = %agent_id,
                first_line = ?lines.first(),
                "invalid SSE response format"
            );
            return Err(PipelineError::NoContentInResponse(format!(
                "Invalid SSE response format from agent {}: expected 'event: message' as first line",
                agent_id
            )));
        }

        // Find the data line
        let data_lines: Vec<&str> = lines
            .iter()
            .filter(|line| line.starts_with("data: "))
            .copied()
            .collect();

        if data_lines.len() != 1 {
            warn!(
                agent = %agent_id,
                found = data_lines.len(),
                "expected exactly one 'data:' line"
            );
            return Err(PipelineError::NoContentInResponse(format!(
                "Expected exactly one 'data:' line from agent {}, found {}",
                agent_id,
                data_lines.len()
            )));
        }

        // Skip "data: " prefix
        Ok(data_lines[0][6..].to_string())
    }

    /// Send an MCP request and return the response
    async fn send_mcp_request(
        &self,
        json_rpc_request: &JsonRpcRequest,
        headers: &HeaderMap,
        agent_id: &str,
    ) -> Result<reqwest::Response, PipelineError> {
        let request_body = serde_json::to_string(json_rpc_request)?;

        debug!(
            "Sending MCP request to agent {}: {}",
            agent_id, request_body
        );

        let response = self
            .client
            .post(format!("{}/mcp", self.url))
            .headers(headers.clone())
            .body(request_body)
            .send()
            .await?;

        Ok(response)
    }

    /// Build a tools/call JSON-RPC request
    fn build_tool_call_request(
        &self,
        tool_name: &str,
        messages: &[Message],
    ) -> Result<JsonRpcRequest, PipelineError> {
        let mut arguments = HashMap::new();
        arguments.insert("messages".to_string(), serde_json::to_value(messages)?);

        let mut params = HashMap::new();
        params.insert("name".to_string(), serde_json::to_value(tool_name)?);
        params.insert("arguments".to_string(), serde_json::to_value(arguments)?);

        Ok(JsonRpcRequest {
            jsonrpc: JSON_RPC_VERSION.to_string(),
            id: JsonRpcId::String(Uuid::new_v4().to_string()),
            method: TOOL_CALL_METHOD.to_string(),
            params: Some(params),
        })
    }

    /// Send request to a specific agent and return the response content
    #[instrument(
        skip(self, messages, agent, request_headers),
        fields(
            agent_id = %agent.id,
            filter_name = %agent.id,
            message_count = messages.len()
        )
    )]
    async fn execute_mcp_filter(
        &mut self,
        messages: &[Message],
        agent: &Agent,
        request_headers: &HeaderMap,
    ) -> Result<Vec<Message>, PipelineError> {
        // Set service name for this filter span
        set_service_name(operation_component::AGENT_FILTER);

        // Update current span name to include filter name
        use opentelemetry::trace::get_active_span;
        get_active_span(|span| {
            span.update_name(format!("execute_mcp_filter ({})", agent.id));
        });

        // Get or create MCP session
        let mcp_session_id = if let Some(session_id) = self.agent_id_session_map.get(&agent.id) {
            session_id.clone()
        } else {
            let session_id = self.get_new_session_id(&agent.id, request_headers).await;
            self.agent_id_session_map
                .insert(agent.id.clone(), session_id.clone());
            session_id
        };

        info!(
            "Using MCP session ID {} for agent {}",
            mcp_session_id, agent.id
        );

        // Build JSON-RPC request
        let tool_name = agent.tool.as_deref().unwrap_or(&agent.id);
        let json_rpc_request = self.build_tool_call_request(tool_name, messages)?;

        // Build headers
        let agent_headers =
            self.build_mcp_headers(request_headers, &agent.id, Some(&mcp_session_id))?;

        let response = self
            .send_mcp_request(&json_rpc_request, &agent_headers, &agent.id)
            .await?;
        let http_status = response.status();
        let response_bytes = response.bytes().await?;

        // Handle HTTP errors
        if !http_status.is_success() {
            let error_body = String::from_utf8_lossy(&response_bytes).to_string();
            return Err(if http_status.is_client_error() {
                PipelineError::ClientError {
                    agent: agent.id.clone(),
                    status: http_status.as_u16(),
                    body: error_body,
                }
            } else {
                PipelineError::ServerError {
                    agent: agent.id.clone(),
                    status: http_status.as_u16(),
                    body: error_body,
                }
            });
        }

        info!(
            "Response from agent {}: {}",
            agent.id,
            String::from_utf8_lossy(&response_bytes)
        );

        // Parse SSE response
        let data_chunk = self.parse_sse_response(&response_bytes, &agent.id)?;
        let response: JsonRpcResponse = serde_json::from_str(&data_chunk)?;
        let response_result = response
            .result
            .ok_or_else(|| PipelineError::NoResultInResponse(agent.id.clone()))?;

        // Check if error field is set in response result
        if response_result
            .get("isError")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            let error_message = response_result
                .get("content")
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|v| v.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown_error")
                .to_string();

            return Err(PipelineError::ClientError {
                agent: agent.id.clone(),
                status: hyper::StatusCode::BAD_REQUEST.as_u16(),
                body: error_message,
            });
        }

        // Extract structured content and parse messages
        let response_json = response_result
            .get("structuredContent")
            .ok_or_else(|| PipelineError::NoStructuredContentInResponse(agent.id.clone()))?;

        let messages: Vec<Message> = response_json
            .get("result")
            .and_then(|v| v.as_array())
            .ok_or_else(|| PipelineError::NoMessagesInResponse(agent.id.clone()))?
            .iter()
            .map(|msg_value| serde_json::from_value(msg_value.clone()))
            .collect::<Result<Vec<Message>, _>>()
            .map_err(PipelineError::ParseError)?;

        Ok(messages)
    }

    /// Build an initialize JSON-RPC request
    fn build_initialize_request(&self) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: JSON_RPC_VERSION.to_string(),
            id: JsonRpcId::String(Uuid::new_v4().to_string()),
            method: MCP_INITIALIZE.to_string(),
            params: Some({
                let mut params = HashMap::new();
                params.insert(
                    "protocolVersion".to_string(),
                    serde_json::Value::String("2024-11-05".to_string()),
                );
                params.insert("capabilities".to_string(), serde_json::json!({}));
                params.insert(
                    "clientInfo".to_string(),
                    serde_json::json!({
                        "name": BRIGHT_STAFF_SERVICE_NAME,
                        "version": "1.0.0"
                    }),
                );
                params
            }),
        }
    }

    /// Send initialized notification after session creation
    async fn send_initialized_notification(
        &self,
        agent_id: &str,
        session_id: &str,
        request_headers: &HeaderMap,
    ) -> Result<(), PipelineError> {
        let initialized_notification = JsonRpcNotification {
            jsonrpc: JSON_RPC_VERSION.to_string(),
            method: MCP_INITIALIZE_NOTIFICATION.to_string(),
            params: None,
        };

        let notification_body = serde_json::to_string(&initialized_notification)?;
        debug!("sending initialized notification for agent {}", agent_id);

        let headers = self.build_mcp_headers(request_headers, agent_id, Some(session_id))?;

        let response = self
            .client
            .post(format!("{}/mcp", self.url))
            .headers(headers)
            .body(notification_body)
            .send()
            .await?;

        info!(
            "initialized notification response status: {}",
            response.status()
        );

        Ok(())
    }

    async fn get_new_session_id(&self, agent_id: &str, request_headers: &HeaderMap) -> String {
        info!("initializing MCP session for agent {}", agent_id);

        let initialize_request = self.build_initialize_request();
        let headers = self
            .build_mcp_headers(request_headers, agent_id, None)
            .expect("Failed to build headers for initialization");

        let response = self
            .send_mcp_request(&initialize_request, &headers, agent_id)
            .await
            .expect("Failed to initialize MCP session");

        info!("initialize response status: {}", response.status());

        let session_id = response
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .expect("No mcp-session-id in response")
            .to_string();

        info!(
            "created new MCP session for agent {}: {}",
            agent_id, session_id
        );

        // Send initialized notification
        self.send_initialized_notification(agent_id, &session_id, &headers)
            .await
            .expect("Failed to send initialized notification");

        session_id
    }

    /// Execute a HTTP-based filter agent
    #[instrument(
        skip(self, messages, agent, request_headers),
        fields(
            agent_id = %agent.id,
            agent_url = %agent.url,
            filter_name = %agent.id,
            message_count = messages.len()
        )
    )]
    async fn execute_http_filter(
        &mut self,
        messages: &[Message],
        agent: &Agent,
        request_headers: &HeaderMap,
    ) -> Result<Vec<Message>, PipelineError> {
        // Set service name for this filter span
        set_service_name(operation_component::AGENT_FILTER);

        // Update current span name to include filter name
        use opentelemetry::trace::get_active_span;
        get_active_span(|span| {
            span.update_name(format!("execute_http_filter ({})", agent.id));
        });

        // Build headers
        let mut agent_headers = request_headers.clone();
        agent_headers.remove(hyper::header::CONTENT_LENGTH);

        // Inject OpenTelemetry trace context automatically
        agent_headers.remove(TRACE_PARENT_HEADER);
        global::get_text_map_propagator(|propagator| {
            let cx =
                tracing_opentelemetry::OpenTelemetrySpanExt::context(&tracing::Span::current());
            propagator.inject_context(&cx, &mut HeaderInjector(&mut agent_headers));
        });

        agent_headers.insert(
            ARCH_UPSTREAM_HOST_HEADER,
            hyper::header::HeaderValue::from_str(&agent.id)
                .map_err(|_| PipelineError::AgentNotFound(agent.id.clone()))?,
        );

        agent_headers.insert(
            ENVOY_RETRY_HEADER,
            hyper::header::HeaderValue::from_str("3").unwrap(),
        );

        agent_headers.insert(
            "Accept",
            hyper::header::HeaderValue::from_static("application/json"),
        );

        agent_headers.insert(
            "Content-Type",
            hyper::header::HeaderValue::from_static("application/json"),
        );

        debug!(
            "Sending HTTP request to agent {} at URL: {}",
            agent.id, agent.url
        );

        // Send messages array directly as request body
        let response = self
            .client
            .post(&agent.url)
            .headers(agent_headers)
            .json(&messages)
            .send()
            .await?;

        let http_status = response.status();
        let response_bytes = response.bytes().await?;

        // Handle HTTP errors
        if !http_status.is_success() {
            let error_body = String::from_utf8_lossy(&response_bytes).to_string();
            return Err(if http_status.is_client_error() {
                PipelineError::ClientError {
                    agent: agent.id.clone(),
                    status: http_status.as_u16(),
                    body: error_body,
                }
            } else {
                PipelineError::ServerError {
                    agent: agent.id.clone(),
                    status: http_status.as_u16(),
                    body: error_body,
                }
            });
        }

        debug!(
            "Response from HTTP agent {}: {}",
            agent.id,
            String::from_utf8_lossy(&response_bytes)
        );

        // Parse response - expecting array of messages directly
        let messages: Vec<Message> =
            serde_json::from_slice(&response_bytes).map_err(PipelineError::ParseError)?;

        Ok(messages)
    }

    /// Send request to terminal agent and return the raw response for streaming
    /// Note: The caller is responsible for creating the plano(agent) span that wraps
    /// both this call and the subsequent response consumption.
    pub async fn invoke_agent(
        &self,
        messages: &[Message],
        mut original_request: ProviderRequestType,
        terminal_agent: &Agent,
        request_headers: &HeaderMap,
    ) -> Result<reqwest::Response, PipelineError> {
        // let mut request = original_request.clone();
        original_request.set_messages(messages);

        let request_url = "/v1/chat/completions";

        let request_body = ProviderRequestType::to_bytes(&original_request).unwrap();
        // let request_body = serde_json::to_string(&request)?;
        debug!("sending request to terminal agent {}", terminal_agent.id);

        let mut agent_headers = request_headers.clone();
        agent_headers.remove(hyper::header::CONTENT_LENGTH);

        // Inject OpenTelemetry trace context automatically
        agent_headers.remove(TRACE_PARENT_HEADER);
        global::get_text_map_propagator(|propagator| {
            let cx =
                tracing_opentelemetry::OpenTelemetrySpanExt::context(&tracing::Span::current());
            propagator.inject_context(&cx, &mut HeaderInjector(&mut agent_headers));
        });

        agent_headers.insert(
            ARCH_UPSTREAM_HOST_HEADER,
            hyper::header::HeaderValue::from_str(&terminal_agent.id)
                .map_err(|_| PipelineError::AgentNotFound(terminal_agent.id.clone()))?,
        );

        agent_headers.insert(
            ENVOY_RETRY_HEADER,
            hyper::header::HeaderValue::from_str("3").unwrap(),
        );

        let response = self
            .client
            .post(format!("{}{}", self.url, request_url))
            .headers(agent_headers)
            .body(request_body)
            .send()
            .await?;

        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hermesllm::apis::openai::{Message, MessageContent, Role};
    use mockito::Server;
    use std::collections::HashMap;

    fn create_test_message(role: Role, content: &str) -> Message {
        Message {
            role,
            content: Some(MessageContent::Text(content.to_string())),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    fn create_test_pipeline(agents: Vec<&str>) -> AgentFilterChain {
        AgentFilterChain {
            id: "test-agent".to_string(),
            filter_chain: Some(agents.iter().map(|s| s.to_string()).collect()),
            description: None,
            default: None,
        }
    }

    #[tokio::test]
    async fn test_agent_not_found_error() {
        let mut processor = PipelineProcessor::default();
        let agent_map = HashMap::new();
        let request_headers = HeaderMap::new();

        let messages = vec![create_test_message(Role::User, "Hello")];

        let pipeline = create_test_pipeline(vec!["nonexistent-agent", "terminal-agent"]);

        let result = processor
            .process_filter_chain(&messages, &pipeline, &agent_map, &request_headers)
            .await;

        assert!(result.is_err());
        matches!(result.unwrap_err(), PipelineError::AgentNotFound(_));
    }

    #[tokio::test]
    async fn test_execute_filter_http_status_error() {
        let mut server = Server::new_async().await;
        let _m = server
            .mock("POST", "/mcp")
            .with_status(500)
            .with_body("boom")
            .create();

        let server_url = server.url();
        let mut processor = PipelineProcessor::new(server_url.clone());
        processor
            .agent_id_session_map
            .insert("agent-1".to_string(), "session-1".to_string());

        let agent = Agent {
            id: "agent-1".to_string(),
            transport: None,
            tool: None,
            url: server_url,
            agent_type: None,
        };

        let messages = vec![create_test_message(Role::User, "Hello")];
        let request_headers = HeaderMap::new();

        let result = processor
            .execute_mcp_filter(&messages, &agent, &request_headers)
            .await;

        match result {
            Err(PipelineError::ServerError { status, body, .. }) => {
                assert_eq!(status, 500);
                assert_eq!(body, "boom");
            }
            _ => panic!("Expected server error for 500 status"),
        }
    }

    #[tokio::test]
    async fn test_execute_filter_http_client_error() {
        let mut server = Server::new_async().await;
        let _m = server
            .mock("POST", "/mcp")
            .with_status(400)
            .with_body("bad request")
            .create();

        let server_url = server.url();
        let mut processor = PipelineProcessor::new(server_url.clone());
        processor
            .agent_id_session_map
            .insert("agent-3".to_string(), "session-3".to_string());

        let agent = Agent {
            id: "agent-3".to_string(),
            transport: None,
            tool: None,
            url: server_url,
            agent_type: None,
        };

        let messages = vec![create_test_message(Role::User, "Ping")];
        let request_headers = HeaderMap::new();

        let result = processor
            .execute_mcp_filter(&messages, &agent, &request_headers)
            .await;

        match result {
            Err(PipelineError::ClientError { status, body, .. }) => {
                assert_eq!(status, 400);
                assert_eq!(body, "bad request");
            }
            _ => panic!("Expected client error for 400 status"),
        }
    }

    #[tokio::test]
    async fn test_execute_filter_mcp_error_flag() {
        let rpc_body = serde_json::json!({
            "jsonrpc": JSON_RPC_VERSION,
            "id": "1",
            "result": {
                "isError": true,
                "content": [
                    { "text": "bad tool call" }
                ]
            }
        });

        let sse_body = format!("event: message\ndata: {}\n\n", rpc_body);

        let mut server = Server::new_async().await;
        let _m = server
            .mock("POST", "/mcp")
            .with_status(200)
            .with_body(sse_body)
            .create();

        let server_url = server.url();
        let mut processor = PipelineProcessor::new(server_url.clone());
        processor
            .agent_id_session_map
            .insert("agent-2".to_string(), "session-2".to_string());

        let agent = Agent {
            id: "agent-2".to_string(),
            transport: None,
            tool: None,
            url: server_url,
            agent_type: None,
        };

        let messages = vec![create_test_message(Role::User, "Hi")];
        let request_headers = HeaderMap::new();

        let result = processor
            .execute_mcp_filter(&messages, &agent, &request_headers)
            .await;

        match result {
            Err(PipelineError::ClientError { status, body, .. }) => {
                assert_eq!(status, 400);
                assert_eq!(body, "bad tool call");
            }
            _ => panic!("Expected client error when isError flag is set"),
        }
    }
}
