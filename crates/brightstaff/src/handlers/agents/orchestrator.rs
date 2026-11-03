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
use tracing::{debug, info, info_span, warn, Instrument};

use super::aggregator::{
    aggregate_with_model, build_aggregation_messages, concat_replies, last_user_text, AgentReply,
};
use super::errors::build_error_chain_response;
use super::pipeline::{PipelineError, PipelineProcessor};
use super::selector::{AgentSelectionError, AgentSelector};
use crate::app_state::AppState;
use crate::handlers::extract_request_id;
use crate::handlers::response::ResponseHandler;
use crate::tracing::{collect_custom_trace_attributes, operation_component, set_service_name};
use common::configuration::{
    AggregatorTarget, BranchErrorPolicy, CompositionStrategy, ExecutionMode,
};

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
    RequestParsing(String),
    #[error("HTTP error: {0}")]
    Http(#[from] hyper::Error),
    #[error("Unsupported endpoint: {0}")]
    UnsupportedEndpoint(String),
    #[error("No agents configured")]
    NoAgentsConfigured,
    #[error("Agent '{0}' not found in configuration")]
    AgentNotFound(String),
    #[error("No messages in conversation history")]
    EmptyHistory,
    #[error("Agent chain completed without producing a response")]
    IncompleteChain,
}

