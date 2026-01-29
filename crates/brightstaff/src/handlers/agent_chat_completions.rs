use std::sync::Arc;

use bytes::Bytes;
use common::consts::TRACE_PARENT_HEADER;
use hermesllm::apis::OpenAIMessage;
use hermesllm::clients::SupportedAPIsFromClient;
use hermesllm::providers::request::ProviderRequest;
use hermesllm::ProviderRequestType;
use http_body_util::combinators::BoxBody;
use http_body_util::BodyExt;
use hyper::{Request, Response};
use serde::ser::Error as SerError;
use tracing::{debug, info, instrument, warn};

use super::agent_selector::{AgentSelectionError, AgentSelector};
use super::pipeline_processor::{PipelineError, PipelineProcessor};
use super::response_handler::ResponseHandler;
use crate::router::plano_orchestrator::OrchestratorService;

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
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    match handle_agent_chat(request, orchestrator_service, agents_list, listeners).await {
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
                    "Client error from agent '{}' (HTTP {}): {}",
                    agent, status, body
                );

                // Create error response with the original status code and body
                let error_json = serde_json::json!({
                    "error": "ClientError",
                    "agent": agent,
                    "status": status,
                    "agent_response": body
                });

                let json_string = error_json.to_string();
                let mut response = Response::new(ResponseHandler::create_full_body(json_string));
                *response.status_mut() =
                    hyper::StatusCode::from_u16(*status).unwrap_or(hyper::StatusCode::BAD_REQUEST);
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
            warn!("Agent chat error chain: {:#?}", error_chain);
            warn!("Root error: {:?}", err);

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
            info!("Structured error info: {}", error_json);

            // Return JSON error response
            Ok(ResponseHandler::create_json_error_response(&error_json))
        }
    }
}

#[instrument(
    name = "agent_chat_handler",
    skip(request, orchestrator_service, agents_list, listeners),
    level = "info",
    fields(
        http.method = %request.method(),
        http.path = %request.uri().path()
    )
)]
async fn handle_agent_chat(
    request: Request<hyper::body::Incoming>,
    orchestrator_service: Arc<OrchestratorService>,
    agents_list: Arc<tokio::sync::RwLock<Option<Vec<common::configuration::Agent>>>>,
    listeners: Arc<tokio::sync::RwLock<Vec<common::configuration::Listener>>>,
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
    let listener = {
        let listeners = listeners.read().await;
        agent_selector
            .find_listener(listener_name, &listeners)
            .await?
    };

    info!("Handling request for listener: {}", listener.name);

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

        if !headers.contains_key(common::consts::REQUEST_ID_HEADER) {
            let request_id = uuid::Uuid::new_v4().to_string();
            info!(
                "Request id not found in headers, generated new request id: {}",
                request_id
            );
            headers.insert(
                common::consts::REQUEST_ID_HEADER,
                hyper::header::HeaderValue::from_str(&request_id).unwrap(),
            );
        }

        headers
    };

    let chat_request_bytes = request.collect().await?.to_bytes();

    debug!(
        "Received request body (raw utf8): {}",
        String::from_utf8_lossy(&chat_request_bytes)
    );

    // Determine the API type from the endpoint
    let api_type =
        SupportedAPIsFromClient::from_endpoint(request_path.as_str()).ok_or_else(|| {
            let err_msg = format!("Unsupported endpoint: {}", request_path);
            warn!("{}", err_msg);
            AgentFilterChainError::RequestParsing(serde_json::Error::custom(err_msg))
        })?;

    let client_request = match ProviderRequestType::try_from((&chat_request_bytes[..], &api_type)) {
        Ok(request) => request,
        Err(err) => {
            warn!("Failed to parse request as ProviderRequestType: {}", err);
            let err_msg = format!("Failed to parse request: {}", err);
            return Err(AgentFilterChainError::RequestParsing(
                serde_json::Error::custom(err_msg),
            ));
        }
    };

    let message: Vec<OpenAIMessage> = client_request.get_messages();

    // Extract trace parent for routing
    let traceparent = request_headers
        .iter()
        .find(|(key, _)| key.as_str() == TRACE_PARENT_HEADER)
        .map(|(_, value)| value.to_str().unwrap_or_default().to_string());

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
    let selected_agents = agent_selector
        .select_agents(&message, &listener, traceparent.clone(), request_id.clone())
        .await?;

    info!("Selected {} agent(s) for execution", selected_agents.len());

    // Execute agents sequentially, passing output from one to the next
    let mut current_messages = message.clone();
    let agent_count = selected_agents.len();

    for (agent_index, selected_agent) in selected_agents.iter().enumerate() {
        let is_last_agent = agent_index == agent_count - 1;

        debug!(
            "Processing agent {}/{}: {}",
            agent_index + 1,
            agent_count,
            selected_agent.id
        );

        // Get agent name
        let agent_name = selected_agent.id.clone();

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

        debug!("Invoking agent: {}", agent_name);

        let llm_response = pipeline_processor
            .invoke_agent(
                &chat_history,
                client_request.clone(),
                agent,
                &request_headers,
            )
            .await?;

        // If this is the last agent, return the streaming response
        if is_last_agent {
            info!(
                "Completed agent chain, returning response from last agent: {}",
                agent_name
            );
            return response_handler
                .create_streaming_response(llm_response)
                .await
                .map_err(AgentFilterChainError::from);
        }

        // For intermediate agents, collect the full response and pass to next agent
        debug!(
            "Collecting response from intermediate agent: {}",
            agent_name
        );
        let response_text = response_handler.collect_full_response(llm_response).await?;

        info!(
            "Agent {} completed, passing {} character response to next agent",
            agent_name,
            response_text.len()
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
