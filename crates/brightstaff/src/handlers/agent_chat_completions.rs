use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use common::llm_providers::LlmProviders;
use hermesllm::apis::OpenAIMessage;
use hermesllm::clients::SupportedAPIsFromClient;
use hermesllm::providers::request::ProviderRequest;
use hermesllm::ProviderRequestType;
use http_body_util::combinators::BoxBody;
use http_body_util::BodyExt;
use hyper::{Request, Response, StatusCode};
use opentelemetry::trace::get_active_span;
use serde::ser::Error as SerError;
use tokio::sync::RwLock;
use tracing::{debug, info, info_span, warn, Instrument};

use super::agent_selector::{AgentSelectionError, AgentSelector};
use super::pipeline_processor::{PipelineError, PipelineProcessor};
use super::response_handler::ResponseHandler;
use crate::router::plano_orchestrator::OrchestratorService;
use crate::tracing::{operation_component, set_service_name};

/// Main errors for agent chat completions
#[derive(Debug, thiserror::Error)]
pub enum AgentFilterChainError {
    #[error("Agent selection error: {0}")]
    Selection(#[from] AgentSelectionError),
    #[error("Pipeline processing error: {0}")]
    Pipeline(#[from] PipelineError),
    #[error("Response handling error: {0}")]
    Response(#[from] super::response_handler::ResponseError),
    #[error("Request parsing error: {0}")]
    RequestParsing(#[from] serde_json::Error),
    #[error("HTTP error: {0}")]
    Http(#[from] hyper::Error),
}

pub async fn agent_chat(
    request: Request<hyper::body::Incoming>,
    orchestrator_service: Arc<OrchestratorService>,
    _: String,
    agents_list: Arc<tokio::sync::RwLock<Option<Vec<common::configuration::Agent>>>>,
    listeners: Arc<tokio::sync::RwLock<Vec<common::configuration::Listener>>>,
    llm_providers: Arc<RwLock<LlmProviders>>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    // Extract request_id from headers or generate a new one
    let request_id: String = match request
        .headers()
        .get(common::consts::REQUEST_ID_HEADER)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
    {
        Some(id) => id,
        None => uuid::Uuid::new_v4().to_string(),
    };

    // Create a span with request_id that will be included in all log lines
    let request_span = info_span!(
        "(orchestrator)",
        component = "orchestrator",
        request_id = %request_id,
        http.method = %request.method(),
        http.path = %request.uri().path()
    );

    // Execute the handler inside the span
    async {
        // Set service name for orchestrator operations
        set_service_name(operation_component::ORCHESTRATOR);

        match handle_agent_chat_inner(
            request,
            orchestrator_service,
            agents_list,
            listeners,
            llm_providers,
            request_id,
        )
        .await
        {
            Ok(response) => Ok(response),
            Err(err) => {
                // Check if this is a client error from the pipeline that should be cascaded
                if let AgentFilterChainError::Pipeline(PipelineError::ClientError {
                    agent,
                    status,
                    body,
                }) = &err
                {
                    warn!(
                        agent = %agent,
                        status = %status,
                        body = %body,
                        "client error from agent"
                    );

                    // Create error response with the original status code and body
                    let error_json = serde_json::json!({
                        "error": "ClientError",
                        "agent": agent,
                        "status": status,
                        "agent_response": body
                    });

                    let json_string = error_json.to_string();
                    let mut response =
                        Response::new(ResponseHandler::create_full_body(json_string));
                    *response.status_mut() = hyper::StatusCode::from_u16(*status)
                        .unwrap_or(hyper::StatusCode::BAD_REQUEST);
                    response.headers_mut().insert(
                        hyper::header::CONTENT_TYPE,
                        "application/json".parse().unwrap(),
                    );
                    return Ok(response);
                }

                // Print detailed error information with full error chain for other errors
                let mut error_chain = Vec::new();
                let mut current_error: &dyn std::error::Error = &err;

                // Collect the full error chain
                loop {
                    error_chain.push(current_error.to_string());
                    match current_error.source() {
                        Some(source) => current_error = source,
                        None => break,
                    }
                }

                // Log the complete error chain
                warn!(error_chain = ?error_chain, "agent chat error chain");
                warn!(root_error = ?err, "root error");

                // Create structured error response as JSON
                let error_json = serde_json::json!({
                    "error": {
                        "type": "AgentFilterChainError",
                        "message": err.to_string(),
                        "error_chain": error_chain,
                        "debug_info": format!("{:?}", err)
                    }
                });

                // Log the error for debugging
                info!(error = %error_json, "structured error info");

                // Return JSON error response
                Ok(ResponseHandler::create_json_error_response(&error_json))
            }
        }
    }
    .instrument(request_span)
    .await
}

async fn handle_agent_chat_inner(
    request: Request<hyper::body::Incoming>,
    orchestrator_service: Arc<OrchestratorService>,
    agents_list: Arc<tokio::sync::RwLock<Option<Vec<common::configuration::Agent>>>>,
    listeners: Arc<tokio::sync::RwLock<Vec<common::configuration::Listener>>>,
    llm_providers: Arc<RwLock<LlmProviders>>,
    request_id: String,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, AgentFilterChainError> {
    // Initialize services
    let agent_selector = AgentSelector::new(orchestrator_service);
    let mut pipeline_processor = PipelineProcessor::default();
    let response_handler = ResponseHandler::new();

    // Extract listener name from headers
    let listener_name = request
        .headers()
        .get("x-arch-agent-listener-name")
        .and_then(|name| name.to_str().ok());

    // Find the appropriate listener
    let listener: common::configuration::Listener = {
        let listeners = listeners.read().await;
        agent_selector
            .find_listener(listener_name, &listeners)
            .await?
    };

    get_active_span(|span| {
        span.update_name(listener.name.to_string());
    });

    info!(listener = %listener.name, "handling request");

    // Parse request body
    let request_path = request
        .uri()
        .path()
        .to_string()
        .strip_prefix("/agents")
        .unwrap()
        .to_string();

    let request_headers = {
        let mut headers = request.headers().clone();
        headers.remove(common::consts::ENVOY_ORIGINAL_PATH_HEADER);

        // Set the request_id in headers if not already present
        if !headers.contains_key(common::consts::REQUEST_ID_HEADER) {
            headers.insert(
                common::consts::REQUEST_ID_HEADER,
                hyper::header::HeaderValue::from_str(&request_id).unwrap(),
            );
        }

        headers
    };

    let chat_request_bytes = request.collect().await?.to_bytes();

    debug!(
        body = %String::from_utf8_lossy(&chat_request_bytes),
        "received request body"
    );

    // Determine the API type from the endpoint
    let api_type =
        SupportedAPIsFromClient::from_endpoint(request_path.as_str()).ok_or_else(|| {
            let err_msg = format!("Unsupported endpoint: {}", request_path);
            warn!("{}", err_msg);
            AgentFilterChainError::RequestParsing(serde_json::Error::custom(err_msg))
        })?;

    let mut client_request =
        match ProviderRequestType::try_from((&chat_request_bytes[..], &api_type)) {
            Ok(request) => request,
            Err(err) => {
                warn!("failed to parse request as ProviderRequestType: {}", err);
                let err_msg = format!("Failed to parse request: {}", err);
                return Err(AgentFilterChainError::RequestParsing(
                    serde_json::Error::custom(err_msg),
                ));
            }
        };

    // If model is not specified in the request, resolve from default provider
    if client_request.model().is_empty() {
        match llm_providers.read().await.default() {
            Some(default_provider) => {
                let default_model = default_provider.name.clone();
                info!(default_model = %default_model, "no model specified in request, using default provider");
                client_request.set_model(default_model);
            }
            None => {
                let err_msg = "No model specified in request and no default provider configured";
                warn!("{}", err_msg);
                let mut bad_request =
                    Response::new(ResponseHandler::create_full_body(err_msg.to_string()));
                *bad_request.status_mut() = StatusCode::BAD_REQUEST;
                return Ok(bad_request);
            }
        }
    }

    let message: Vec<OpenAIMessage> = client_request.get_messages();

    let request_id = request_headers
        .get(common::consts::REQUEST_ID_HEADER)
        .and_then(|val| val.to_str().ok())
        .map(|s| s.to_string());

    // Create agent map for pipeline processing and agent selection
    let agent_map = {
        let agents = agents_list.read().await;
        let agents = agents.as_ref().unwrap();
        agent_selector.create_agent_map(agents)
    };

    // Select appropriate agents using arch orchestrator llm model
    let selection_start = Instant::now();
    let selected_agents = agent_selector
        .select_agents(&message, &listener, request_id.clone())
        .await?;

    // Record selection attributes on the current orchestrator span
    let selection_elapsed_ms = selection_start.elapsed().as_secs_f64() * 1000.0;
    get_active_span(|span| {
        span.set_attribute(opentelemetry::KeyValue::new(
            "selection.listener",
            listener.name.clone(),
        ));
        span.set_attribute(opentelemetry::KeyValue::new(
            "selection.agent_count",
            selected_agents.len() as i64,
        ));
        span.set_attribute(opentelemetry::KeyValue::new(
            "selection.agents",
            selected_agents
                .iter()
                .map(|a| a.id.as_str())
                .collect::<Vec<_>>()
                .join(","),
        ));
        span.set_attribute(opentelemetry::KeyValue::new(
            "selection.determination_ms",
            format!("{:.2}", selection_elapsed_ms),
        ));
    });

    info!(
        count = selected_agents.len(),
        "selected agents for execution"
    );

    // Execute agents sequentially, passing output from one to the next
    let mut current_messages = message.clone();
    let agent_count = selected_agents.len();

    for (agent_index, selected_agent) in selected_agents.iter().enumerate() {
        // Get agent name
        let agent_name = selected_agent.id.clone();
        let is_last_agent = agent_index == agent_count - 1;

        debug!(
            agent_index = agent_index + 1,
            total = agent_count,
            agent = %agent_name,
            "processing agent"
        );

        // Process the filter chain
        let chat_history = pipeline_processor
            .process_filter_chain(
                &current_messages,
                selected_agent,
                &agent_map,
                &request_headers,
            )
            .await?;

        // Get agent details and invoke
        let agent = agent_map.get(&agent_name).unwrap();

        debug!(agent = %agent_name, "invoking agent");

        let agent_span = info_span!(
            "agent",
            agent_id = %agent_name,
            message_count = chat_history.len(),
        );

        let llm_response = async {
            set_service_name(operation_component::AGENT);
            get_active_span(|span| {
                span.update_name(format!("{} /v1/chat/completions", agent_name));
            });

            pipeline_processor
                .invoke_agent(
                    &chat_history,
                    client_request.clone(),
                    agent,
                    &request_headers,
                )
                .await
        }
        .instrument(agent_span.clone())
        .await?;

        // If this is the last agent, return the streaming response
        if is_last_agent {
            info!(
                agent = %agent_name,
                "completed agent chain, returning response"
            );
            // Capture the orchestrator span (parent of the agent span) so it
            // stays open for the full streaming duration alongside the agent span.
            let orchestrator_span = tracing::Span::current();
            return async {
                response_handler
                    .create_streaming_response(
                        llm_response,
                        tracing::Span::current(), // agent span (inner)
                        orchestrator_span,        // orchestrator span (outer)
                    )
                    .await
                    .map_err(AgentFilterChainError::from)
            }
            .instrument(agent_span)
            .await;
        }

        // For intermediate agents, collect the full response and pass to next agent
        debug!(agent = %agent_name, "collecting response from intermediate agent");
        let response_text = async { response_handler.collect_full_response(llm_response).await }
            .instrument(agent_span)
            .await?;

        info!(
            agent = %agent_name,
            response_len = response_text.len(),
            "agent completed, passing response to next agent"
        );

        // remove last message and add new one at the end
        let last_message = current_messages.pop().unwrap();

        // Create a new message with the agent's response as assistant message
        // and add it to the conversation history
        current_messages.push(OpenAIMessage {
            role: hermesllm::apis::openai::Role::Assistant,
            content: Some(hermesllm::apis::openai::MessageContent::Text(response_text)),
            name: Some(agent_name.clone()),
            tool_calls: None,
            tool_call_id: None,
        });

        current_messages.push(last_message);
    }

    // This should never be reached since we return in the last agent iteration
    unreachable!("Agent execution loop should have returned a response")
}