pub async fn agent_chat(
    request: Request<hyper::body::Incoming>,
    state: Arc<AppState>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let request_id = extract_request_id(&request);
    let custom_attrs =
        collect_custom_trace_attributes(request.headers(), state.span_attributes.as_ref());

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
    let listener = agent_selector.find_listener(listener_name, &state.listeners)?;

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
            warn!(path = %request_path, "unsupported endpoint");
            AgentFilterChainError::UnsupportedEndpoint(request_path.clone())
        })?;

    let client_request = ProviderRequestType::try_from((&chat_request_bytes[..], &api_type))
        .map_err(|err| {
            warn!(error = %err, "failed to parse request as ProviderRequestType");
            AgentFilterChainError::RequestParsing(format!("Failed to parse request: {}", err))
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
    let agents = state
        .agents_list
        .as_ref()
        .ok_or(AgentFilterChainError::NoAgentsConfigured)?;
    let agent_map = agent_selector.create_agent_map(agents);

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

/// Everything needed to merge agent replies for one turn. `None` at the call
/// site keeps the original sequential, last-agent-wins behavior.
pub struct AggregationPlan {
    pub strategy: CompositionStrategy,
    /// Present for `synthesize`. Unused for `concat`.
    pub target: Option<AggregatorTarget>,
    pub client: reqwest::Client,
    pub llm_provider_url: String,
    pub execution: ExecutionMode,
    pub branch_timeout: std::time::Duration,
    pub on_branch_error: BranchErrorPolicy,
}

/// Resolve the listener's `orchestration` block into an aggregation plan.
/// Concat needs no aggregator; synthesize does. Returns `None` only when the
/// block is absent or synthesize cannot resolve its aggregator.
async fn build_aggregation_plan(
    listener: &common::configuration::Listener,
    state: &AppState,
) -> Option<AggregationPlan> {
    let orchestration = listener.orchestration.as_ref()?;

    let target = if orchestration.strategy == CompositionStrategy::Concat {
        None
    } else {
        let agent_ids: Vec<String> = state
            .agents_list
            .as_ref()
            .map(|agents| agents.iter().map(|a| a.id.clone()).collect())
            .unwrap_or_default();
        let providers = state.llm_providers.read().await;
        match orchestration
            .resolve_aggregator(providers.iter().map(|(_, p)| p.as_ref()), &agent_ids)
        {
            Some(t) => Some(t),
            None => {
                warn!(
                    aggregator = ?orchestration.aggregator,
                    "synthesize aggregator did not resolve; falling back to last-agent-wins"
                );
                return None;
            }
        }
    };

    Some(AggregationPlan {
        strategy: orchestration.strategy,
        target,
        client: state.http_client.clone(),
        llm_provider_url: state.llm_provider_url.clone(),
        execution: orchestration.execution,
        branch_timeout: orchestration.branch_timeout_or_default(),
        on_branch_error: orchestration.on_branch_error,
    })
}

/// Per-turn inputs that stay the same for every agent in the chain.
struct ChainContext<'a> {
    request_headers: &'a hyper::HeaderMap,
    custom_attrs: &'a std::collections::HashMap<String, String>,
    aggregation: Option<AggregationPlan>,
}

/// Run the selected agents concurrently, each seeing only the user's
/// conversation. Replies come back in route order regardless of which branch
/// finished first, so the merge input is deterministic.
///
/// All branches share one deadline rather than each getting its own: with a
/// fan-out the client waits for the slowest branch, so the budget that matters
/// is the wall clock for the whole turn. Branches still running when it expires
/// are aborted and reported as unavailable.
#[allow(clippy::too_many_arguments)]
async fn execute_parallel(
    pipeline_processor: &PipelineProcessor,
    selected_agents: &[common::configuration::AgentFilterChain],
    agent_map: &std::collections::HashMap<String, common::configuration::Agent>,
    client_request: &ProviderRequestType,
    messages: &[OpenAIMessage],
    request_headers: &hyper::HeaderMap,
    custom_attrs: &std::collections::HashMap<String, String>,
    plan: &AggregationPlan,
) -> Result<Vec<AgentReply>, AgentFilterChainError> {
    let deadline = tokio::time::Instant::now() + plan.branch_timeout;
    let mut tasks = tokio::task::JoinSet::new();

    for selected_agent in selected_agents {
        let agent_name = selected_agent.id.clone();
        let agent = agent_map
            .get(&agent_name)
            .ok_or_else(|| AgentFilterChainError::AgentNotFound(agent_name.clone()))?
            .clone();

        let mut processor = pipeline_processor.branch();
        let selected = selected_agent.clone();
        let agent_map = agent_map.clone();
        let request = client_request.clone();
        let history = messages.to_vec();
        let headers = request_headers.clone();
        let attrs = custom_attrs.clone();

        // tokio::spawn does not inherit the tracing context, so each branch
        // carries its own span or the agent's nested spans misparent.
        let branch_span = info_span!(
            "agent",
            agent_id = %agent_name,
            message_count = history.len(),
        );

        tasks.spawn(
            async move {
                set_service_name(operation_component::AGENT);
                get_active_span(|span| {
                    span.update_name(format!("{} /v1/chat/completions", agent_name));
                    for (key, value) in &attrs {
                        span.set_attribute(opentelemetry::KeyValue::new(
                            key.clone(),
                            value.clone(),
                        ));
                    }
                });

                let outcome = tokio::time::timeout_at(
                    deadline,
                    run_branch(
                        &mut processor,
                        &selected,
                        &agent,
                        &agent_map,
                        request,
                        history,
                        &headers,
                    ),
                )
                .await;

                let result = match outcome {
                    Ok(Ok(text)) => Ok(text),
                    Ok(Err(err)) => Err(err),
                    Err(_) => Err("timed out".to_string()),
                };
                (agent_name, result)
            }
            .instrument(branch_span),
        );
    }

    let mut by_agent: std::collections::HashMap<String, Result<String, String>> =
        std::collections::HashMap::new();
    while let Some(joined) = tasks.join_next().await {
        match joined {
            Ok((agent_name, result)) => {
                if let Err(reason) = &result {
                    warn!(agent = %agent_name, reason = %reason, "branch produced no answer");
                }
                by_agent.insert(agent_name, result);
            }
            Err(err) => warn!(error = %err, "branch task failed to join"),
        }
    }

    // Reorder into the orchestrator's route order; completion order is arbitrary.
    Ok(selected_agents
        .iter()
        .filter_map(|selected| {
            by_agent.remove(&selected.id).map(|result| AgentReply {
                agent: selected.id.clone(),
                result,
            })
        })
        .collect())
}

/// Run the agent's input filters over the conversation, if it declares any.
async fn apply_input_filters(
    processor: &mut PipelineProcessor,
    selected_agent: &common::configuration::AgentFilterChain,
    agent_map: &std::collections::HashMap<String, common::configuration::Agent>,
    client_request: &ProviderRequestType,
    messages: Vec<OpenAIMessage>,
    request_headers: &hyper::HeaderMap,
) -> Result<Vec<OpenAIMessage>, AgentFilterChainError> {
    let has_filters = selected_agent
        .input_filters
        .as_ref()
        .map(|f| !f.is_empty())
        .unwrap_or(false);

    if !has_filters {
        return Ok(messages);
    }

    let filter_body = serde_json::json!({
        "model": client_request.model(),
        "messages": messages,
    });
    let filter_bytes = serde_json::to_vec(&filter_body).map_err(PipelineError::ParseError)?;

    let filtered_bytes = processor
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
    Ok(serde_json::from_value(filtered_body["messages"].clone())
        .map_err(PipelineError::ParseError)?)
}

/// One branch: optional input filters, then the agent call, collected to text.
async fn run_branch(
    processor: &mut PipelineProcessor,
    selected_agent: &common::configuration::AgentFilterChain,
    agent: &common::configuration::Agent,
    agent_map: &std::collections::HashMap<String, common::configuration::Agent>,
    client_request: ProviderRequestType,
    messages: Vec<OpenAIMessage>,
    request_headers: &hyper::HeaderMap,
) -> Result<String, String> {
    let chat_history = apply_input_filters(
        processor,
        selected_agent,
        agent_map,
        &client_request,
        messages,
        request_headers,
    )
    .await
    .map_err(|err| err.to_string())?;

    let response = processor
        .invoke_agent(&chat_history, client_request, agent, request_headers)
        .await
        .map_err(|err| err.to_string())?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("upstream returned {status}"));
    }

    ResponseHandler::new()
        .collect_full_response(response)
        .await
        .map_err(|err| err.to_string())
}

