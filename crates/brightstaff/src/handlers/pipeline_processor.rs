use std::collections::HashMap;

use common::configuration::{Agent, AgentFilterChain};
use common::consts::{
    ARCH_UPSTREAM_HOST_HEADER, BRIGHT_STAFF_SERVICE_NAME, ENVOY_RETRY_HEADER, TRACE_PARENT_HEADER,
};
use common::traces::{generate_random_span_id, SpanBuilder, SpanKind};
use hermesllm::apis::openai::Message;
use hermesllm::{ProviderRequest, ProviderRequestType};
use hyper::header::HeaderMap;
use std::time::{Instant, SystemTime};
use tracing::{debug, info, warn};

use crate::tracing::operation_component::{self};
use crate::tracing::{http, OperationNameBuilder};

use crate::handlers::jsonrpc::{
    JsonRpcId, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, JSON_RPC_VERSION,
    MCP_INITIALIZE, MCP_INITIALIZE_NOTIFICATION, TOOL_CALL_METHOD,
};
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

    /// Record a span for filter execution
    #[allow(clippy::too_many_arguments)]
    fn record_filter_span(
        &self,
        collector: &std::sync::Arc<common::traces::TraceCollector>,
        agent_name: &str,
        tool_name: &str,
        start_time: SystemTime,
        end_time: SystemTime,
        elapsed: std::time::Duration,
        trace_id: String,
        parent_span_id: String,
        span_id: String,
    ) -> String {
        // let (trace_id, parent_span_id) = self.extract_trace_context();

        // Build operation name: POST /agents/* {filter_name}
        // Using generic path since we don't have access to specific endpoint here
        let operation_name = OperationNameBuilder::new()
            .with_method("POST")
            .with_path("/agents/*")
            .with_target(agent_name)
            .build();

        let mut span_builder = SpanBuilder::new(&operation_name)
            .with_span_id(span_id.clone())
            .with_kind(SpanKind::Client)
            .with_start_time(start_time)
            .with_end_time(end_time)
            .with_attribute(http::METHOD, "POST")
            .with_attribute(http::TARGET, "/agents/*")
            .with_attribute("filter.name", agent_name.to_string())
            .with_attribute("filter.tool_name", tool_name.to_string())
            .with_attribute(
                "duration_ms",
                format!("{:.2}", elapsed.as_secs_f64() * 1000.0),
            );

        if !trace_id.is_empty() {
            span_builder = span_builder.with_trace_id(trace_id);
        }
        if !parent_span_id.is_empty() {
            span_builder = span_builder.with_parent_span_id(parent_span_id);
        }

        let span = span_builder.build();
        // Use plano(filter) as service name for filter execution spans
        collector.record_span(operation_component::AGENT_FILTER, span);
        span_id.clone()
    }

    /// Record a span for MCP protocol interactions
    #[allow(clippy::too_many_arguments)]
    fn record_agent_filter_span(
        &self,
        collector: &std::sync::Arc<common::traces::TraceCollector>,
        operation: &str,
        agent_id: &str,
        start_time: SystemTime,
        end_time: SystemTime,
        elapsed: std::time::Duration,
        additional_attrs: Option<HashMap<&str, String>>,
        trace_id: String,
        parent_span_id: String,
        span_id: Option<String>,
    ) {
        // let (trace_id, parent_span_id) = self.extract_trace_context();

        // Build operation name: POST /mcp {agent_id}
        let operation_name = OperationNameBuilder::new()
            .with_method("POST")
            .with_path("/mcp")
            .with_operation(operation)
            .with_target(agent_id)
            .build();

        let mut span_builder = SpanBuilder::new(&operation_name)
            .with_span_id(span_id.unwrap_or_else(generate_random_span_id))
            .with_kind(SpanKind::Client)
            .with_start_time(start_time)
            .with_end_time(end_time)
            .with_attribute(http::METHOD, "POST")
            .with_attribute(http::TARGET, format!("/mcp ({})", operation))
            .with_attribute("mcp.operation", operation.to_string())
            .with_attribute("mcp.agent_id", agent_id.to_string())
            .with_attribute(
                "duration_ms",
                format!("{:.2}", elapsed.as_secs_f64() * 1000.0),
            );

        if let Some(attrs) = additional_attrs {
            for (key, value) in attrs {
                span_builder = span_builder.with_attribute(key, value);
            }
        }

        if !trace_id.is_empty() {
            span_builder = span_builder.with_trace_id(trace_id);
        }
        if !parent_span_id.is_empty() {
            span_builder = span_builder.with_parent_span_id(parent_span_id);
        }

        let span = span_builder.build();
        // MCP spans also use plano(filter) service name as they are part of filter operations
        collector.record_span(operation_component::AGENT_FILTER, span);
    }

    /// Process the filter chain of agents (all except the terminal agent)
    #[allow(clippy::too_many_arguments)]
    pub async fn process_filter_chain(
        &mut self,
        chat_history: &[Message],
        agent_filter_chain: &AgentFilterChain,
        agent_map: &HashMap<String, Agent>,
        request_headers: &HeaderMap,
        trace_collector: Option<&std::sync::Arc<common::traces::TraceCollector>>,
        trace_id: String,
        parent_span_id: String,
    ) -> Result<Vec<Message>, PipelineError> {
        let mut chat_history_updated = chat_history.to_vec();

        // If filter_chain is None or empty, proceed without filtering
        let filter_chain = match agent_filter_chain.filter_chain.as_ref() {
            Some(fc) if !fc.is_empty() => fc,
            _ => return Ok(chat_history_updated),
        };

        for agent_name in filter_chain {
            debug!("Processing filter agent: {}", agent_name);

            let agent = agent_map
                .get(agent_name)
                .ok_or_else(|| PipelineError::AgentNotFound(agent_name.clone()))?;

            let tool_name = agent.tool.as_deref().unwrap_or(&agent.id);

            info!(
                "executing filter: {}/{}, url: {}, type: {}, conversation length: {}",
                agent_name,
                tool_name,
                agent.url,
                agent.agent_type.as_deref().unwrap_or("mcp"),
                chat_history.len()
            );

            let start_time = SystemTime::now();
            let start_instant = Instant::now();

            // Generate filter span ID before execution so MCP spans can use it as parent
            let filter_span_id = generate_random_span_id();

            if agent.agent_type.as_deref().unwrap_or("mcp") == "mcp" {
                chat_history_updated = self
                    .execute_mcp_filter(
                        &chat_history_updated,
                        agent,
                        request_headers,
                        trace_collector,
                        trace_id.clone(),
                        filter_span_id.clone(),
                    )
                    .await?;
            } else {
                chat_history_updated = self
                    .execute_http_filter(
                        &chat_history_updated,
                        agent,
                        request_headers,
                        trace_collector,
                        trace_id.clone(),
                        filter_span_id.clone(),
                    )
                    .await?;
            }

            let end_time = SystemTime::now();
            let elapsed = start_instant.elapsed();

            info!(
                "Filter '{}' completed in {:.2}ms, updated conversation length: {}",
                agent_name,
                elapsed.as_secs_f64() * 1000.0,
                chat_history_updated.len()
            );

            // Record span for this filter execution
            if let Some(collector) = trace_collector {
                self.record_filter_span(
                    collector,
                    agent_name,
                    tool_name,
                    start_time,
                    end_time,
                    elapsed,
                    trace_id.clone(),
                    parent_span_id.clone(),
                    filter_span_id,
                );
            }
        }

        Ok(chat_history_updated)
    }

    /// Build common MCP headers for requests
    fn build_mcp_headers(
        &self,
        request_headers: &HeaderMap,
        agent_id: &str,
        session_id: Option<&str>,
        trace_id: String,
        parent_span_id: String,
    ) -> Result<HeaderMap, PipelineError> {
        let trace_parent = format!("00-{}-{}-01", trace_id, parent_span_id);
        let mut headers = request_headers.clone();
        headers.remove(hyper::header::CONTENT_LENGTH);

        headers.remove(TRACE_PARENT_HEADER);
        headers.insert(
            TRACE_PARENT_HEADER,
            hyper::header::HeaderValue::from_str(&trace_parent).unwrap(),
        );

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
                "Invalid SSE response format from agent {}: expected 'event: message' as first line, got: {:?}",
                agent_id,
                lines.first()
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
                "Expected exactly one 'data:' line from agent {}, found {}",
                agent_id,
                data_lines.len()
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
    async fn execute_mcp_filter(
        &mut self,
        messages: &[Message],
        agent: &Agent,
        request_headers: &HeaderMap,
        trace_collector: Option<&std::sync::Arc<common::traces::TraceCollector>>,
        trace_id: String,
        filter_span_id: String,
    ) -> Result<Vec<Message>, PipelineError> {
        // Get or create MCP session
        let mcp_session_id = if let Some(session_id) = self.agent_id_session_map.get(&agent.id) {
            session_id.clone()
        } else {
            let session_id = self
                .get_new_session_id(
                    &agent.id,
                    trace_id.clone(),
                    filter_span_id.clone(),
                    request_headers,
                )
                .await;
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

        // Generate span ID for this MCP tool call (child of filter span)
        let mcp_span_id = generate_random_span_id();

        // Build headers
        let agent_headers = self.build_mcp_headers(
            request_headers,
            &agent.id,
            Some(&mcp_session_id),
            trace_id.clone(),
            mcp_span_id.clone(),
        )?;

        // Send request with tracing
        let start_time = SystemTime::now();
        let start_instant = Instant::now();

        let response = self
            .send_mcp_request(&json_rpc_request, &agent_headers, &agent.id)
            .await?;
        let http_status = response.status();
        let response_bytes = response.bytes().await?;

        let end_time = SystemTime::now();
        let elapsed = start_instant.elapsed();

        // Record MCP tool call span
        if let Some(collector) = trace_collector {
            let mut attrs = HashMap::new();
            attrs.insert("mcp.method", "tools/call".to_string());
            attrs.insert("mcp.tool_name", tool_name.to_string());
            attrs.insert("mcp.session_id", mcp_session_id.clone());
            attrs.insert("http.status_code", http_status.as_u16().to_string());

            self.record_agent_filter_span(
                collector,
                "tool_call",
                &agent.id,
                start_time,
                end_time,
                elapsed,
                Some(attrs),
                trace_id.clone(),
                filter_span_id.clone(),
                Some(mcp_span_id),
            );
        }

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
        trace_id: String,
        parent_span_id: String,
        request_headers: &HeaderMap,
    ) -> Result<(), PipelineError> {
        let initialized_notification = JsonRpcNotification {
            jsonrpc: JSON_RPC_VERSION.to_string(),
            method: MCP_INITIALIZE_NOTIFICATION.to_string(),
            params: None,
        };

        let notification_body = serde_json::to_string(&initialized_notification)?;
        debug!("Sending initialized notification for agent {}", agent_id);

        let headers = self.build_mcp_headers(
            request_headers,
            agent_id,
            Some(session_id),
            trace_id.clone(),
            parent_span_id.clone(),
        )?;

        let response = self
            .client
            .post(format!("{}/mcp", self.url))
            .headers(headers)
            .body(notification_body)
            .send()
            .await?;

        info!(
            "Initialized notification response status: {}",
            response.status()
        );

        Ok(())
    }

    async fn get_new_session_id(
        &self,
        agent_id: &str,
        trace_id: String,
        parent_span_id: String,
        request_headers: &HeaderMap,
    ) -> String {
        info!("Initializing MCP session for agent {}", agent_id);

        let initialize_request = self.build_initialize_request();
        let headers = self
            .build_mcp_headers(
                request_headers,
                agent_id,
                None,
                trace_id.clone(),
                parent_span_id.clone(),
            )
            .expect("Failed to build headers for initialization");

        let response = self
            .send_mcp_request(&initialize_request, &headers, agent_id)
            .await
            .expect("Failed to initialize MCP session");

        info!("Initialize response status: {}", response.status());

        let session_id = response
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .expect("No mcp-session-id in response")
            .to_string();

        info!(
            "Created new MCP session for agent {}: {}",
            agent_id, session_id
        );

        // Send initialized notification
        self.send_initialized_notification(
            agent_id,
            &session_id,
            trace_id.clone(),
            parent_span_id.clone(),
            &headers,
        )
        .await
        .expect("Failed to send initialized notification");

        session_id
    }

    /// Execute a HTTP-based filter agent
    async fn execute_http_filter(
        &mut self,
        messages: &[Message],
        agent: &Agent,
        request_headers: &HeaderMap,
        trace_collector: Option<&std::sync::Arc<common::traces::TraceCollector>>,
        trace_id: String,
        filter_span_id: String,
    ) -> Result<Vec<Message>, PipelineError> {
        let tool_name = agent.tool.as_deref().unwrap_or(&agent.id);

        // Generate span ID for this HTTP call (child of filter span)
        let http_span_id = generate_random_span_id();

        // Build headers
        let trace_parent = format!("00-{}-{}-01", trace_id, http_span_id);
        let mut agent_headers = request_headers.clone();
        agent_headers.remove(hyper::header::CONTENT_LENGTH);

        agent_headers.remove(TRACE_PARENT_HEADER);
        agent_headers.insert(
            TRACE_PARENT_HEADER,
            hyper::header::HeaderValue::from_str(&trace_parent).unwrap(),
        );

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

        // Send request with tracing
        let start_time = SystemTime::now();
        let start_instant = Instant::now();

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

        let end_time = SystemTime::now();
        let elapsed = start_instant.elapsed();

        // Record HTTP call span
        if let Some(collector) = trace_collector {
            let mut attrs = HashMap::new();
            attrs.insert("http.tool_name", tool_name.to_string());
            attrs.insert("http.url", agent.url.clone());
            attrs.insert("http.status_code", http_status.as_u16().to_string());

            self.record_agent_filter_span(
                collector,
                "http_call",
                &agent.id,
                start_time,
                end_time,
                elapsed,
                Some(attrs),
                trace_id.clone(),
                filter_span_id.clone(),
                Some(http_span_id),
            );
        }

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
    pub async fn invoke_agent(
        &self,
        messages: &[Message],
        mut original_request: ProviderRequestType,
        terminal_agent: &Agent,
        request_headers: &HeaderMap,
        trace_id: String,
        agent_span_id: String,
    ) -> Result<reqwest::Response, PipelineError> {
        // let mut request = original_request.clone();
        original_request.set_messages(messages);

        let request_body = ProviderRequestType::to_bytes(&original_request).unwrap();
        // let request_body = serde_json::to_string(&request)?;
        debug!("Sending request to terminal agent {}", terminal_agent.id);

        let mut agent_headers = request_headers.clone();
        agent_headers.remove(hyper::header::CONTENT_LENGTH);

        // Set traceparent header to make the egress span a child of the agent span
        if !trace_id.is_empty() && !agent_span_id.is_empty() {
            let trace_parent = format!("00-{}-{}-01", trace_id, agent_span_id);
            agent_headers.remove(TRACE_PARENT_HEADER);
            agent_headers.insert(
                TRACE_PARENT_HEADER,
                hyper::header::HeaderValue::from_str(&trace_parent).unwrap(),
            );
        }

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
            .post(format!("{}/v1/chat/completions", self.url))
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
            content: MessageContent::Text(content.to_string()),
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
            .process_filter_chain(
                &messages,
                &pipeline,
                &agent_map,
                &request_headers,
                None,
                String::new(),
                String::new(),
            )
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
            .execute_mcp_filter(
                &messages,
                &agent,
                &request_headers,
                None,
                "trace-123".to_string(),
                "span-123".to_string(),
            )
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
            .execute_mcp_filter(
                &messages,
                &agent,
                &request_headers,
                None,
                "trace-456".to_string(),
                "span-456".to_string(),
            )
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
            .execute_mcp_filter(
                &messages,
                &agent,
                &request_headers,
                None,
                "trace-789".to_string(),
                "span-789".to_string(),
            )
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
