use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use hermesllm::apis::OpenAIMessage;
use hermesllm::clients::SupportedAPIsFromClient;
use hermesllm::providers::request::ProviderRequest;
use hermesllm::ProviderRequestType;
use http_body_util::combinators::BoxBody;
use http_body_util::BodyExt;
use hyper::{Request, Response};
use opentelemetry::trace::get_active_span;
use serde::ser::Error as SerError;
use tracing::{debug, info, info_span, warn, Instrument};

use super::pipeline::{PipelineError, PipelineProcessor};
use super::selector::{AgentSelectionError, AgentSelector};
use crate::app_state::AppState;
use crate::handlers::errors::build_error_chain_response;
use crate::handlers::request::extract_request_id;
use crate::handlers::response::ResponseHandler;
use crate::tracing::{collect_custom_trace_attributes, operation_component, set_service_name};

/// Main errors for agent chat completions
#[derive(Debug, thiserror::Error)]
pub enum AgentFilterChainError {
    #[error("Agent selection error: {0}")]
    Selection(#[from] AgentSelectionError),
    #[error("Pipeline processing error: {0}")]
    Pipeline(#[from] PipelineError),
    #[error("Response handling error: {0}")]
    Response(#[from] common::errors::BrightStaffError),
    #[error("Request parsing error: {0}")]
    RequestParsing(#[from] serde_json::Error),
    #[error("HTTP error: {0}")]
    Http(#[from] hyper::Error),
}

pub async fn agent_chat(
    request: Request<hyper::body::Incoming>,
    state: Arc<AppState>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let request_id = extract_request_id(&request);
    let custom_attrs =
        collect_custom_trace_attributes(request.headers(), state.span_attributes.as_ref().as_ref());

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

        match handle_agent_chat_inner(request, state, request_id, custom_attrs).await {
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
                        .unwrap_or(hyper::StatusCode::INTERNAL_SERVER_ERROR);
                    response.headers_mut().insert(
                        hyper::header::CONTENT_TYPE,
                        hyper::header::HeaderValue::from_static("application/json"),
                    );
                    return Ok(response);
                }

                build_error_chain_response(&err)
            }
        }
    }
    .instrument(request_span)
    .await
}

/// Parsed and validated agent request data.
struct AgentRequest {
    client_request: ProviderRequestType,
    messages: Vec<OpenAIMessage>,
    request_headers: hyper::HeaderMap,
    request_id: Option<String>,
}

/// Parse the incoming HTTP request, resolve the listener, and extract messages.
async fn parse_agent_request(
    request: Request<hyper::body::Incoming>,
    state: &AppState,
    request_id: &str,
    custom_attrs: &std::collections::HashMap<String, String>,
) -> Result<(AgentRequest, common::configuration::Listener, AgentSelector), AgentFilterChainError> {
    let agent_selector = AgentSelector::new(Arc::clone(&state.orchestrator_service));

    // Extract listener name from headers
    let listener_name = request
        .headers()
        .get("x-arch-agent-listener-name")
        .and_then(|name| name.to_str().ok());

    // Find the appropriate listener
    let listener: common::configuration::Listener = {
        let listeners = state.listeners.read().await;
        agent_selector
            .find_listener(listener_name, &listeners)
            .await?
    };

    get_active_span(|span| {
        span.update_name(listener.name.to_string());
        for (key, value) in custom_attrs {
            span.set_attribute(opentelemetry::KeyValue::new(key.clone(), value.clone()));
        }
    });

    info!(listener = %listener.name, "handling request");

    // Parse request body
    let full_path = request.uri().path().to_string();
    let request_path = full_path
        .strip_prefix("/agents")
        .unwrap_or(&full_path)
        .to_string();

    let request_headers = {
        let mut headers = request.headers().clone();
        headers.remove(common::consts::ENVOY_ORIGINAL_PATH_HEADER);

        if !headers.contains_key(common::consts::REQUEST_ID_HEADER) {
            if let Ok(val) = hyper::header::HeaderValue::from_str(request_id) {
                headers.insert(common::consts::REQUEST_ID_HEADER, val);
            }
        }

        headers
    };

    let chat_request_bytes = request.collect().await?.to_bytes();

    debug!(
        body = %String::from_utf8_lossy(&chat_request_bytes),
        "received request body"
    );

    let api_type =
        SupportedAPIsFromClient::from_endpoint(request_path.as_str()).ok_or_else(|| {
            let err_msg = format!("Unsupported endpoint: {}", request_path);
            warn!("{}", err_msg);
            AgentFilterChainError::RequestParsing(serde_json::Error::custom(err_msg))
        })?;

    let client_request = ProviderRequestType::try_from((&chat_request_bytes[..], &api_type))
        .map_err(|err| {
            warn!("failed to parse request as ProviderRequestType: {}", err);
            AgentFilterChainError::RequestParsing(serde_json::Error::custom(format!(
                "Failed to parse request: {}",
                err
            )))
        })?;

    let messages: Vec<OpenAIMessage> = client_request.get_messages();

    let request_id = request_headers
        .get(common::consts::REQUEST_ID_HEADER)
        .and_then(|val| val.to_str().ok())
        .map(|s| s.to_string());

    Ok((
        AgentRequest {
            client_request,
            messages,
            request_headers,
            request_id,
        },
        listener,
        agent_selector,
    ))
}

/// Select agents via the orchestrator model and record selection metrics.
async fn select_and_build_agent_map(
    agent_selector: &AgentSelector,
    state: &AppState,
    messages: &[OpenAIMessage],
    listener: &common::configuration::Listener,
    request_id: Option<String>,
) -> Result<
    (
        Vec<common::configuration::AgentFilterChain>,
        std::collections::HashMap<String, common::configuration::Agent>,
    ),
    AgentFilterChainError,
> {
    let agent_map = {
        let agents = state.agents_list.read().await;
        let agents = agents.as_ref().ok_or_else(|| {
            AgentFilterChainError::RequestParsing(serde_json::Error::custom("No agents configured"))
        })?;
        agent_selector.create_agent_map(agents)
    };

    let selection_start = Instant::now();
    let selected_agents = agent_selector
        .select_agents(messages, listener, request_id)
        .await?;

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

    Ok((selected_agents, agent_map))
}

/// Execute the agent chain: run each selected agent sequentially, streaming
/// the final agent's response back to the client.
async fn execute_agent_chain(
    selected_agents: &[common::configuration::AgentFilterChain],
    agent_map: &std::collections::HashMap<String, common::configuration::Agent>,
    client_request: ProviderRequestType,
    messages: Vec<OpenAIMessage>,
    request_headers: &hyper::HeaderMap,
    custom_attrs: &std::collections::HashMap<String, String>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, AgentFilterChainError> {
    let mut pipeline_processor = PipelineProcessor::default();
    let response_handler = ResponseHandler::new();
    let mut current_messages = messages;
    let agent_count = selected_agents.len();

    for (agent_index, selected_agent) in selected_agents.iter().enumerate() {
        let agent_name = selected_agent.id.clone();
        let is_last_agent = agent_index == agent_count - 1;

        debug!(
            agent_index = agent_index + 1,
            total = agent_count,
            agent = %agent_name,
            "processing agent"
        );

        let chat_history = if selected_agent
            .input_filters
            .as_ref()
            .map(|f| !f.is_empty())
            .unwrap_or(false)
        {
            let filter_body = serde_json::json!({
                "model": client_request.model(),
                "messages": current_messages,
            });
            let filter_bytes =
                serde_json::to_vec(&filter_body).map_err(PipelineError::ParseError)?;

            let filtered_bytes = pipeline_processor
                .process_raw_filter_chain(
                    &filter_bytes,
                    selected_agent,
                    agent_map,
                    request_headers,
                    "/v1/chat/completions",
                )
                .await?;

            let filtered_body: serde_json::Value =
                serde_json::from_slice(&filtered_bytes).map_err(PipelineError::ParseError)?;
            serde_json::from_value(filtered_body["messages"].clone())
                .map_err(PipelineError::ParseError)?
        } else {
            current_messages.clone()
        };

        let agent = agent_map.get(&agent_name).ok_or_else(|| {
            AgentFilterChainError::RequestParsing(serde_json::Error::custom(format!(
                "Selected agent '{}' not found in configuration",
                agent_name
            )))
        })?;

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
                for (key, value) in custom_attrs {
                    span.set_attribute(opentelemetry::KeyValue::new(key.clone(), value.clone()));
                }
            });

            pipeline_processor
                .invoke_agent(
                    &chat_history,
                    client_request.clone(),
                    agent,
                    request_headers,
                )
                .await
        }
        .instrument(agent_span.clone())
        .await?;

        if is_last_agent {
            info!(
                agent = %agent_name,
                "completed agent chain, returning response"
            );
            let orchestrator_span = tracing::Span::current();
            return async {
                response_handler
                    .create_streaming_response(
                        llm_response,
                        tracing::Span::current(),
                        orchestrator_span,
                    )
                    .await
                    .map_err(AgentFilterChainError::from)
            }
            .instrument(agent_span)
            .await;
        }

        debug!(agent = %agent_name, "collecting response from intermediate agent");
        let response_text = async { response_handler.collect_full_response(llm_response).await }
            .instrument(agent_span)
            .await?;

        info!(
            agent = %agent_name,
            response_len = response_text.len(),
            "agent completed, passing response to next agent"
        );

        let Some(last_message) = current_messages.pop() else {
            warn!(agent = %agent_name, "no messages in conversation history");
            break;
        };

        current_messages.push(OpenAIMessage {
            role: hermesllm::apis::openai::Role::Assistant,
            content: Some(hermesllm::apis::openai::MessageContent::Text(response_text)),
            name: Some(agent_name.clone()),
            tool_calls: None,
            tool_call_id: None,
        });

        current_messages.push(last_message);
    }

    unreachable!("Agent execution loop should have returned a response")
}

async fn handle_agent_chat_inner(
    request: Request<hyper::body::Incoming>,
    state: Arc<AppState>,
    request_id: String,
    custom_attrs: std::collections::HashMap<String, String>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, AgentFilterChainError> {
    let (agent_req, listener, agent_selector) =
        parse_agent_request(request, &state, &request_id, &custom_attrs).await?;

    let (selected_agents, agent_map) = select_and_build_agent_map(
        &agent_selector,
        &state,
        &agent_req.messages,
        &listener,
        agent_req.request_id,
    )
    .await?;

    execute_agent_chain(
        &selected_agents,
        &agent_map,
        agent_req.client_request,
        agent_req.messages,
        &agent_req.request_headers,
        &custom_attrs,
    )
    .await
}