/// Execute the agent chain: run each selected agent sequentially. Without an
/// aggregation plan the final agent's response is streamed straight back, which
/// is the original behavior. With one, every reply is collected and merged.
async fn execute_agent_chain(
    mut pipeline_processor: PipelineProcessor,
    selected_agents: &[common::configuration::AgentFilterChain],
    agent_map: &std::collections::HashMap<String, common::configuration::Agent>,
    client_request: ProviderRequestType,
    messages: Vec<OpenAIMessage>,
    ctx: ChainContext<'_>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, AgentFilterChainError> {
    let ChainContext {
        request_headers,
        custom_attrs,
        aggregation,
    } = ctx;
    let response_handler = ResponseHandler::new();
    let user_request = last_user_text(&messages);
    let mut current_messages = messages;
    let agent_count = selected_agents.len();

    // A single agent has nothing to merge, so it keeps the cheaper streaming path.
    let aggregation = aggregation.filter(|_| agent_count > 1);
    let mut replies: Vec<AgentReply> = Vec::new();

    if aggregation.as_ref().map(|p| p.execution) == Some(ExecutionMode::Parallel) {
        let plan = aggregation.expect("execution mode was just read from the plan");
        let replies = execute_parallel(
            &pipeline_processor,
            selected_agents,
            agent_map,
            &client_request,
            &current_messages,
            request_headers,
            custom_attrs,
            &plan,
        )
        .await?;

        if plan.on_branch_error == BranchErrorPolicy::Fail {
            if let Some(failed) = replies.iter().find(|r| r.text().is_none()) {
                let reason = failed.result.as_ref().err().cloned().unwrap_or_default();
                return Err(PipelineError::NoContentInResponse(format!(
                    "agent '{}' produced no answer: {reason}",
                    failed.agent
                ))
                .into());
            }
        }

        if !replies.iter().any(|r| r.text().is_some()) {
            warn!("no agent produced a response to merge");
            return Err(AgentFilterChainError::IncompleteChain);
        }

        return merge_replies(
            plan,
            &mut pipeline_processor,
            agent_map,
            &client_request,
            request_headers,
            &user_request,
            replies,
        )
        .await;
    }

    for (agent_index, selected_agent) in selected_agents.iter().enumerate() {
        let agent_name = selected_agent.id.clone();
        let is_last_agent = agent_index == agent_count - 1;

        debug!(
            agent_index = agent_index + 1,
            total = agent_count,
            agent = %agent_name,
            "processing agent"
        );

        let chat_history = apply_input_filters(
            &mut pipeline_processor,
            selected_agent,
            agent_map,
            &client_request,
            current_messages.clone(),
            request_headers,
        )
        .await?;

        let agent = agent_map
            .get(&agent_name)
            .ok_or_else(|| AgentFilterChainError::AgentNotFound(agent_name.clone()))?;

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
        .await;

        // When merging, one failing agent should not sink the whole turn unless
        // the listener asked for that.
        let llm_response = match llm_response {
            Ok(response) => response,
            Err(err) => match aggregation.as_ref().map(|a| a.on_branch_error) {
                Some(BranchErrorPolicy::Continue) => {
                    warn!(agent = %agent_name, error = %err, "agent failed, continuing without it");
                    replies.push(AgentReply::failed(&agent_name, err.to_string()));
                    continue;
                }
                _ => return Err(err.into()),
            },
        };

        // Upstream status is only inspected when merging. The streaming path
        // forwards the response to the client untouched, as it always has.
        if let Some(plan) = aggregation.as_ref() {
            let status = llm_response.status();
            if !status.is_success() {
                let body = llm_response.text().await.unwrap_or_default();
                warn!(agent = %agent_name, status = %status, "agent returned an error status");

                if plan.on_branch_error == BranchErrorPolicy::Fail {
                    return Err(PipelineError::ClientError {
                        agent: agent_name.clone(),
                        status: status.as_u16(),
                        body,
                    }
                    .into());
                }
                replies.push(AgentReply::failed(
                    &agent_name,
                    format!("upstream returned {status}"),
                ));
                continue;
            }
        }

        if is_last_agent && aggregation.is_none() {
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

        if aggregation.is_some() && !response_text.trim().is_empty() {
            replies.push(AgentReply::answered(&agent_name, response_text.clone()));
        }

        if is_last_agent {
            break;
        }

        let Some(last_message) = current_messages.pop() else {
            warn!(agent = %agent_name, "no messages in conversation history");
            return Err(AgentFilterChainError::EmptyHistory);
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

    let Some(plan) = aggregation else {
        return Err(AgentFilterChainError::IncompleteChain);
    };

    if !replies.iter().any(|r| r.text().is_some()) {
        warn!("no agent produced a response to merge");
        return Err(AgentFilterChainError::IncompleteChain);
    }

    merge_replies(
        plan,
        &mut pipeline_processor,
        agent_map,
        &client_request,
        request_headers,
        &user_request,
        replies,
    )
    .await
}

/// Merge collected agent replies into one answer and shape it like the client
/// asked for. If merging fails we fall back to the last reply rather than
/// failing the turn outright.
async fn merge_replies(
    plan: AggregationPlan,
    pipeline_processor: &mut PipelineProcessor,
    agent_map: &std::collections::HashMap<String, common::configuration::Agent>,
    client_request: &ProviderRequestType,
    request_headers: &hyper::HeaderMap,
    user_request: &str,
    replies: Vec<AgentReply>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, AgentFilterChainError> {
    let aggregate_span = info_span!("aggregate", agent_count = replies.len());

    let merged = async {
        match plan.strategy {
            CompositionStrategy::Concat => Ok(concat_replies(&replies)),
            CompositionStrategy::Synthesize => match &plan.target {
                Some(AggregatorTarget::Model { provider, model }) => {
                    let url = format!(
                        "{}{}",
                        plan.llm_provider_url,
                        common::consts::CHAT_COMPLETIONS_PATH
                    );
                    aggregate_with_model(
                        &plan.client,
                        &url,
                        provider,
                        model,
                        user_request,
                        &replies,
                    )
                    .await
                    .map_err(|err| err.to_string())
                }
                Some(AggregatorTarget::Agent(id)) => {
                    let agent = agent_map
                        .get(id)
                        .ok_or_else(|| format!("aggregator agent '{id}' not found"))?;
                    let chat_history = build_aggregation_messages(user_request, &replies);
                    let response = pipeline_processor
                        .invoke_agent(
                            &chat_history,
                            client_request.clone(),
                            agent,
                            request_headers,
                        )
                        .await
                        .map_err(|err| err.to_string())?;
                    ResponseHandler::new()
                        .collect_full_response(response)
                        .await
                        .map_err(|err| err.to_string())
                }
                None => Err("synthesize plan is missing an aggregator target".to_string()),
            },
        }
    }
    .instrument(aggregate_span)
    .await;

    let content = match merged {
        Ok(text) => text,
        Err(err) => {
            warn!(error = %err, "aggregation failed, returning last agent response");
            replies
                .iter()
                .rev()
                .find_map(|r| r.text().map(str::to_string))
                .ok_or(AgentFilterChainError::IncompleteChain)?
        }
    };

    info!(
        agent_count = replies.len(),
        response_len = content.len(),
        "merged agent responses"
    );

    let model = client_request.model();
    Ok(if client_request.is_streaming() {
        ResponseHandler::create_synthetic_stream_response(&content, model)
    } else {
        ResponseHandler::create_completion_response(&content, model)
    })
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

    let aggregation = build_aggregation_plan(&listener, &state).await;

    execute_agent_chain(
        PipelineProcessor::default(),
        &selected_agents,
        &agent_map,
        agent_req.client_request,
        agent_req.messages,
        ChainContext {
            request_headers: &agent_req.request_headers,
            custom_attrs: &custom_attrs,
            aggregation,
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::configuration::{Agent, AgentFilterChain};
    use mockito::{Matcher, Server};
    use std::collections::HashMap;

    /// Build the client request the way production does: parse raw client bytes
    /// through the real endpoint resolver.
    fn client_request() -> ProviderRequestType {
        let body = serde_json::json!({
            "model": "gpt-4o",
            "stream": false,
            "messages": [{"role": "user", "content": "original question"}],
        });
        let bytes = serde_json::to_vec(&body).unwrap();
        let api_type = SupportedAPIsFromClient::from_endpoint("/v1/chat/completions").unwrap();
        ProviderRequestType::try_from((&bytes[..], &api_type)).unwrap()
    }

    fn user_message(text: &str) -> OpenAIMessage {
        OpenAIMessage {
            role: hermesllm::apis::openai::Role::User,
            content: Some(hermesllm::apis::openai::MessageContent::Text(
                text.to_string(),
            )),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// Minimal non-streaming OpenAI chat-completion envelope.
    fn completion_body(content: &str) -> String {
        serde_json::json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "created": 0,
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": content},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
        })
        .to_string()
    }

    fn chain_entry(id: &str) -> AgentFilterChain {
        AgentFilterChain {
            id: id.to_string(),
            description: None,
            default: None,
            input_filters: None,
        }
    }

    /// `invoke_agent` dials the api router and selects via `x-arch-upstream`.
    fn agent_map(ids: &[&str]) -> HashMap<String, Agent> {
        ids.iter()
            .map(|id| {
                (
                    id.to_string(),
                    Agent {
                        id: id.to_string(),
                        agent_type: None,
                        url: "http://unused.invalid".to_string(),
                        tool: None,
                        transport: None,
                    },
                )
            })
            .collect()
    }

    async fn run_chain(
        url: String,
        chain: &[AgentFilterChain],
        ids: &[&str],
    ) -> Result<Response<BoxBody<Bytes, hyper::Error>>, AgentFilterChainError> {
        run_chain_with(url, chain, ids, None).await
    }

    async fn run_chain_with(
        url: String,
        chain: &[AgentFilterChain],
        ids: &[&str],
        aggregation: Option<AggregationPlan>,
    ) -> Result<Response<BoxBody<Bytes, hyper::Error>>, AgentFilterChainError> {
        execute_agent_chain(
            PipelineProcessor::new(url),
            chain,
            &agent_map(ids),
            client_request(),
            vec![user_message("original question")],
            ChainContext {
                request_headers: &hyper::HeaderMap::new(),
                custom_attrs: &HashMap::new(),
                aggregation,
            },
        )
        .await
    }

    /// Aggregate through a model provider reachable at `url`.
    fn model_plan(url: String, on_branch_error: BranchErrorPolicy) -> AggregationPlan {
        AggregationPlan {
            strategy: CompositionStrategy::Synthesize,
            target: Some(AggregatorTarget::Model {
                provider: "openai".to_string(),
                model: "gpt-4o".to_string(),
            }),
            client: reqwest::Client::new(),
            llm_provider_url: url,
            execution: ExecutionMode::Sequential,
            branch_timeout: std::time::Duration::from_secs(60),
            on_branch_error,
        }
    }

    fn concat_plan(on_branch_error: BranchErrorPolicy) -> AggregationPlan {
        AggregationPlan {
            strategy: CompositionStrategy::Concat,
            target: None,
            client: reqwest::Client::new(),
            llm_provider_url: "http://unused.invalid".to_string(),
            execution: ExecutionMode::Parallel,
            branch_timeout: std::time::Duration::from_secs(60),
            on_branch_error,
        }
    }

    fn parallel_plan(url: String, on_branch_error: BranchErrorPolicy) -> AggregationPlan {
        AggregationPlan {
            execution: ExecutionMode::Parallel,
            ..model_plan(url, on_branch_error)
        }
    }

    async fn body_to_string(response: Response<BoxBody<Bytes, hyper::Error>>) -> String {
        let collected = response.into_body().collect().await.unwrap();
        String::from_utf8(collected.to_bytes().to_vec()).unwrap()
    }

    /// Intermediate reply is injected as a named assistant turn; user ask stays last.
    #[tokio::test]
    async fn intermediate_reply_is_injected_as_named_assistant_before_user_turn() {
        let mut server = Server::new_async().await;

        let first = server
            .mock("POST", "/v1/chat/completions")
            .match_header("x-arch-upstream", "agent_one")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(completion_body("answer from one"))
            .create_async()
            .await;

        let second = server
            .mock("POST", "/v1/chat/completions")
            .match_header("x-arch-upstream", "agent_two")
            .match_body(Matcher::PartialJson(serde_json::json!({
                "messages": [
                    {"role": "assistant", "content": "answer from one", "name": "agent_one"},
                    {"role": "user", "content": "original question"}
                ]
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(completion_body("answer from two"))
            .create_async()
            .await;

        let chain = vec![chain_entry("agent_one"), chain_entry("agent_two")];
        let result = run_chain(server.url(), &chain, &["agent_one", "agent_two"]).await;

        assert!(result.is_ok(), "chain failed: {:?}", result.err());
        first.assert_async().await;
        second.assert_async().await;
    }

    /// Route order from the orchestrator is the order agents run.
    #[tokio::test]
    async fn chain_executes_agents_in_the_order_supplied() {
        let mut server = Server::new_async().await;

        let first = server
            .mock("POST", "/v1/chat/completions")
            .match_header("x-arch-upstream", "agent_one")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(completion_body("answer from one"))
            .create_async()
            .await;

        let second = server
            .mock("POST", "/v1/chat/completions")
            .match_header("x-arch-upstream", "agent_two")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(completion_body("answer from two"))
            .create_async()
            .await;

        let third = server
            .mock("POST", "/v1/chat/completions")
            .match_header("x-arch-upstream", "agent_three")
            .match_body(Matcher::PartialJson(serde_json::json!({
                "messages": [
                    {"role": "assistant", "content": "answer from one", "name": "agent_one"},
                    {"role": "assistant", "content": "answer from two", "name": "agent_two"},
                    {"role": "user", "content": "original question"}
                ]
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(completion_body("answer from three"))
            .create_async()
            .await;

        let chain = vec![
            chain_entry("agent_one"),
            chain_entry("agent_two"),
            chain_entry("agent_three"),
        ];
        let result = run_chain(
            server.url(),
            &chain,
            &["agent_one", "agent_two", "agent_three"],
        )
        .await;

        assert!(result.is_ok(), "chain failed: {:?}", result.err());
        first.assert_async().await;
        second.assert_async().await;
        third.assert_async().await;
    }

    /// Only the final agent's response reaches the client.
    #[tokio::test]
    async fn only_the_final_agent_response_is_returned_to_the_client() {
        let mut server = Server::new_async().await;

        let _first = server
            .mock("POST", "/v1/chat/completions")
            .match_header("x-arch-upstream", "agent_one")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(completion_body("answer from one"))
            .create_async()
            .await;

        let _second = server
            .mock("POST", "/v1/chat/completions")
            .match_header("x-arch-upstream", "agent_two")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(completion_body("answer from two"))
            .create_async()
            .await;

        let chain = vec![chain_entry("agent_one"), chain_entry("agent_two")];
        let response = run_chain(server.url(), &chain, &["agent_one", "agent_two"])
            .await
            .expect("chain should succeed");

        let body = body_to_string(response).await;
        assert!(
            body.contains("answer from two"),
            "final agent reply missing from client response: {body}"
        );
        assert!(
            !body.contains("answer from one"),
            "intermediate reply leaked into the client response: {body}"
        );
    }

    /// Single-agent chain streams straight through.
    #[tokio::test]
    async fn single_agent_chain_returns_that_agents_response() {
        let mut server = Server::new_async().await;

        let only = server
            .mock("POST", "/v1/chat/completions")
            .match_header("x-arch-upstream", "solo_agent")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(completion_body("solo answer"))
            .create_async()
            .await;

        let chain = vec![chain_entry("solo_agent")];
        let response = run_chain(server.url(), &chain, &["solo_agent"])
            .await
            .expect("chain should succeed");

        only.assert_async().await;
        assert!(body_to_string(response).await.contains("solo answer"));
    }

    /// Empty route list must not silently return an empty 200.
    #[tokio::test]
    async fn empty_chain_reports_incomplete_rather_than_succeeding() {
        let result = run_chain("http://unused.invalid".to_string(), &[], &[]).await;

        assert!(matches!(
            result,
            Err(AgentFilterChainError::IncompleteChain)
        ));
    }

    /// Unknown agent in the chain is a hard error.
    #[tokio::test]
    async fn unknown_agent_in_chain_is_a_hard_error() {
        let chain = vec![chain_entry("ghost_agent")];
        let result = run_chain("http://unused.invalid".to_string(), &chain, &[]).await;

        match result {
            Err(AgentFilterChainError::AgentNotFound(name)) => assert_eq!(name, "ghost_agent"),
            other => panic!("expected AgentNotFound, got {other:?}"),
        }
    }

    fn agent_mock(server: &mut mockito::ServerGuard, id: &str, answer: &str) -> mockito::Mock {
        server
            .mock("POST", "/v1/chat/completions")
            .match_header("x-arch-upstream", id)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(completion_body(answer))
            .create()
    }

    /// The aggregator is reached by provider hint rather than upstream agent id.
    fn aggregator_mock(server: &mut mockito::ServerGuard, merged: &str) -> mockito::Mock {
        server
            .mock("POST", "/v1/chat/completions")
            .match_header("x-arch-llm-provider-hint", "openai")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(completion_body(merged))
            .create()
    }

    /// The whole point of the feature: every agent's answer reaches the merge,
    /// and the client sees the merged text instead of only the last reply.
    #[tokio::test]
    async fn aggregation_merges_every_agent_reply() {
        let mut server = Server::new_async().await;
        let _one = agent_mock(&mut server, "agent_one", "answer from one");
        let _two = agent_mock(&mut server, "agent_two", "answer from two");

        let merge = server
            .mock("POST", "/v1/chat/completions")
            .match_header("x-arch-llm-provider-hint", "openai")
            .match_body(Matcher::Regex(
                r#"answer from one[\s\S]*answer from two"#.to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(completion_body("one and two"))
            .create_async()
            .await;

        let chain = vec![chain_entry("agent_one"), chain_entry("agent_two")];
        let response = run_chain_with(
            server.url(),
            &chain,
            &["agent_one", "agent_two"],
            Some(model_plan(server.url(), BranchErrorPolicy::Continue)),
        )
        .await
        .expect("chain should succeed");

        merge.assert_async().await;
        assert!(body_to_string(response).await.contains("one and two"));
    }

    /// One agent has nothing to merge, so aggregation is skipped entirely.
    #[tokio::test]
    async fn single_agent_skips_the_aggregator() {
        let mut server = Server::new_async().await;
        let _only = agent_mock(&mut server, "solo_agent", "solo answer");
        let merge = aggregator_mock(&mut server, "should not be called");

        let chain = vec![chain_entry("solo_agent")];
        let response = run_chain_with(
            server.url(),
            &chain,
            &["solo_agent"],
            Some(model_plan(server.url(), BranchErrorPolicy::Continue)),
        )
        .await
        .expect("chain should succeed");

        merge.expect(0).assert_async().await;
        assert!(body_to_string(response).await.contains("solo answer"));
    }

    /// A failing agent under the default policy is dropped, not fatal.
    #[tokio::test]
    async fn failing_agent_is_skipped_when_policy_is_continue() {
        let mut server = Server::new_async().await;
        let _ok = agent_mock(&mut server, "agent_one", "answer from one");
        let _bad = server
            .mock("POST", "/v1/chat/completions")
            .match_header("x-arch-upstream", "agent_two")
            .with_status(500)
            .with_body("upstream exploded")
            .create();
        let merge = aggregator_mock(&mut server, "merged despite failure");

        let chain = vec![chain_entry("agent_one"), chain_entry("agent_two")];
        let response = run_chain_with(
            server.url(),
            &chain,
            &["agent_one", "agent_two"],
            Some(model_plan(server.url(), BranchErrorPolicy::Continue)),
        )
        .await
        .expect("chain should succeed");

        merge.assert_async().await;
        assert!(body_to_string(response)
            .await
            .contains("merged despite failure"));
    }

    /// `on_branch_error: fail` keeps the strict behavior.
    #[tokio::test]
    async fn failing_agent_fails_the_turn_when_policy_is_fail() {
        let mut server = Server::new_async().await;
        let _ok = agent_mock(&mut server, "agent_one", "answer from one");
        let _bad = server
            .mock("POST", "/v1/chat/completions")
            .match_header("x-arch-upstream", "agent_two")
            .with_status(500)
            .with_body("upstream exploded")
            .create();

        let chain = vec![chain_entry("agent_one"), chain_entry("agent_two")];
        let result = run_chain_with(
            server.url(),
            &chain,
            &["agent_one", "agent_two"],
            Some(model_plan(server.url(), BranchErrorPolicy::Fail)),
        )
        .await;

        assert!(
            result.is_err(),
            "expected the failing branch to fail the turn"
        );
    }

    /// If the merge call dies we still answer, using the last reply we have.
    #[tokio::test]
    async fn aggregator_failure_falls_back_to_the_last_reply() {
        let mut server = Server::new_async().await;
        let _one = agent_mock(&mut server, "agent_one", "answer from one");
        let _two = agent_mock(&mut server, "agent_two", "answer from two");
        let _merge = server
            .mock("POST", "/v1/chat/completions")
            .match_header("x-arch-llm-provider-hint", "openai")
            .with_status(500)
            .with_body("aggregator down")
            .create();

        let chain = vec![chain_entry("agent_one"), chain_entry("agent_two")];
        let response = run_chain_with(
            server.url(),
            &chain,
            &["agent_one", "agent_two"],
            Some(model_plan(server.url(), BranchErrorPolicy::Continue)),
        )
        .await
        .expect("fallback should still produce a response");

        assert!(body_to_string(response).await.contains("answer from two"));
    }

    /// Clients that asked for a stream get SSE, not a bare completion object.
    #[tokio::test]
    async fn streaming_clients_get_the_merge_as_sse() {
        let mut server = Server::new_async().await;
        let _one = agent_mock(&mut server, "agent_one", "answer from one");
        let _two = agent_mock(&mut server, "agent_two", "answer from two");
        let _merge = aggregator_mock(&mut server, "merged answer");

        let body = serde_json::json!({
            "model": "gpt-4o",
            "stream": true,
            "messages": [{"role": "user", "content": "original question"}],
        });
        let bytes = serde_json::to_vec(&body).unwrap();
        let api_type = SupportedAPIsFromClient::from_endpoint("/v1/chat/completions").unwrap();
        let streaming_request = ProviderRequestType::try_from((&bytes[..], &api_type)).unwrap();

        let chain = vec![chain_entry("agent_one"), chain_entry("agent_two")];
        let response = execute_agent_chain(
            PipelineProcessor::new(server.url()),
            &chain,
            &agent_map(&["agent_one", "agent_two"]),
            streaming_request,
            vec![user_message("original question")],
            ChainContext {
                request_headers: &hyper::HeaderMap::new(),
                custom_attrs: &HashMap::new(),
                aggregation: Some(model_plan(server.url(), BranchErrorPolicy::Continue)),
            },
        )
        .await
        .expect("chain should succeed");

        assert_eq!(
            response
                .headers()
                .get(hyper::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("text/event-stream")
        );

        let body = body_to_string(response).await;
        assert!(body.contains("merged answer"));
        assert!(body.contains("data: [DONE]"));
    }

    /// Concat joins fixed payloads with no model call — the cheap path for
    /// complementary specialists.
    #[tokio::test]
    async fn parallel_concat_joins_without_an_aggregator_call() {
        let mut server = Server::new_async().await;
        let _one = agent_mock(&mut server, "agent_one", "TOKEN_ONE");
        let _two = agent_mock(&mut server, "agent_two", "TOKEN_TWO");
        let merge = aggregator_mock(&mut server, "should not be called");

        let chain = vec![chain_entry("agent_one"), chain_entry("agent_two")];
        let response = run_chain_with(
            server.url(),
            &chain,
            &["agent_one", "agent_two"],
            Some(concat_plan(BranchErrorPolicy::Continue)),
        )
        .await
        .expect("chain should succeed");

        merge.expect(0).assert_async().await;
        let body = body_to_string(response).await;
        assert!(body.contains("TOKEN_ONE"));
        assert!(body.contains("TOKEN_TWO"));
        assert!(body.contains("[agent_one]"));
        assert!(body.contains("[agent_two]"));
    }

    /// Parallel branches still produce a merged answer covering both agents.
    #[tokio::test]
    async fn parallel_merges_every_branch() {
        let mut server = Server::new_async().await;
        let _one = agent_mock(&mut server, "agent_one", "answer from one");
        let _two = agent_mock(&mut server, "agent_two", "answer from two");

        let merge = server
            .mock("POST", "/v1/chat/completions")
            .match_header("x-arch-llm-provider-hint", "openai")
            .match_body(Matcher::Regex(
                r#"answer from one[\s\S]*answer from two"#.to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(completion_body("one and two"))
            .create_async()
            .await;

        let chain = vec![chain_entry("agent_one"), chain_entry("agent_two")];
        let response = run_chain_with(
            server.url(),
            &chain,
            &["agent_one", "agent_two"],
            Some(parallel_plan(server.url(), BranchErrorPolicy::Continue)),
        )
        .await
        .expect("chain should succeed");

        merge.assert_async().await;
        assert!(body_to_string(response).await.contains("one and two"));
    }

    /// The point of fanning out: no branch sees a sibling's answer. The first
    /// mock reproduces the sequential chained body and must never be hit.
    #[tokio::test]
    async fn parallel_branches_do_not_see_sibling_answers() {
        let mut server = Server::new_async().await;
        let _one = agent_mock(&mut server, "agent_one", "answer from one");

        let chained = server
            .mock("POST", "/v1/chat/completions")
            .match_header("x-arch-upstream", "agent_two")
            .match_body(Matcher::PartialJson(serde_json::json!({
                "messages": [
                    {"role": "assistant", "content": "answer from one", "name": "agent_one"},
                    {"role": "user", "content": "original question"}
                ]
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(completion_body("leaked"))
            .create();

        let _two = agent_mock(&mut server, "agent_two", "answer from two");
        let _merge = aggregator_mock(&mut server, "merged");

        let chain = vec![chain_entry("agent_one"), chain_entry("agent_two")];
        run_chain_with(
            server.url(),
            &chain,
            &["agent_one", "agent_two"],
            Some(parallel_plan(server.url(), BranchErrorPolicy::Continue)),
        )
        .await
        .expect("chain should succeed");

        chained.expect(0).assert_async().await;
    }

    /// Completion order is arbitrary, so replies are reordered into route order
    /// to keep the merge input deterministic.
    #[tokio::test]
    async fn parallel_replies_are_reordered_into_route_order() {
        let mut server = Server::new_async().await;
        let _one = agent_mock(&mut server, "agent_one", "alpha");
        let _two = agent_mock(&mut server, "agent_two", "beta");

        // Route order is [agent_two, agent_one], so beta must precede alpha.
        let merge = server
            .mock("POST", "/v1/chat/completions")
            .match_header("x-arch-llm-provider-hint", "openai")
            .match_body(Matcher::Regex(r#"beta[\s\S]*alpha"#.to_string()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(completion_body("ordered"))
            .create_async()
            .await;

        let chain = vec![chain_entry("agent_two"), chain_entry("agent_one")];
        run_chain_with(
            server.url(),
            &chain,
            &["agent_one", "agent_two"],
            Some(parallel_plan(server.url(), BranchErrorPolicy::Continue)),
        )
        .await
        .expect("chain should succeed");

        merge.assert_async().await;
    }

    /// A failed branch reaches the merge labeled unavailable, so the reply can
    /// say what is missing rather than looking complete.
    #[tokio::test]
    async fn parallel_failed_branch_is_labeled_unavailable_for_the_merge() {
        let mut server = Server::new_async().await;
        let _one = agent_mock(&mut server, "agent_one", "answer from one");
        let _bad = server
            .mock("POST", "/v1/chat/completions")
            .match_header("x-arch-upstream", "agent_two")
            .with_status(503)
            .with_body("down")
            .create();

        let merge = server
            .mock("POST", "/v1/chat/completions")
            .match_header("x-arch-llm-provider-hint", "openai")
            .match_body(Matcher::Regex(
                r#"agent_two\\" status=\\"unavailable"#.to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(completion_body("partial answer"))
            .create_async()
            .await;

        let chain = vec![chain_entry("agent_one"), chain_entry("agent_two")];
        let response = run_chain_with(
            server.url(),
            &chain,
            &["agent_one", "agent_two"],
            Some(parallel_plan(server.url(), BranchErrorPolicy::Continue)),
        )
        .await
        .expect("chain should succeed");

        merge.assert_async().await;
        assert!(body_to_string(response).await.contains("partial answer"));
    }

    /// Under `fail`, a dead branch fails the turn instead of degrading.
    #[tokio::test]
    async fn parallel_failed_branch_fails_the_turn_under_fail_policy() {
        let mut server = Server::new_async().await;
        let _one = agent_mock(&mut server, "agent_one", "answer from one");
        let _bad = server
            .mock("POST", "/v1/chat/completions")
            .match_header("x-arch-upstream", "agent_two")
            .with_status(503)
            .with_body("down")
            .create();

        let chain = vec![chain_entry("agent_one"), chain_entry("agent_two")];
        let result = run_chain_with(
            server.url(),
            &chain,
            &["agent_one", "agent_two"],
            Some(parallel_plan(server.url(), BranchErrorPolicy::Fail)),
        )
        .await;

        assert!(result.is_err(), "expected the dead branch to fail the turn");
    }

    /// If every branch comes back empty there is nothing to merge, and the turn
    /// reports that rather than returning an empty 200.
    #[tokio::test]
    async fn parallel_reports_incomplete_when_no_branch_answers() {
        let mut server = Server::new_async().await;
        for id in ["agent_one", "agent_two"] {
            server
                .mock("POST", "/v1/chat/completions")
                .match_header("x-arch-upstream", id)
                .with_status(503)
                .with_body("down")
                .create();
        }

        let chain = vec![chain_entry("agent_one"), chain_entry("agent_two")];
        let result = run_chain_with(
            server.url(),
            &chain,
            &["agent_one", "agent_two"],
            Some(parallel_plan(server.url(), BranchErrorPolicy::Continue)),
        )
        .await;

        assert!(matches!(
            result,
            Err(AgentFilterChainError::IncompleteChain)
        ));
    }

    /// Without an aggregation plan nothing about the old path changes.
    #[tokio::test]
    async fn no_plan_keeps_last_agent_wins() {
        let mut server = Server::new_async().await;
        let _one = agent_mock(&mut server, "agent_one", "answer from one");
        let _two = agent_mock(&mut server, "agent_two", "answer from two");
        let merge = aggregator_mock(&mut server, "should not be called");

        let chain = vec![chain_entry("agent_one"), chain_entry("agent_two")];
        let response = run_chain(server.url(), &chain, &["agent_one", "agent_two"])
            .await
            .expect("chain should succeed");

        merge.expect(0).assert_async().await;
        let body = body_to_string(response).await;
        assert!(body.contains("answer from two"));
        assert!(!body.contains("answer from one"));
    }
}
